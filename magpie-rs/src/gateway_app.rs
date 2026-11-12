use anyhow::Result;
use axum::extract::Query;
use axum::Router;
use serde::Deserialize;
use std::sync::Arc;

use crate::admin::{self, AdminState};
use crate::providers::claude::{self, ClaudeState};
use crate::providers::deepseek::{self, DeepSeekState};
use crate::providers::gemini::{self, GeminiState};
use crate::providers::glm::{self, GlmState};
use crate::providers::llama::{self, LlamaState};
use crate::providers::moonshot::{self, MoonshotState};
use crate::providers::openai::{self, OpenAIState};
use crate::providers::qwen::{self, QwenState};
use crate::rate_limit::{self, RateLimiter};
use crate::unified_router::{self, UnifiedState};
use codearts_gateway::{
    account::Pool,
    auth::KeyAuth,
    config::GatewayConfig,
    crypto::Encryptor,
    gateway::Gateway,
    logs::RequestLog,
    router::ModelRouter,
    routes::v1_router,
    store::JsonFileStore,
    upstream::{CodeArtsClient, CODEARTS_GATEWAY_PROD},
    usage::UsageStore,
    AppState,
};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default)]
    pub codearts_ak: String,
    #[serde(default)]
    pub codearts_sk: String,
    #[serde(default)]
    pub gateway_key: String,
    #[serde(default)]
    pub gateway_keys: Vec<String>,
    #[serde(default = "default_codearts_gateway")]
    pub codearts_gateway: String,
    #[serde(default)]
    pub gemini_api_key: String,
    #[serde(default = "default_gemini_base")]
    pub gemini_base_url: String,
    #[serde(default)]
    pub claude_api_key: String,
    #[serde(default = "default_claude_base")]
    pub claude_base_url: String,
    #[serde(default)]
    pub openai_api_key: String,
    #[serde(default = "default_openai_base")]
    pub openai_base_url: String,
    #[serde(default)]
    pub deepseek_api_key: String,
    #[serde(default = "default_deepseek_base")]
    pub deepseek_base_url: String,
    #[serde(default)]
    pub qwen_api_key: String,
    #[serde(default = "default_qwen_base")]
    pub qwen_base_url: String,
    #[serde(default)]
    pub llama_api_key: String,
    #[serde(default = "default_llama_base")]
    pub llama_base_url: String,
    #[serde(default)]
    pub moonshot_api_key: String,
    #[serde(default = "default_moonshot_base")]
    pub moonshot_base_url: String,
    #[serde(default)]
    pub glm_api_key: String,
    #[serde(default = "default_glm_base")]
    pub glm_base_url: String,
    #[serde(default = "default_rate_limit_rpm")]
    pub rate_limit_rpm: u32,
    #[serde(default = "default_audit_log_path")]
    pub audit_log_path: String,
    #[serde(default = "default_accounts_file")]
    pub accounts_file: String,
    #[serde(default = "default_routes_file")]
    pub routes_file: String,
    #[serde(default = "default_audit_file")]
    pub audit_file: String,
}

fn default_audit_file() -> String {
    "audit.jsonl".to_string()
}

fn default_routes_file() -> String {
    "routes.json".to_string()
}

fn default_accounts_file() -> String {
    "accounts_admin.json".to_string()
}

fn default_audit_log_path() -> String {
    "audit.jsonl".to_string()
}

fn default_rate_limit_rpm() -> u32 {
    60
}

fn default_glm_base() -> String {
    glm::GLM_PROD.to_string()
}

fn default_moonshot_base() -> String {
    moonshot::MOONSHOT_PROD.to_string()
}

fn default_llama_base() -> String {
    llama::LLAMA_PROD.to_string()
}

fn default_qwen_base() -> String {
    qwen::QWEN_PROD.to_string()
}

fn default_deepseek_base() -> String {
    deepseek::DEEPSEEK_PROD.to_string()
}

fn default_openai_base() -> String {
    openai::OPENAI_PROD.to_string()
}

fn default_claude_base() -> String {
    claude::CLAUDE_PROD.to_string()
}

fn default_gemini_base() -> String {
    gemini::GEMINI_PROD.to_string()
}

fn default_listen() -> String {
    "127.0.0.1:8080".into()
}

fn default_codearts_gateway() -> String {
    CODEARTS_GATEWAY_PROD.to_string()
}

/// Build the magpie-rs router: /healthz plus the embedded CodeArts gateway
/// under /codearts. This is the single gateway entry point.
pub fn build_router(cfg: &Config) -> Result<Router> {
    let gw_cfg = GatewayConfig::default();
    let encryptor = Arc::new(Encryptor::new(&cfg.gateway_key));
    let store = Arc::new(JsonFileStore::new("accounts.json".to_string()));
    let pool = Arc::new(Pool::new());
    for acc in store.load()? {
        pool.add_data(acc);
    }
    if pool.list().is_empty() {
        pool.add(codearts_gateway::account::Account::new(
            "default".into(),
            "default".into(),
            cfg.codearts_ak.clone(),
        ));
    }
    let mut all_keys = cfg.gateway_keys.clone();
    if !cfg.gateway_key.is_empty() {
        all_keys.push(cfg.gateway_key.clone());
    }
    let keys = Arc::new(KeyAuth::new(
        all_keys.first().map(|s| s.as_str()).unwrap_or(""),
        all_keys.clone(),
    ));
    let router = Arc::new(ModelRouter::new(store.load_router().unwrap_or_default()));
    // Load admin-managed routes from file (override store config if present)
    if let Ok(content) = std::fs::read_to_string(&cfg.routes_file) {
        if let Ok(cfg_routes) =
            serde_json::from_str::<codearts_gateway::router::RouterConfig>(&content)
        {
            router.replace(cfg_routes);
        }
    }
    let save_routes = {
        let router = router.clone();
        let path = cfg.routes_file.clone();
        move || {
            let cfg_routes = router.config();
            if let Ok(json) = serde_json::to_string(&cfg_routes) {
                let _ = std::fs::write(&path, json);
            }
        }
    };
    let upstream = Arc::new(CodeArtsClient::new(
        cfg.codearts_ak.clone(),
        cfg.codearts_sk.clone(),
        cfg.codearts_gateway.clone(),
    ));
    let gateway = Arc::new(Gateway::new(
        pool.clone(),
        upstream,
        keys.clone(),
        gw_cfg.clone(),
        router.clone(),
    ));
    let logs = Arc::new(RequestLog::new(500));
    let usage = Arc::new(UsageStore::open("usage.db")?);
    let state = Arc::new(AppState {
        gateway,
        keys,
        pool: pool.clone(),
        store: store.clone(),
        key_store: store,
        encryptor,
        logs,
        usage,
        cfg: gw_cfg,
        listen: cfg.listen.clone(),
        router: router.clone(),
        secret: cfg.gateway_key.clone(),
        trust_proxy_headers: true,
    });

    let codearts = v1_router(state);
    let gemini_state = Arc::new(GeminiState {
        api_key: cfg.gemini_api_key.clone(),
        base_url: cfg.gemini_base_url.clone(),
        http: reqwest::Client::new(),
    });
    let gemini = gemini::router(gemini_state.clone());
    let claude_state = Arc::new(ClaudeState {
        api_key: cfg.claude_api_key.clone(),
        base_url: cfg.claude_base_url.clone(),
        http: reqwest::Client::new(),
    });
    let claude = claude::router(claude_state.clone());
    let openai_state = Arc::new(OpenAIState {
        api_key: cfg.openai_api_key.clone(),
        base_url: cfg.openai_base_url.clone(),
        http: reqwest::Client::new(),
    });
    let openai = openai::router(openai_state.clone());
    let deepseek_state = Arc::new(DeepSeekState {
        api_key: cfg.deepseek_api_key.clone(),
        base_url: cfg.deepseek_base_url.clone(),
        http: reqwest::Client::new(),
    });
    let deepseek = deepseek::router(deepseek_state.clone());
    let qwen_state = Arc::new(QwenState {
        api_key: cfg.qwen_api_key.clone(),
        base_url: cfg.qwen_base_url.clone(),
        http: reqwest::Client::new(),
    });
    let qwen = qwen::router(qwen_state.clone());
    let llama_state = Arc::new(LlamaState {
        api_key: cfg.llama_api_key.clone(),
        base_url: cfg.llama_base_url.clone(),
        http: reqwest::Client::new(),
    });
    let llama = llama::router(llama_state.clone());
    let moonshot_state = Arc::new(MoonshotState {
        api_key: cfg.moonshot_api_key.clone(),
        base_url: cfg.moonshot_base_url.clone(),
        http: reqwest::Client::new(),
    });
    let moonshot = moonshot::router(moonshot_state.clone());
    let glm_state = Arc::new(GlmState {
        api_key: cfg.glm_api_key.clone(),
        base_url: cfg.glm_base_url.clone(),
        http: reqwest::Client::new(),
    });
    let glm = glm::router(glm_state.clone());
    let unified_state = Arc::new(UnifiedState {
        gemini: gemini_state.clone(),
        claude: claude_state.clone(),
        openai: openai_state.clone(),
        deepseek: deepseek_state.clone(),
        qwen: qwen_state.clone(),
        llama: llama_state.clone(),
        moonshot: moonshot_state.clone(),
        glm: glm_state.clone(),
    });
    let audit_log = std::sync::Arc::new(crate::audit::AuditLog::with_file(1000, &cfg.audit_file));
    // Load admin-managed accounts from file
    if let Ok(content) = std::fs::read_to_string(&cfg.accounts_file) {
        if let Ok(accounts) =
            serde_json::from_str::<Vec<codearts_gateway::account::AccountData>>(&content)
        {
            for data in accounts {
                pool.add_data(data);
            }
        }
    }
    let save_accounts = {
        let pool = pool.clone();
        let path = cfg.accounts_file.clone();
        move || {
            let accounts = pool.list_data();
            if let Ok(json) = serde_json::to_string(&accounts) {
                let _ = std::fs::write(&path, json);
            }
        }
    };
    let admin_keys = std::sync::Arc::new(std::sync::Mutex::new(all_keys.clone()));
    let save_keys = {
        let keys_path = "keys.json".to_string();
        move |keys: &Vec<String>| {
            if let Ok(json) = serde_json::to_string(keys) {
                let _ = std::fs::write(&keys_path, json);
            }
        }
    };
    let admin_state = Arc::new(AdminState {
        keys: admin_keys.clone(),
        audit: audit_log.clone(),
        save: Some(std::sync::Arc::new(save_keys)),
        save_accounts: Some(std::sync::Arc::new(save_accounts)),
        save_routes: Some(std::sync::Arc::new(save_routes)),
        pool: pool.clone(),
        router: router.clone(),
        admin_key: cfg.gateway_key.clone(),
    });
    let admin = admin::router(admin_state.clone()).layer(axum::middleware::from_fn_with_state(
        admin_state.clone(),
        admin::auth_middleware,
    ));
    let limiter = RateLimiter::new(
        cfg.rate_limit_rpm as usize,
        std::time::Duration::from_secs(60),
    );
    let unified = unified_router::router(unified_state).layer(
        axum::middleware::from_fn_with_state(limiter.clone(), rate_limit::middleware),
    );
    Ok(Router::new()
        .route("/healthz", axum::routing::get(|| async { "ok" }))
        .route(
            "/audit",
            axum::routing::get({
                let audit = audit_log.clone();
                move |Query(params): Query<std::collections::HashMap<String, String>>| async move {
                    let key = params.get("key").cloned();
                    let status = params.get("status").and_then(|v| v.parse::<u16>().ok());
                    let since = params.get("since").and_then(|v| v.parse::<u64>().ok());
                    let until = params.get("until").and_then(|v| v.parse::<u64>().ok());
                    let limit = params
                        .get("limit")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(100);
                    let entries = audit.query(key.as_deref(), status, since, until, limit);
                    axum::Json(entries)
                }
            }),
        )
        .route(
            "/metrics",
            axum::routing::get(|| async { crate::metrics::render_metrics() }),
        )
        .nest("/codearts", codearts)
        .nest("/gemini", gemini)
        .nest("/claude", claude)
        .nest("/openai", openai)
        .nest("/deepseek", deepseek)
        .nest("/qwen", qwen)
        .nest("/llama", llama)
        .nest("/moonshot", moonshot)
        .nest("/glm", glm)
        .nest("/v1", unified)
        .nest("/admin", admin))
}
