use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub ts: u64,
    pub key: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u64,
}

pub struct AuditLog {
    entries: Mutex<Vec<AuditEntry>>,
    capacity: usize,
    file: Option<Mutex<std::fs::File>>,
}

impl AuditLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            capacity,
            file: None,
        }
    }

    pub fn with_file(capacity: usize, path: &str) -> Self {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(Mutex::new);
        Self {
            entries: Mutex::new(Vec::new()),
            capacity,
            file,
        }
    }

    pub fn log(&self, key: &str, method: &str, path: &str, status: u16, duration_ms: u64) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = AuditEntry {
            ts,
            key: key.to_string(),
            method: method.to_string(),
            path: path.to_string(),
            status,
            duration_ms,
        };
        // Append to file if configured
        if let Some(file) = &self.file {
            if let Ok(mut f) = file.lock() {
                if let Ok(json) = serde_json::to_string(&entry) {
                    let _ = writeln!(f, "{}", json);
                }
            }
        }
        let mut guard = self.entries.lock().unwrap();
        guard.push(entry);
        if guard.len() > self.capacity {
            guard.remove(0);
        }
    }

    pub fn query(
        &self,
        key: Option<&str>,
        status: Option<u16>,
        since: Option<u64>,
        until: Option<u64>,
        limit: usize,
    ) -> Vec<AuditEntry> {
        let guard = self.entries.lock().unwrap();
        guard
            .iter()
            .rev()
            .filter(|e| {
                if let Some(k) = key {
                    if e.key != k {
                        return false;
                    }
                }
                if let Some(st) = status {
                    if e.status != st {
                        return false;
                    }
                }
                if let Some(s) = since {
                    if e.ts < s {
                        return false;
                    }
                }
                if let Some(u) = until {
                    if e.ts > u {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn recent(&self, n: usize) -> Vec<AuditEntry> {
        let guard = self.entries.lock().unwrap();
        guard.iter().rev().take(n).cloned().collect()
    }

    pub fn file_path(&self) -> Option<PathBuf> {
        self.file.as_ref().map(|_| PathBuf::from("audit.jsonl"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_and_reads_back() {
        let log = AuditLog::new(10);
        log.log("key1", "POST", "/v1/chat/completions", 200, 100);
        log.log("key2", "POST", "/v1/chat/completions", 429, 5);
        let entries = log.recent(10);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "key2");
        assert_eq!(entries[1].key, "key1");
    }

    #[test]
    fn persists_to_file() {
        let dir = std::env::temp_dir().join("magpie_audit_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("audit.jsonl");
        let _ = std::fs::remove_file(&path);
        let log = AuditLog::with_file(10, path.to_str().unwrap());
        log.log("key1", "POST", "/v1", 200, 100);
        log.log("key2", "POST", "/v1", 429, 5);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("key1"));
        assert!(content.contains("key2"));
        assert!(content.contains("429"));
    }

    #[test]
    fn query_filters() {
        let log = AuditLog::new(10);
        log.log("key1", "POST", "/v1", 200, 100);
        log.log("key2", "POST", "/v1", 429, 5);
        log.log("key1", "POST", "/v1", 200, 200);
        let by_key = log.query(Some("key1"), None, None, None, 10);
        assert_eq!(by_key.len(), 2);
        let by_status = log.query(None, Some(429), None, None, 10);
        assert_eq!(by_status.len(), 1);
        assert_eq!(by_status[0].key, "key2");
        let all = log.query(None, None, None, None, 10);
        assert_eq!(all.len(), 3);
        let limited = log.query(None, None, None, None, 2);
        assert_eq!(limited.len(), 2);
    }

    #[test]
    fn respects_capacity() {
        let log = AuditLog::new(2);
        for i in 0..5 {
            log.log(&format!("key{}", i), "POST", "/v1", 200, 10);
        }
        let entries = log.recent(10);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "key4");
        assert_eq!(entries[1].key, "key3");
    }

    #[test]
    fn writes_to_file() {
        let tmp = std::env::temp_dir().join("magpie_audit_test.jsonl");
        let _ = std::fs::remove_file(&tmp);
        let log = AuditLog::with_file(10, tmp.to_str().unwrap());
        log.log("key1", "POST", "/v1", 200, 100);
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(content.contains("key1"));
        assert!(content.contains("\"status\":200"));
        let _ = std::fs::remove_file(&tmp);
    }
}
