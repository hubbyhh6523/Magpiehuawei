pub mod account;
pub mod admin;
pub mod auth;
pub mod circuit;
pub mod config;
pub mod crypto;
pub mod error;
pub mod gateway;
pub mod logs;
pub mod openai;
pub mod persist;
pub mod router;
pub mod routes;
pub mod session;
pub mod sse_tap;
pub mod statics;
pub mod store;
pub mod upstream;
pub mod usage;

use std::sync::Arc;

use crate::account::Pool;
use crate::auth::KeyAuth;
use crate::config::GatewayConfig;
use crate::crypto::Encryptor;
use crate::gateway::Gateway;
use crate::logs::RequestLog;
use crate::router::ModelRouter;
use crate::store::JsonFileStore;
use crate::usage::UsageStore;

/// Shared application state.
pub struct AppState {
    pub gateway: Arc<Gateway>,
    pub keys: Arc<KeyAuth>,
    pub pool: Arc<Pool>,
    pub store: Arc<JsonFileStore>,
    pub key_store: Arc<JsonFileStore>,
    pub encryptor: Arc<Encryptor>,
    pub logs: Arc<RequestLog>,
    pub usage: Arc<UsageStore>,
    pub cfg: GatewayConfig,
    pub listen: String,
    pub router: Arc<ModelRouter>,
    pub secret: String,
    pub trust_proxy_headers: bool,
}
