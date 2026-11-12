use anyhow::Result;
use magpie_rs::gateway_app::{build_router, Config};
use tracing_subscriber::EnvFilter;

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
    let app = build_router(&cfg)?;
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    tracing::info!(listen = %cfg.listen, "magpie-rs listening");
    axum::serve(listener, app).await?;
    Ok(())
}
