use anyhow::Result;
use axum::Router;
use serde::Deserialize;
use std::sync::Arc;

use crate::providers::gemini::{self, GeminiState};
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
    #[serde(default = "default_codearts_gateway")]
    pub codearts_gateway: String,
    #[serde(default)]
    pub gemini_api_key: String,
    #[serde(default = "default_gemini_base")]
    pub gemini_base_url: String,
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
    let keys = Arc::new(KeyAuth::new(
        &cfg.gateway_key,
        vec![cfg.gateway_key.clone()],
    ));
    let router = Arc::new(ModelRouter::new(store.load_router().unwrap_or_default()));
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
        pool,
        store: store.clone(),
        key_store: store,
        encryptor,
        logs,
        usage,
        cfg: gw_cfg,
        listen: cfg.listen.clone(),
        router,
        secret: cfg.gateway_key.clone(),
        trust_proxy_headers: true,
    });

    let codearts = v1_router(state);
    let gemini_state = Arc::new(GeminiState {
        api_key: cfg.gemini_api_key.clone(),
        base_url: cfg.gemini_base_url.clone(),
        http: reqwest::Client::new(),
    });
    let gemini = gemini::router(gemini_state);
    Ok(Router::new()
        .route("/healthz", axum::routing::get(|| async { "ok" }))
        .nest("/codearts", codearts)
        .nest("/gemini", gemini))
}
