use axum::extract::State;
use axum::http::StatusCode;
use axum::{routing::post, Json, Router};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const GLM_PROD: &str = "https://open.bigmodel.cn/api/paas/v4";

#[derive(Clone)]
pub struct GlmState {
    pub api_key: String,
    pub base_url: String,
    pub http: reqwest::Client,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    #[serde(default)]
    pub stream: bool,
}

pub fn router(state: Arc<GlmState>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(handle_chat))
        .with_state(state)
}

pub async fn handle_chat(
    State(state): State<Arc<GlmState>>,
    Json(req): Json<ChatRequest>,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let url = format!("{}/chat/completions", state.base_url);
    let resp = state
        .http
        .post(&url)
        .header("Authorization", format!("Bearer {}", state.api_key))
        .header("content-type", "application/json")
        .json(&req)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    if !resp.status().is_success() {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let stream = resp
        .bytes_stream()
        .map(|chunk| chunk.map_err(|_: reqwest::Error| std::io::Error::other("stream")));
    Ok(axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .body(axum::body::Body::from_stream(stream))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn end_to_end_glm_streaming_passthrough() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(|| async {
                axum::response::Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(axum::body::Body::from(
                        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"}}]}\n\n"
                            .to_string(),
                    ))
                    .unwrap()
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let state = Arc::new(GlmState {
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
                "model": "glm-4-flash",
                "messages": [{"role": "user", "content": "hi"}],
                "stream": true
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
        let body = resp.text().await.unwrap();
        assert!(body.contains("chat.completion.chunk"));
        assert!(body.contains("hello"));
    }
}
