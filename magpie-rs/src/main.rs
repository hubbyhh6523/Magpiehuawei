use anyhow::Result;
use serde::Deserialize;
use tracing_subscriber::EnvFilter;

use std::sync::Arc;

use codearts_gateway::{
    account::Pool, auth::KeyAuth, config::GatewayConfig, crypto::Encryptor, gateway::Gateway,
    logs::RequestLog, router::ModelRouter, store::JsonFileStore, upstream::CodeArtsClient,
    usage::UsageStore, AppState,
};

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default = "default_listen")]
    listen: String,
    #[serde(default)]
    codearts_ak: String,
    #[serde(default)]
    codearts_sk: String,
    #[serde(default)]
    gateway_key: String,
}
fn default_listen() -> String {
    "127.0.0.1:8080".into()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let cfg: Config = config::Config::builder()
        .add_source(config::File::with_name("config").required(false))
        .add_source(config::Environment::with_prefix("MAGPIE_RS"))
        .build()?
        .try_deserialize()?;

    let gw_cfg = GatewayConfig::default();
    let encryptor = Arc::new(Encryptor::new(&cfg.gateway_key));
    let store = Arc::new(JsonFileStore::new("accounts.json".to_string()));
    let pool = Arc::new(Pool::new());
    for acc in store.load()? {
        pool.add_data(acc);
    }
    let keys = Arc::new(KeyAuth::new(
        &cfg.gateway_key,
        vec![cfg.gateway_key.clone()],
    ));
    let router = Arc::new(ModelRouter::new(store.load_router().unwrap_or_default()));
    let upstream = Arc::new(CodeArtsClient::new(
        cfg.codearts_ak.clone(),
        cfg.codearts_sk.clone(),
        codearts_gateway::upstream::CODEARTS_GATEWAY_PROD.to_string(),
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

    let codearts = codearts_gateway::routes::v1_router(state);
    let app = axum::Router::new()
        .route("/healthz", axum::routing::get(|| async { "ok" }))
        .nest("/codearts", codearts);
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    tracing::info!(listen = %cfg.listen, "magpie-rs listening");
    axum::serve(listener, app).await?;
    Ok(())
}
