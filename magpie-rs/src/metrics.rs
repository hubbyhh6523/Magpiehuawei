use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

pub static REQUESTS_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static REQUEST_ERRORS_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static DURATION_SUM_MS: AtomicU64 = AtomicU64::new(0);
pub static DURATION_COUNT: AtomicU64 = AtomicU64::new(0);

static REQUESTS_BY_PROVIDER: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

// Histogram buckets in ms: 10, 50, 100, 500, 1000, 5000, +Inf
const BUCKETS_MS: [u64; 6] = [10, 50, 100, 500, 1000, 5000];
static DURATION_BUCKETS: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

pub fn inc_requests() {
    REQUESTS_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn inc_errors() {
    REQUEST_ERRORS_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn inc_requests_provider(provider: &str) {
    let mut guard = REQUESTS_BY_PROVIDER.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    *map.entry(provider.to_string()).or_insert(0) += 1;
}

pub fn observe_duration_ms(ms: u64) {
    DURATION_SUM_MS.fetch_add(ms, Ordering::Relaxed);
    DURATION_COUNT.fetch_add(1, Ordering::Relaxed);
    for (i, bucket) in BUCKETS_MS.iter().enumerate() {
        if ms <= *bucket {
            DURATION_BUCKETS[i].fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub struct RequestTimer {
    start: Instant,
}

impl RequestTimer {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

pub fn render_metrics() -> String {
    let total = REQUESTS_TOTAL.load(Ordering::Relaxed);
    let errors = REQUEST_ERRORS_TOTAL.load(Ordering::Relaxed);
    let sum_ms = DURATION_SUM_MS.load(Ordering::Relaxed);
    let count = DURATION_COUNT.load(Ordering::Relaxed);
    let mut out = String::new();
    out.push_str("# HELP magpie_requests_total Total chat completion requests.\n");
    out.push_str("# TYPE magpie_requests_total counter\n");
    out.push_str(&format!("magpie_requests_total {}\n", total));
    out.push_str("# HELP magpie_request_errors_total Total failed chat completion requests.\n");
    out.push_str("# TYPE magpie_request_errors_total counter\n");
    out.push_str(&format!("magpie_request_errors_total {}\n", errors));

    // Per-provider counters
    out.push_str(
        "# HELP magpie_requests_by_provider_total Chat completion requests by provider.\n",
    );
    out.push_str("# TYPE magpie_requests_by_provider_total counter\n");
    let guard = REQUESTS_BY_PROVIDER.lock().unwrap();
    if let Some(map) = guard.as_ref() {
        for (provider, count) in map {
            out.push_str(&format!(
                "magpie_requests_by_provider_total{{provider=\"{}\"}} {}\n",
                provider, count
            ));
        }
    }
    drop(guard);

    // Histogram
    out.push_str("# HELP magpie_request_duration_ms Chat completion request duration in ms.\n");
    out.push_str("# TYPE magpie_request_duration_ms histogram\n");
    let mut cumulative = 0u64;
    for (i, bucket) in BUCKETS_MS.iter().enumerate() {
        cumulative += DURATION_BUCKETS[i].load(Ordering::Relaxed);
        out.push_str(&format!(
            "magpie_request_duration_ms_bucket{{le=\"{}\"}} {}\n",
            bucket, cumulative
        ));
    }
    out.push_str(&format!(
        "magpie_request_duration_ms_bucket{{le=\"+Inf\"}} {}\n",
        count
    ));
    out.push_str(&format!("magpie_request_duration_ms_sum {}\n", sum_ms));
    out.push_str(&format!("magpie_request_duration_ms_count {}\n", count));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_increment() {
        let before = REQUESTS_TOTAL.load(Ordering::Relaxed);
        inc_requests();
        assert!(REQUESTS_TOTAL.load(Ordering::Relaxed) > before);
    }

    #[test]
    fn per_provider_counter() {
        inc_requests_provider("gemini");
        inc_requests_provider("gemini");
        let out = render_metrics();
        assert!(out.contains("provider=\"gemini\""));
    }

    #[test]
    fn histogram_buckets() {
        observe_duration_ms(5);
        observe_duration_ms(120);
        let out = render_metrics();
        assert!(out.contains("magpie_request_duration_ms_bucket{le=\"10\"}"));
        assert!(out.contains("magpie_request_duration_ms_bucket{le=\"500\"}"));
        assert!(out.contains("magpie_request_duration_ms_bucket{le=\"+Inf\"}"));
    }

    #[test]
    fn renders_prometheus_text() {
        let out = render_metrics();
        assert!(out.contains("magpie_requests_total"));
        assert!(out.contains("magpie_request_errors_total"));
        assert!(out.contains("magpie_request_duration_ms"));
    }
}
