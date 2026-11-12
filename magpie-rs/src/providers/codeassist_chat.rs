use axum::extract::State;
use axum::http::StatusCode;
use axum::{routing::post, Json, Router};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;

use super::code_assist::{code_assist_envelope, generate_content_stream, unwrap_response_stream};
use super::gemini::openai_chunk;
use super::google_auth::{refresh, GoogleApp};

pub const GEMINI_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";

#[derive(Clone)]
pub struct CodeAssistState {
    pub app: GoogleApp,
    pub base_url: String,
    pub refresh_token: String,
    pub project: String,
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

fn to_gemini_request(req: &ChatRequest) -> Value {
    let mut contents = Vec::new();
    let mut system: Option<Value> = None;
    for m in &req.messages {
        let text = text_of(&m.content);
        let part = json!({"text": text});
        match m.role.as_str() {
            "system" | "developer" => system = Some(json!({"role": "user", "parts": [part]})),
            "assistant" => contents.push(json!({"role": "model", "parts": [part]})),
            _ => contents.push(json!({"role": "user", "parts": [part]})),
        }
    }
    let mut request = json!({"contents": contents});
    if let Some(sys) = system {
        request["systemInstruction"] = sys;
    }
    request
}

pub fn router(state: Arc<CodeAssistState>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(handle_chat))
        .with_state(state)
}

async fn handle_chat(
    State(state): State<Arc<CodeAssistState>>,
    Json(req): Json<ChatRequest>,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let access = refresh(&state.app, &state.refresh_token)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let request = to_gemini_request(&req);
    let body = json!({"model": req.model, "request": request});
    let (envelope, _model) =
        code_assist_envelope("gemini", &body, &state.project, |m| m.to_string())
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let raw = generate_content_stream(
        &state.http,
        &state.base_url,
        &access,
        "magpie-rs",
        &envelope,
    )
    .await
    .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let unwrapped = unwrap_response_stream(raw);

    let id = format!("chatcmpl-codeassist-{}", nanos());
    let model = req.model.clone();
    let stream = unwrapped
        .map(move |chunk| {
            let model = model.clone();
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
        .header("content-type", "text/event-stream")
        .body(axum::body::Body::from_stream(stream))
        .unwrap())
}

fn nanos() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .map(|n| format!("{:x}", n))
        .unwrap_or_else(|_| "0".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_gemini_request_converts() {
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
            ],
            stream: true,
        };
        let r = to_gemini_request(&req);
        assert_eq!(r["systemInstruction"]["parts"][0]["text"], "be nice");
        assert_eq!(r["contents"].as_array().unwrap().len(), 1);
    }

    use super::super::google_auth::GOOGLE_AUTHORIZE_URL;

    #[tokio::test]
    async fn end_to_end_openai_sse_through_codeassist() {
        // Mock Google token endpoint + Code Assist endpoint.
        let token_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let token_addr = token_listener.local_addr().unwrap();
        let token_app = axum::Router::new().route(
            "/token",
            axum::routing::post(|| async {
                axum::Json(serde_json::json!({"access_token": "at-1"}))
            }),
        );
        tokio::spawn(async move {
            axum::serve(token_listener, token_app).await.unwrap();
        });

        let ca_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ca_addr = ca_listener.local_addr().unwrap();
        let ca_app = axum::Router::new().route(
            "/v1internal:generateContent",
            axum::routing::post(|| async {
                axum::response::Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(axum::body::Body::from(
                        "data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hello\"}]}}]}}\n\n"
                            .to_string(),
                    ))
                    .unwrap()
            }),
        );
        tokio::spawn(async move {
            axum::serve(ca_listener, ca_app).await.unwrap();
        });

        let state = Arc::new(CodeAssistState {
            app: GoogleApp {
                client_id: "test-client".into(),
                client_secret: "test-secret".into(),
                authorize_url: GOOGLE_AUTHORIZE_URL.into(),
                token_url: format!("http://{}/token", token_addr),
                scopes: vec!["scope".into()],
                callback: "/oauth2callback".into(),
                redirect_host: "localhost".into(),
            },
            base_url: format!("http://{}", ca_addr),
            refresh_token: "rt-1".into(),
            project: "proj".into(),
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
                "model": "gemini-2.5-flash",
                "messages": [{"role": "user", "content": "hi"}],
                "stream": true
            }))
            .send()
            .await
            .unwrap();
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        eprintln!("DEBUG status={} body={}", status, body);
        assert!(status.is_success(), "status: {}", status);
        assert!(body.contains("chat.completion.chunk"), "body: {}", body);
        assert!(body.contains("hello"), "body: {}", body);
        assert!(body.contains("[DONE]"), "body: {}", body);
    }
}
