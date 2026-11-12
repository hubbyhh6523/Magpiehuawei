use axum::{routing::post, Json, Router};
use magpie_rs::gateway_app::{build_router, Config};
use magpie_rs::providers::claude;
use magpie_rs::providers::deepseek;
use magpie_rs::providers::gemini;
use magpie_rs::providers::glm;
use magpie_rs::providers::llama;
use magpie_rs::providers::moonshot;
use magpie_rs::providers::openai;
use magpie_rs::providers::qwen;

async fn spawn_mock_upstream() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream = Router::new()
        .route(
            "/v1/sessions",
            post(|| async { Json(serde_json::json!({"data": {"id": "sess-1"}})) }),
        )
        .route(
            "/api/v2/chat/completions",
            post(|| async {
                let body = "data: {\"id\":\"chatcmpl-1\"}\n\ndata: [DONE]\n\n";
                axum::response::Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(axum::body::Body::from(body))
                    .unwrap()
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    format!("http://{}", addr)
}

#[tokio::test]
async fn sse_end_to_end_through_magpie() {
    let upstream = spawn_mock_upstream().await;
    let cfg = Config {
        listen: "127.0.0.1:0".into(),
        codearts_ak: "test-ak".into(),
        codearts_sk: "test-sk".into(),
        gateway_key: "test-key".into(),
        gateway_keys: vec![],
        codearts_gateway: upstream,
        gemini_api_key: "test-gemini-key".into(),
        gemini_base_url: gemini::GEMINI_PROD.into(),
        claude_api_key: String::new(),
        claude_base_url: claude::CLAUDE_PROD.into(),
        openai_api_key: String::new(),
        openai_base_url: openai::OPENAI_PROD.into(),
        deepseek_api_key: String::new(),
        deepseek_base_url: deepseek::DEEPSEEK_PROD.into(),
        qwen_api_key: String::new(),
        qwen_base_url: qwen::QWEN_PROD.into(),
        llama_api_key: String::new(),
        llama_base_url: llama::LLAMA_PROD.into(),
        moonshot_api_key: String::new(),
        moonshot_base_url: moonshot::MOONSHOT_PROD.into(),
        glm_api_key: String::new(),
        glm_base_url: glm::GLM_PROD.into(),
        rate_limit_rpm: 60,
        audit_log_path: "audit.jsonl".to_string(),
        accounts_file: "accounts_admin.json".to_string(),
        routes_file: "routes.json".to_string(),
        audit_file: "audit_test.jsonl".to_string(),
    };
    let app = build_router(&cfg).unwrap();
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
        .post(format!(
            "http://127.0.0.1:{}/codearts/v1/chat/completions",
            port
        ))
        .header("Authorization", "Bearer test-key")
        .json(&serde_json::json!({
            "model": "auto",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let status = resp.status();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.text().await.unwrap_or_default();
    eprintln!(
        "DEBUG status={} content_type={} body={}",
        status, content_type, body
    );
    assert!(status.is_success(), "status: {}", status);
    assert!(
        content_type.contains("text/event-stream"),
        "content-type: {}",
        content_type
    );
    assert!(body.contains("data:"), "body: {}", body);
    assert!(body.contains("[DONE]"), "body: {}", body);
}
