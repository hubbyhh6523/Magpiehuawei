use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub const GOOGLE_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// One Google OAuth client (Gemini CLI or Antigravity).
#[derive(Debug, Clone)]
pub struct GoogleApp {
    pub client_id: String,
    pub client_secret: String,
    pub authorize_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub callback: String,
    pub redirect_host: String,
}

/// A Google sign-in as logins.json keeps it (same fields as Gemini CLI's
/// oauth_creds.json).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoogleAuth {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expiry_date: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

/// Build the consent-screen URL for an authorization-code flow.
pub fn authorize_url(app: &GoogleApp, redirect_uri: &str, state: &str) -> String {
    let scopes = app.scopes.join(" ");
    format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&access_type=offline&prompt=consent",
        app.authorize_url,
        urlencode(&app.client_id),
        urlencode(redirect_uri),
        urlencode(&scopes),
        urlencode(state),
    )
}

/// Trade an authorization code for tokens.
pub async fn exchange_code(app: &GoogleApp, code: &str, redirect_uri: &str) -> Result<GoogleAuth> {
    let form = [
        ("code", code),
        ("client_id", &app.client_id),
        ("client_secret", &app.client_secret),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
    ];
    let resp = reqwest::Client::new()
        .post(&app.token_url)
        .form(&form)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("token exchange failed: {} {}", status, body);
    }
    let v: serde_json::Value = serde_json::from_str(&body)?;
    Ok(GoogleAuth {
        access_token: v
            .get("access_token")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        refresh_token: v
            .get("refresh_token")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        expiry_date: v.get("expiry_date").and_then(|t| t.as_i64()).unwrap_or(0),
        project: String::new(),
    })
}

/// Refresh an access token; returns the new access token.
pub async fn refresh(app: &GoogleApp, refresh_token: &str) -> Result<String> {
    let form = [
        ("client_id", &app.client_id),
        ("client_secret", &app.client_secret),
        ("refresh_token", &refresh_token.to_string()),
        ("grant_type", &"refresh_token".to_string()),
    ];
    let resp = reqwest::Client::new()
        .post(&app.token_url)
        .form(&form)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("token refresh failed: {} {}", status, body);
    }
    let v: serde_json::Value = serde_json::from_str(&body)?;
    v.get("access_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("no access_token in refresh response"))
}

/// Persist sign-ins to logins.json, keyed by user email.
pub struct TokenStore {
    path: PathBuf,
}

impl TokenStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<HashMap<String, GoogleAuth>> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let text = std::fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn save(&self, logins: &HashMap<String, GoogleAuth>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(logins)?)?;
        Ok(())
    }

    pub fn upsert(&self, user: &str, auth: GoogleAuth) -> Result<()> {
        let mut logins = self.load()?;
        logins.insert(user.to_string(), auth);
        self.save(&logins)
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app(token_url: &str) -> GoogleApp {
        GoogleApp {
            client_id: "test-client".into(),
            client_secret: "test-secret".into(),
            authorize_url: "http://127.0.0.1/auth".into(),
            token_url: token_url.to_string(),
            scopes: vec!["scope.a".into()],
            callback: "/oauth2callback".into(),
            redirect_host: "localhost".into(),
        }
    }

    #[tokio::test]
    async fn exchange_and_refresh_roundtrip() {
        // Mock token endpoint: exchange then refresh.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/token",
            axum::routing::post(|body: String| async move {
                if body.contains("authorization_code") {
                    axum::Json(serde_json::json!({
                        "access_token": "at-1",
                        "refresh_token": "rt-1",
                        "expiry_date": 123
                    }))
                } else {
                    axum::Json(serde_json::json!({"access_token": "at-2"}))
                }
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let app_cfg = test_app(&format!("http://{}/token", addr));
        let auth = exchange_code(&app_cfg, "code-1", "http://localhost/oauth2callback")
            .await
            .unwrap();
        assert_eq!(auth.access_token, "at-1");
        assert_eq!(auth.refresh_token, "rt-1");
        let at = refresh(&app_cfg, &auth.refresh_token).await.unwrap();
        assert_eq!(at, "at-2");
    }

    #[tokio::test]
    async fn token_store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("magpie-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = TokenStore::new(dir.join("logins.json"));
        let auth = GoogleAuth {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expiry_date: 42,
            project: "proj".into(),
        };
        store.upsert("user@example.com", auth).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.get("user@example.com").unwrap().refresh_token, "rt");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
