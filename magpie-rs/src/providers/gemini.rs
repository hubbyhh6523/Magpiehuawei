use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::{routing::post, Json, Router};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::convert::Infallible;
use std::sync::Arc;

pub const GEMINI_PROD: &str = "https://generativelanguage.googleapis.com";

#[derive(Clone)]
pub struct GeminiState {
    pub api_key: String,
    pub base_url: String,
    pub http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Debug, Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
}

fn text_of(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    }
}

fn to_gemini(req: &ChatRequest) -> GeminiRequest {
    let mut contents = Vec::new();
    let mut system: Option<GeminiContent> = None;
    for m in &req.messages {
        let text = text_of(&m.content);
        match m.role.as_str() {
            "system" | "developer" => {
                system = Some(GeminiContent {
                    role: "user".into(),
                    parts: vec![GeminiPart { text }],
                })
            }
            "assistant" => contents.push(GeminiContent {
                role: "model".into(),
                parts: vec![GeminiPart { text }],
            }),
            _ => contents.push(GeminiContent {
                role: "user".into(),
                parts: vec![GeminiPart { text }],
            }),
        }
    }
    GeminiRequest {
        contents,
        system_instruction: system,
    }
}

pub fn openai_chunk(id: &str, model: &str, delta_text: &str, finish: Option<&str>) -> String {
    let mut delta = serde_json::Map::new();
    if !delta_text.is_empty() {
        delta.insert("content".into(), Value::String(delta_text.to_string()));
    }
    let mut choice = serde_json::Map::new();
    choice.insert("index".into(), Value::from(0));
    choice.insert("delta".into(), Value::Object(delta));
    if let Some(f) = finish {
        choice.insert("finish_reason".into(), Value::String(f.to_string()));
    }
    let chunk = serde_json::json!({
        "id": id,
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [Value::Object(choice)],
    });
    format!("data: {}\n\n", chunk)
}

pub fn router(state: Arc<GeminiState>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(handle_chat))
        .with_state(state)
}

pub async fn handle_chat(
    State(state): State<Arc<GeminiState>>,
    Json(req): Json<ChatRequest>,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let model = req.model.clone();
    let body = to_gemini(&req);
    let url = format!(
        "{}/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
        state.base_url, model, state.api_key
    );
    let resp = state
        .http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?;
    if !resp.status().is_success() {
        return Err(axum::http::StatusCode::BAD_GATEWAY);
    }
    if !req.stream {
        let text = resp
            .text()
            .await
            .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?;
        let mut out = String::new();
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(v) = serde_json::from_str::<Value>(data) {
                    if let Some(t) = v
                        .pointer("/candidates/0/content/parts/0/text")
                        .and_then(|t| t.as_str())
                    {
                        out.push_str(t);
                    }
                }
            }
        }
        let resp_json = serde_json::json!({
            "id": "chatcmpl-gemini",
            "object": "chat.completion",
            "model": model,
            "choices": [{"index": 0, "message": {"role": "assistant", "content": out}, "finish_reason": "stop"}],
        });
        return Ok(axum::Json(resp_json).into_response());
    }

    let id = format!("chatcmpl-gemini-{}", uuid_like());
    let model_c = model.clone();
    let stream = resp
        .bytes_stream()
        .map(move |chunk| {
            let model = model_c.clone();
            let id = id.clone();
            match chunk {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut out = String::new();
                    for line in text.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if let Ok(v) = serde_json::from_str::<Value>(data) {
                                if let Some(t) = v
                                    .pointer("/candidates/0/content/parts/0/text")
                                    .and_then(|t| t.as_str())
                                {
                                    out.push_str(&openai_chunk(&id, &model, t, None));
                                }
                            }
                        }
                    }
                    Ok::<_, Infallible>(bytes::Bytes::from(out))
                }
                Err(_) => Ok::<_, Infallible>(bytes::Bytes::new()),
            }
        })
        .chain(futures_util::stream::once(async {
            Ok::<_, Infallible>(bytes::Bytes::from("data: [DONE]\n\n"))
        }));

    Ok(axum::response::Response::builder()
        .header(CONTENT_TYPE, "text/event-stream")
        .body(axum::body::Body::from_stream(stream))
        .unwrap())
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{:x}", n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_openai_messages_to_gemini() {
        let req = ChatRequest {
            model: "gemini-2.5-flash".into(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: serde_json::json!("be nice"),
                },
                Message {
                    role: "user".into(),
                    content: serde_json::json!("hi"),
                },
                Message {
                    role: "assistant".into(),
                    content: serde_json::json!("hello"),
                },
            ],
            stream: true,
        };
        let r = serde_json::to_value(to_gemini(&req)).unwrap();
        assert_eq!(r["system_instruction"]["parts"][0]["text"], "be nice");
        assert_eq!(r["contents"].as_array().unwrap().len(), 2);
        assert_eq!(r["contents"][0]["role"], "user");
        assert_eq!(r["contents"][1]["role"], "model");
    }

    #[test]
    fn openai_chunk_format() {
        let chunk = openai_chunk("id1", "gemini-2.5-flash", "hello", None);
        assert!(chunk.contains("chat.completion.chunk"));
        assert!(chunk.contains("hello"));
    }
}
