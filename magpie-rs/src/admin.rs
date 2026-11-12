use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::{routing::get, Json, Router};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

use crate::audit::AuditLog;
use crate::metrics;
use codearts_gateway::account::{Account, Pool};
use codearts_gateway::router::ModelRouter;

#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct AdminState {
    pub keys: Arc<Mutex<Vec<String>>>,
    pub audit: Arc<AuditLog>,
    pub save: Option<Arc<dyn Fn(&Vec<String>) + Send + Sync>>,
    pub save_accounts: Option<Arc<dyn Fn() + Send + Sync>>,
    pub save_routes: Option<Arc<dyn Fn() + Send + Sync>>,
    pub pool: Arc<Pool>,
    pub router: Arc<ModelRouter>,
    pub admin_key: String,
}

pub fn router(state: Arc<AdminState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/keys", get(list_keys).post(add_key))
        .route("/api/keys/{key}", axum::routing::delete(delete_key))
        .route("/api/audit", get(query_audit))
        .route("/api/accounts", get(list_accounts).post(add_account))
        .route("/api/accounts/{id}", axum::routing::delete(delete_account))
        .route("/api/routes", get(get_routes).post(set_routes))
        .route("/api/metrics", get(get_metrics))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(ADMIN_HTML)
}

async fn list_keys(State(state): State<Arc<AdminState>>) -> Json<Vec<String>> {
    let keys = state.keys.lock().unwrap().clone();
    Json(keys)
}

#[derive(Debug, Deserialize)]
pub struct AddKeyRequest {
    pub key: String,
}

async fn add_key(
    State(state): State<Arc<AdminState>>,
    Json(req): Json<AddKeyRequest>,
) -> Result<StatusCode, StatusCode> {
    let mut keys = state.keys.lock().unwrap();
    if keys.contains(&req.key) {
        return Err(StatusCode::CONFLICT);
    }
    keys.push(req.key.clone());
    if let Some(save) = &state.save {
        save(&keys);
    }
    Ok(StatusCode::CREATED)
}

async fn delete_key(State(state): State<Arc<AdminState>>, Path(key): Path<String>) -> StatusCode {
    let mut keys = state.keys.lock().unwrap();
    keys.retain(|k| k != &key);
    if let Some(save) = &state.save {
        save(&keys);
    }
    StatusCode::NO_CONTENT
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub key: Option<String>,
    pub status: Option<u16>,
    pub since: Option<u64>,
    pub until: Option<u64>,
    pub limit: Option<usize>,
}

async fn query_audit(
    State(state): State<Arc<AdminState>>,
    axum::extract::Query(q): axum::extract::Query<AuditQuery>,
) -> Json<serde_json::Value> {
    let entries = state.audit.query(
        q.key.as_deref(),
        q.status,
        q.since,
        q.until,
        q.limit.unwrap_or(100),
    );
    Json(serde_json::json!({"entries": entries}))
}

async fn list_accounts(State(state): State<Arc<AdminState>>) -> Json<serde_json::Value> {
    let accounts = state.pool.list_data();
    Json(serde_json::json!({"accounts": accounts}))
}

#[derive(Debug, Deserialize)]
pub struct AddAccountRequest {
    pub id: String,
    pub name: String,
    pub credential: String,
}

async fn add_account(
    State(state): State<Arc<AdminState>>,
    Json(req): Json<AddAccountRequest>,
) -> StatusCode {
    let account = Account::new(req.id.clone(), req.name, req.credential);
    state.pool.add(account);
    if let Some(save) = &state.save_accounts {
        save();
    }
    StatusCode::CREATED
}

async fn delete_account(
    State(state): State<Arc<AdminState>>,
    Path(id): Path<String>,
) -> StatusCode {
    if state.pool.remove(&id) {
        if let Some(save) = &state.save_accounts {
            save();
        }
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn get_routes(State(state): State<Arc<AdminState>>) -> Json<serde_json::Value> {
    let cfg = state.router.config();
    Json(serde_json::json!({"config": cfg}))
}

async fn set_routes(
    State(state): State<Arc<AdminState>>,
    Json(cfg): Json<codearts_gateway::router::RouterConfig>,
) -> StatusCode {
    state.router.replace(cfg);
    if let Some(save) = &state.save_routes {
        save();
    }
    StatusCode::NO_CONTENT
}

async fn get_metrics() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        metrics::render_metrics(),
    )
}

const ADMIN_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>magpie-rs Admin</title>
<style>
body { font-family: sans-serif; margin: 2rem; }
h1 { margin-bottom: 1rem; }
section { margin-bottom: 2rem; }
input, button, textarea { padding: 0.5rem; margin-right: 0.5rem; }
ul { list-style: none; padding: 0; }
li { margin-bottom: 0.5rem; }
pre { background: #f4f4f4; padding: 1rem; overflow-x: auto; }
#login { margin-bottom: 2rem; }
.toast { position: fixed; top: 1rem; right: 1rem; padding: 0.75rem 1.25rem; border-radius: 4px; color: #fff; display: none; z-index: 1000; }
.toast.error { background: #d32f2f; }
.toast.success { background: #388e3c; }
.spinner { display: inline-block; width: 1rem; height: 1rem; border: 2px solid #ccc; border-top-color: #333; border-radius: 50%; animation: spin 0.8s linear infinite; vertical-align: middle; margin-right: 0.5rem; }
@keyframes spin { to { transform: rotate(360deg); } }
table { border-collapse: collapse; width: 100%; }
th, td { border: 1px solid #ddd; padding: 0.5rem; text-align: left; }
th { background: #f4f4f4; }
</style>
</head>
<body>
<h1>magpie-rs Admin</h1>
<div id="toast" class="toast"></div>
<div id="login">
<input id="admin-key" placeholder="admin key" type="password">
<button onclick="login()">Login</button>
<span id="login-status"></span>
</div>
<div id="content" style="display:none">
<section>
<h2>Gateway Keys</h2>
<input id="new-key" placeholder="new key">
<button onclick="addKey()">Add</button>
<ul id="keys"></ul>
</section>
<section>
<h2>Accounts</h2>
<input id="acc-id" placeholder="id">
<input id="acc-name" placeholder="name">
<input id="acc-cred" placeholder="credential">
<button onclick="addAccount()">Add</button>
<table id="accounts-table">
<thead><tr><th>ID</th><th>Name</th><th>Status</th><th>Actions</th></tr></thead>
<tbody id="accounts"></tbody>
</table>
</section>
<section>
<h2>Routes</h2>
<textarea id="routes" rows="10" cols="80"></textarea><br>
<button onclick="saveRoutes()">Save</button>
<span id="routes-status"></span>
</section>
<section>
<h2>Audit Log</h2>
<input id="audit-key" placeholder="filter by key">
<input id="audit-status" placeholder="status (e.g. 200)">
<button onclick="loadAudit()">Query</button>
<pre id="audit"></pre>
</section>
<section>
<h2>Metrics</h2>
<pre id="metrics"></pre>
</section>
</div>
<script>
function headers() {
  const key = localStorage.getItem('admin_key') || '';
  return {'Authorization': 'Bearer ' + key, 'Content-Type': 'application/json'};
}
function showToast(msg, type) {
  const t = document.getElementById('toast');
  t.textContent = msg;
  t.className = 'toast ' + type;
  t.style.display = 'block';
  setTimeout(() => { t.style.display = 'none'; }, 3000);
}
function setLoading(btn, loading) {
  if (loading) {
    btn.disabled = true;
    btn.innerHTML = '<span class="spinner"></span>' + btn.textContent;
  } else {
    btn.disabled = false;
    btn.innerHTML = btn.textContent.replace('<span class="spinner"></span>', '');
  }
}
async function login() {
  const key = document.getElementById('admin-key').value;
  localStorage.setItem('admin_key', key);
  const res = await fetch('/admin/api/keys', {headers: headers()});
  if (res.ok) {
    document.getElementById('login').style.display = 'none';
    document.getElementById('content').style.display = 'block';
    loadKeys(); loadAccounts(); loadRoutes(); loadAudit(); loadMetrics();
  } else {
    document.getElementById('login-status').textContent = 'invalid key';
  }
}
async function loadKeys() {
  const res = await fetch('/admin/api/keys', {headers: headers()});
  const keys = await res.json();
  const ul = document.getElementById('keys');
  ul.innerHTML = '';
  keys.forEach(k => {
    const li = document.createElement('li');
    li.textContent = k;
    const btn = document.createElement('button');
    btn.textContent = 'Delete';
    btn.onclick = () => deleteKey(k);
    li.appendChild(btn);
    ul.appendChild(li);
  });
}
async function addKey() {
  const key = document.getElementById('new-key').value;
  const btn = event.target;
  setLoading(btn, true);
  const res = await fetch('/admin/api/keys', {
    method: 'POST', headers: headers(),
    body: JSON.stringify({key})
  });
  setLoading(btn, false);
  if (res.ok) {
    showToast('Key added', 'success');
    document.getElementById('new-key').value = '';
    loadKeys();
  } else {
    showToast('Failed to add key', 'error');
  }
}
async function deleteKey(key) {
  const res = await fetch('/admin/api/keys/' + encodeURIComponent(key), {method: 'DELETE', headers: headers()});
  if (res.ok) {
    showToast('Key deleted', 'success');
    loadKeys();
  } else {
    showToast('Failed to delete key', 'error');
  }
}
async function loadAccounts() {
  const res = await fetch('/admin/api/accounts', {headers: headers()});
  const data = await res.json();
  const tbody = document.getElementById('accounts');
  tbody.innerHTML = '';
  (data.accounts || []).forEach(a => {
    const tr = document.createElement('tr');
    tr.innerHTML = '<td>' + a.id + '</td><td>' + a.name + '</td><td>' + (a.status || 'unknown') + '</td>';
    const td = document.createElement('td');
    const btn = document.createElement('button');
    btn.textContent = 'Delete';
    btn.onclick = () => deleteAccount(a.id);
    td.appendChild(btn);
    tr.appendChild(td);
    tbody.appendChild(tr);
  });
}
async function addAccount() {
  const id = document.getElementById('acc-id').value;
  const name = document.getElementById('acc-name').value;
  const credential = document.getElementById('acc-cred').value;
  const btn = event.target;
  setLoading(btn, true);
  const res = await fetch('/admin/api/accounts', {
    method: 'POST', headers: headers(),
    body: JSON.stringify({id, name, credential})
  });
  setLoading(btn, false);
  if (res.ok) {
    showToast('Account added', 'success');
    document.getElementById('acc-id').value = '';
    document.getElementById('acc-name').value = '';
    document.getElementById('acc-cred').value = '';
    loadAccounts();
  } else {
    showToast('Failed to add account', 'error');
  }
}
async function deleteAccount(id) {
  const res = await fetch('/admin/api/accounts/' + encodeURIComponent(id), {method: 'DELETE', headers: headers()});
  if (res.ok) {
    showToast('Account deleted', 'success');
    loadAccounts();
  } else {
    showToast('Failed to delete account', 'error');
  }
}
async function loadRoutes() {
  const res = await fetch('/admin/api/routes', {headers: headers()});
  const data = await res.json();
  document.getElementById('routes').value = JSON.stringify(data.config, null, 2);
}
async function saveRoutes() {
  const btn = event.target;
  setLoading(btn, true);
  try {
    const cfg = JSON.parse(document.getElementById('routes').value);
    const res = await fetch('/admin/api/routes', {
      method: 'POST', headers: headers(),
      body: JSON.stringify(cfg)
    });
    setLoading(btn, false);
    if (res.ok) {
      showToast('Routes saved', 'success');
      document.getElementById('routes-status').textContent = 'saved';
      loadRoutes();
    } else {
      showToast('Failed to save routes', 'error');
      document.getElementById('routes-status').textContent = 'error';
    }
  } catch (e) {
    setLoading(btn, false);
    showToast('Invalid JSON', 'error');
    document.getElementById('routes-status').textContent = 'invalid JSON';
  }
}
async function loadAudit() {
  const key = document.getElementById('audit-key').value;
  const status = document.getElementById('audit-status').value;
  const params = new URLSearchParams();
  if (key) params.set('key', key);
  if (status) params.set('status', status);
  const res = await fetch('/admin/api/audit?' + params.toString(), {headers: headers()});
  const data = await res.json();
  document.getElementById('audit').textContent = JSON.stringify(data.entries, null, 2);
}
async function loadMetrics() {
  const res = await fetch('/admin/api/metrics', {headers: headers()});
  const text = await res.text();
  document.getElementById('metrics').textContent = text;
}
</script>
</body>
</html>"#;

#[allow(clippy::result_large_err)]
pub async fn auth_middleware(
    State(state): State<Arc<AdminState>>,
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::response::Response> {
    let key = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if key == state.admin_key {
        Ok(next.run(req).await)
    } else {
        Err((StatusCode::UNAUTHORIZED, "unauthorized").into_response())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn admin_page_renders() {
        let state = Arc::new(AdminState {
            keys: Arc::new(Mutex::new(vec!["key1".into()])),
            audit: Arc::new(AuditLog::new(10)),
            save: None,
            save_accounts: None,
            save_routes: None,
            pool: Arc::new(Pool::new()),
            router: Arc::new(ModelRouter::new(Default::default())),
            admin_key: "admin-key".into(),
        });
        let app = router(state);
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(resp.status().is_success());
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("magpie-rs Admin"));
    }

    #[tokio::test]
    async fn keys_crud() {
        let state = Arc::new(AdminState {
            keys: Arc::new(Mutex::new(vec![])),
            audit: Arc::new(AuditLog::new(10)),
            save: None,
            save_accounts: None,
            save_routes: None,
            pool: Arc::new(Pool::new()),
            router: Arc::new(ModelRouter::new(Default::default())),
            admin_key: "admin-key".into(),
        });
        let app = router(state.clone());
        // add
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/keys")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"key":"key1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);
        // list
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/keys")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("key1"));
        // delete
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/keys/key1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);
    }
}
