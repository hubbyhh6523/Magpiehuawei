use anyhow::Result;
use serde::Deserialize;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default = "default_listen")]
    listen: String,
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
        .add_source(config::File::with_name("magpie-rs/config").required(false))
        .add_source(config::Environment::with_prefix("MAGPIE_RS"))
        .build()?
        .try_deserialize()?;
    let app = axum::Router::new().route("/healthz", axum::routing::get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    tracing::info!(listen = %cfg.listen, "magpie-rs listening");
    axum::serve(listener, app).await?;
    Ok(())
}
