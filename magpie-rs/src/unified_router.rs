use axum::extract::State;
use axum::http::StatusCode;
use axum::{routing::post, Json, Router};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::providers::claude::ClaudeState;
use crate::providers::deepseek::DeepSeekState;
use crate::providers::gemini::GeminiState;
use crate::providers::glm::GlmState;
use crate::providers::llama::LlamaState;
use crate::providers::moonshot::MoonshotState;
use crate::providers::openai::OpenAIState;
use crate::providers::qwen::QwenState;

#[derive(Clone)]
pub struct UnifiedState {
    pub gemini: Arc<GeminiState>,
    pub claude: Arc<ClaudeState>,
    pub openai: Arc<OpenAIState>,
    pub deepseek: Arc<DeepSeekState>,
    pub qwen: Arc<QwenState>,
    pub llama: Arc<LlamaState>,
    pub moonshot: Arc<MoonshotState>,
    pub glm: Arc<GlmState>,
}

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    #[serde(default)]
    pub stream: bool,
}

pub fn detect_provider(model: &str) -> &'static str {
    if model.starts_with("gemini") {
        "gemini"
    } else if model.starts_with("claude") {
        "claude"
    } else if model.starts_with("glm") {
        "glm"
    } else if model.starts_with("moonshot") {
        "moonshot"
    } else if model.starts_with("llama") {
        "llama"
    } else if model.starts_with("qwen") {
        "qwen"
    } else if model.starts_with("deepseek") {
        "deepseek"
    } else if model.starts_with("gpt") || model.starts_with("o1") || model.starts_with("codex") {
        "openai"
    } else {
        "unknown"
    }
}

pub fn router(state: Arc<UnifiedState>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(handle_chat))
        .with_state(state)
}

async fn handle_chat(
    State(state): State<Arc<UnifiedState>>,
    Json(req): Json<ChatRequest>,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let timer = crate::metrics::RequestTimer::start();
    crate::metrics::inc_requests();
    crate::metrics::inc_requests_provider(detect_provider(&req.model));
    let result = match detect_provider(&req.model) {
        "gemini" => {
            let gemini_req = crate::providers::gemini::ChatRequest {
                model: req.model.clone(),
                messages: req
                    .messages
                    .iter()
                    .map(|m| {
                        serde_json::from_value(m.clone()).unwrap_or(
                            crate::providers::gemini::Message {
                                role: "user".into(),
                                content: Value::String(String::new()),
                            },
                        )
                    })
                    .collect(),
                stream: req.stream,
            };
            crate::providers::gemini::handle_chat(State(state.gemini.clone()), Json(gemini_req))
                .await
        }
        "claude" => {
            let claude_req = crate::providers::claude::ChatRequest {
                model: req.model.clone(),
                messages: req
                    .messages
                    .iter()
                    .map(|m| {
                        serde_json::from_value(m.clone()).unwrap_or(
                            crate::providers::claude::Message {
                                role: "user".into(),
                                content: Value::String(String::new()),
                            },
                        )
                    })
                    .collect(),
                stream: req.stream,
            };
            crate::providers::claude::handle_chat(State(state.claude.clone()), Json(claude_req))
                .await
        }
        "glm" => {
            let glm_req = crate::providers::glm::ChatRequest {
                model: req.model.clone(),
                messages: req.messages.clone(),
                stream: req.stream,
            };
            crate::providers::glm::handle_chat(State(state.glm.clone()), Json(glm_req)).await
        }
        "moonshot" => {
            let moonshot_req = crate::providers::moonshot::ChatRequest {
                model: req.model.clone(),
                messages: req.messages.clone(),
                stream: req.stream,
            };
            crate::providers::moonshot::handle_chat(
                State(state.moonshot.clone()),
                Json(moonshot_req),
            )
            .await
        }
        "llama" => {
            let llama_req = crate::providers::llama::ChatRequest {
                model: req.model.clone(),
                messages: req.messages.clone(),
                stream: req.stream,
            };
            crate::providers::llama::handle_chat(State(state.llama.clone()), Json(llama_req)).await
        }
        "qwen" => {
            let qwen_req = crate::providers::qwen::ChatRequest {
                model: req.model.clone(),
                messages: req.messages.clone(),
                stream: req.stream,
            };
            crate::providers::qwen::handle_chat(State(state.qwen.clone()), Json(qwen_req)).await
        }
        "deepseek" => {
            let deepseek_req = crate::providers::deepseek::ChatRequest {
                model: req.model.clone(),
                messages: req.messages.clone(),
                stream: req.stream,
            };
            crate::providers::deepseek::handle_chat(
                State(state.deepseek.clone()),
                Json(deepseek_req),
            )
            .await
        }
        "openai" => {
            let openai_req = crate::providers::openai::ChatRequest {
                model: req.model.clone(),
                messages: req.messages.clone(),
                stream: req.stream,
            };
            crate::providers::openai::handle_chat(State(state.openai.clone()), Json(openai_req))
                .await
        }
        _ => Err(StatusCode::BAD_REQUEST),
    };
    if result.is_err() {
        crate::metrics::inc_errors();
    }
    crate::metrics::observe_duration_ms(timer.elapsed_ms());
    tracing::info!(
        model = %req.model,
        provider = detect_provider(&req.model),
        duration_ms = timer.elapsed_ms(),
        "chat completion"
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_provider_by_model_prefix() {
        assert_eq!(detect_provider("gemini-2.5-flash"), "gemini");
        assert_eq!(detect_provider("claude-sonnet-4-5"), "claude");
        assert_eq!(detect_provider("gpt-4o"), "openai");
        assert_eq!(detect_provider("o1-preview"), "openai");
        assert_eq!(detect_provider("codex-mini"), "openai");
        assert_eq!(detect_provider("unknown-model"), "unknown");
    }
}
