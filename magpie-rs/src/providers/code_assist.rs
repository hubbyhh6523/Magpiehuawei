use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const CODE_ASSIST_PROD: &str = "https://cloudcode-pa.googleapis.com";
pub const CODE_ASSIST_DAILY: &str = "https://daily-cloudcode-pa.googleapis.com";

pub fn code_assist_envelope(
    agent: &str,
    body: &Value,
    project: &str,
    wire_model: impl Fn(&str) -> String,
) -> Result<(Value, String)> {
    let mut env = body.clone();
    let model = env
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    env["project"] = json!(project);
    let id = random_token(12);
    if agent == "antigravity" {
        env["userAgent"] = json!("antigravity");
        env["requestType"] = json!("agent");
        env["requestId"] = json!(format!("agent-{}", id));
        if let Some(req) = env.get_mut("request").and_then(|r| r.as_object_mut()) {
            let session = antigravity_session(&Value::Object(req.clone()));
            req.insert("sessionId".into(), json!(session));
        }
    } else {
        env["user_prompt_id"] = json!(id);
        if !model.is_empty() {
            env["model"] = json!(wire_model(&model));
        }
    }
    Ok((env, model))
}

pub fn antigravity_session(request: &Value) -> String {
    let mut first = String::new();
    if let Some(contents) = request.get("contents").and_then(|c| c.as_array()) {
        for c in contents {
            if c.get("role").and_then(|r| r.as_str()) != Some("user") {
                continue;
            }
            if let Some(parts) = c.get("parts").and_then(|p| p.as_array()) {
                for p in parts {
                    if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                        if !t.is_empty() {
                            first = t.to_string();
                            break;
                        }
                    }
                }
            }
            break;
        }
    }
    if first.is_empty() {
        first = random_token(16);
    }
    let sum = Sha256::digest(first.as_bytes());
    let n = u64::from_be_bytes([
        sum[0], sum[1], sum[2], sum[3], sum[4], sum[5], sum[6], sum[7],
    ]) & 0x7fffffffffffffff;
    format!("-{}", n)
}

pub async fn call(
    http: &reqwest::Client,
    base: &str,
    method: &str,
    token: &str,
    user_agent: &str,
    body: &Value,
) -> Result<Value> {
    let url = format!("{}/v1internal:{}", base, method);
    let resp = http
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", user_agent)
        .json(body)
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("code assist {}: {} {}", method, status, text);
    }
    Ok(serde_json::from_str(&text)?)
}

#[derive(Debug, Deserialize)]
pub struct CodeAssistTier {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize)]
pub struct LoadCodeAssistReply {
    #[serde(default, rename = "currentTier")]
    pub current_tier: Option<CodeAssistTier>,
    #[serde(default, rename = "paidTier")]
    pub paid_tier: Option<CodeAssistTier>,
}

impl LoadCodeAssistReply {
    pub fn tiers(&self) -> (String, String) {
        (
            self.current_tier
                .as_ref()
                .map(|t| t.id.clone())
                .unwrap_or_default(),
            self.paid_tier
                .as_ref()
                .map(|t| t.id.clone())
                .unwrap_or_default(),
        )
    }
}

fn random_token(n: usize) -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..n)
        .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_envelope_adds_project_and_prompt_id() {
        let body = json!({"model": "gemini-2.5-flash", "request": {"contents": []}});
        let (env, model) =
            code_assist_envelope("gemini", &body, "proj-1", |m| format!("wire-{}", m)).unwrap();
        assert_eq!(model, "gemini-2.5-flash");
        assert_eq!(env["project"], "proj-1");
        assert_eq!(env["user_prompt_id"].as_str().unwrap().len(), 12);
        assert_eq!(env["model"], "wire-gemini-2.5-flash");
    }

    #[test]
    fn antigravity_envelope_adds_agent_fields_and_stable_session() {
        let body = json!({"model": "m", "request": {"contents": [
            {"role": "user", "parts": [{"text": "hello"}]}
        ]}});
        let (env, _) = code_assist_envelope("antigravity", &body, "p", |m| m.to_string()).unwrap();
        assert_eq!(env["userAgent"], "antigravity");
        assert_eq!(env["requestType"], "agent");
        assert!(env["requestId"].as_str().unwrap().starts_with("agent-"));
        let s1 = env["request"]["sessionId"].as_str().unwrap().to_string();
        let (env2, _) = code_assist_envelope("antigravity", &body, "p", |m| m.to_string()).unwrap();
        assert_eq!(env2["request"]["sessionId"], s1);
    }

    #[tokio::test]
    async fn call_posts_to_v1internal_with_bearer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/v1internal:loadCodeAssist",
            axum::routing::post(
                |headers: axum::http::HeaderMap, body: axum::Json<Value>| async move {
                    assert_eq!(headers.get("authorization").unwrap(), "Bearer test-token");
                    assert_eq!(body.0["project"], "proj");
                    axum::Json(serde_json::json!({
                        "currentTier": {"id": "free", "name": "Free", "isDefault": true},
                        "paidTier": {"id": "standard", "name": "Standard"}
                    }))
                },
            ),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let http = reqwest::Client::new();
        let reply = call(
            &http,
            &format!("http://{}", addr),
            "loadCodeAssist",
            "test-token",
            "magpie-test",
            &json!({"project": "proj"}),
        )
        .await
        .unwrap();
        let reply: LoadCodeAssistReply = serde_json::from_value(reply).unwrap();
        let (current, paid) = reply.tiers();
        assert_eq!(current, "free");
        assert_eq!(paid, "standard");
    }
}

/// Streaming generateContent: POST `{base}/v1internal:generateContent?alt=sse`
/// and return the raw upstream SSE byte stream.
pub async fn generate_content_stream(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    user_agent: &str,
    body: &Value,
) -> Result<impl futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>>> {
    let url = format!("{}/v1internal:generateContent?alt=sse", base);
    let resp = http
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", user_agent)
        .json(body)
        .send()
        .await?;
    let status = resp.status();
    if status.as_u16() >= 400 {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("generateContent: {} {}", status, text);
    }
    Ok(resp.bytes_stream())
}

/// Unwrap Code Assist's `{response: ...}` SSE frames into bare Gemini SSE
/// frames; non-JSON and error frames pass through untouched.
pub fn unwrap_response_stream(
    stream: impl futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>>,
) -> impl futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>> {
    use futures_util::StreamExt;
    stream.map(|chunk| match chunk {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let mut out = String::new();
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        if let Some(inner) = v.get("response") {
                            out.push_str(&format!(
                                "data: {}\n\n",
                                serde_json::to_string(inner).unwrap_or_default()
                            ));
                            continue;
                        }
                    }
                    out.push_str(line);
                    out.push('\n');
                } else {
                    out.push_str(line);
                    out.push('\n');
                }
            }
            Ok(bytes::Bytes::from(out))
        }
        Err(e) => Err(e),
    })
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn streams_and_unwraps_response_frames() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/v1internal:generateContent",
            axum::routing::post(|| async {
                axum::response::Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(axum::body::Body::from(
                        "data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}]}}]}}\n\n"
                            .to_string(),
                    ))
                    .unwrap()
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let http = reqwest::Client::new();
        let raw = generate_content_stream(
            &http,
            &format!("http://{}", addr),
            "tok",
            "magpie-test",
            &json!({"model": "m", "project": "p", "request": {}}),
        )
        .await
        .unwrap();
        let unwrapped = unwrap_response_stream(raw);
        let body: Vec<u8> = unwrapped
            .map(|r| r.unwrap())
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .flatten()
            .collect();
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("candidates"), "body: {}", text);
        assert!(!text.contains("\"response\":"), "body: {}", text);
    }
}
