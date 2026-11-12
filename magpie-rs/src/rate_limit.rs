use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::IntoResponse;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct RateLimiter {
    pub limit: usize,
    pub window: Duration,
    buckets: std::sync::Arc<Mutex<HashMap<String, Vec<Instant>>>>,
}

impl RateLimiter {
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            limit,
            window,
            buckets: std::sync::Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn check(&self, key: &str) -> bool {
        let mut guard = self.buckets.lock().unwrap();
        let now = Instant::now();
        let entry = guard.entry(key.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t) < self.window);
        if entry.len() >= self.limit {
            false
        } else {
            entry.push(now);
            true
        }
    }
}

#[allow(clippy::result_large_err)]
pub async fn middleware(
    State(limiter): State<RateLimiter>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<axum::response::Response, axum::response::Response> {
    let key = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("anonymous")
        .to_string();
    if limiter.check(&key) {
        Ok(next.run(req).await)
    } else {
        Err((StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_within_limit() {
        let limiter = RateLimiter::new(3, Duration::from_secs(1));
        assert!(limiter.check("key1"));
        assert!(limiter.check("key1"));
        assert!(limiter.check("key1"));
        assert!(!limiter.check("key1"));
    }

    #[test]
    fn separate_keys_independent() {
        let limiter = RateLimiter::new(1, Duration::from_secs(1));
        assert!(limiter.check("key1"));
        assert!(!limiter.check("key1"));
        assert!(limiter.check("key2"));
    }
}
