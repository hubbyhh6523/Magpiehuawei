use axum::extract::State;
use axum::http::StatusCode;
use axum::{routing::post, Json, Router};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub const CLAUDE_PROD: &str = "https://api.anthropic.com";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Clone)]
pub struct ClaudeState {
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

pub fn to_claude_request(req: &ChatRequest) -> Value {
    let mut system = String::new();
    let mut messages = Vec::new();
    for m in &req.messages {
        let text = text_of(&m.content);
        match m.role.as_str() {
            "system" | "developer" => {
                if !system.is_empty() {
                    system.push('\n');
                }
                system.push_str(&text);
            }
            "assistant" => messages.push(json!({"role": "assistant", "content": text})),
            _ => messages.push(json!({"role": "user", "content": text})),
        }
    }
    let mut request = json!({
        "model": req.model,
        "max_tokens": 4096,
        "messages": messages,
    });
    if !system.is_empty() {
        request["system"] = json!(system);
    }
    request
}

pub fn openai_chunk(id: &str, model: &str, text: &str) -> String {
    let chunk = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {"content": text},
        }],
    });
    format!("data: {}\n\n", chunk)
}

pub fn parse_claude_sse(text: &str, id: &str, model: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if v.get("type").and_then(|t| t.as_str()) == Some("content_block_delta") {
                    if let Some(t) = v.pointer("/delta/text").and_then(|t| t.as_str()) {
                        out.push_str(&openai_chunk(id, model, t));
                    }
                }
            }
        }
    }
    out
}

pub fn router(state: Arc<ClaudeState>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(handle_chat))
        .with_state(state)
}

pub async fn handle_chat(
    State(state): State<Arc<ClaudeState>>,
    Json(req): Json<ChatRequest>,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let request = to_claude_request(&req);
    let url = format!("{}/v1/messages", state.base_url);
    let resp = state
        .http
        .post(&url)
        .header("x-api-key", &state.api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    if !resp.status().is_success() {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let model = req.model.clone();
    let stream = resp.bytes_stream().map(move |chunk| {
        let model = model.clone();
        chunk
            .map(move |b| {
                let text = String::from_utf8_lossy(&b);
                bytes::Bytes::from(parse_claude_sse(&text, "chatcmpl-claude", &model))
            })
            .map_err(|_: reqwest::Error| std::io::Error::other("stream"))
    });
    let id = "chatcmpl-claude";
    let model = req.model.clone();
    let stream = stream.chain(futures_util::stream::once(async move {
        Ok::<_, std::io::Error>(bytes::Bytes::from(format!(
            "data: {{\"id\":\"{}\",\"object\":\"chat.completion.chunk\",\"model\":\"{}\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n",
            id, model
        )))
    }));
    Ok(axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .body(axum::body::Body::from_stream(stream))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_openai_messages_to_claude() {
        let req = ChatRequest {
            model: "claude-sonnet-4-5".into(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: json!("be nice"),
                },
                Message {
                    role: "user".into(),
                    content: json!("hi"),
                },
                Message {
                    role: "assistant".into(),
                    content: json!("hello"),
                },
            ],
            stream: true,
        };
        let r = to_claude_request(&req);
        assert_eq!(r["system"], "be nice");
        assert_eq!(r["messages"].as_array().unwrap().len(), 2);
        assert_eq!(r["messages"][0]["role"], "user");
        assert_eq!(r["messages"][1]["role"], "assistant");
    }

    #[test]
    fn parses_claude_sse_to_openai_chunks() {
        let sse = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n";
        let out = parse_claude_sse(sse, "id1", "claude-sonnet-4-5");
        assert!(out.contains("chat.completion.chunk"));
        assert!(out.contains("hello"));
    }

    #[tokio::test]
    async fn end_to_end_claude_streaming() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/v1/messages",
            axum::routing::post(|| async {
                axum::response::Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(axum::body::Body::from(
                        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hi there\"}}\n\n"
                            .to_string(),
                    ))
                    .unwrap()
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let state = Arc::new(ClaudeState {
            api_key: "test-key".into(),
            base_url: format!("http://{}", addr),
            http: reqwest::Client::new(),
        });
        let app = router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{}/v1/chat/completions", port))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-5",
                "messages": [{"role": "user", "content": "hi"}],
                "stream": true
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
        let body = resp.text().await.unwrap();
        assert!(body.contains("chat.completion.chunk"));
        assert!(body.contains("hi there"));
        assert!(body.contains("[DONE]"));
    }
}
