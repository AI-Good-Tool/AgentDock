//! Runtime log storage: a bounded in-memory ring for the Logs page plus an
//! append-only JSONL file written by one background thread, so callers never
//! wait on disk. Nothing is recorded while logging is switched off.
use agentdock_core::applog::{Entry, Level};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

const RING: usize = 3000;
const FILE_LIMIT: u64 = 5 * 1024 * 1024;

pub struct AppLog {
    ring: Mutex<VecDeque<Entry>>,
    tx: Mutex<Option<mpsc::Sender<String>>>,
    path: PathBuf,
}

impl AppLog {
    pub fn new(dir: &Path) -> Arc<Self> {
        Arc::new(Self {
            ring: Mutex::new(VecDeque::with_capacity(256)),
            tx: Mutex::new(None),
            path: dir.join("logs/app.log"),
        })
    }
    /// Open the store and install it as the process sink.
    pub fn install(dir: &Path, enabled: bool) -> Arc<Self> {
        let log = Self::new(dir);
        let sink = log.clone();
        agentdock_core::applog::install(move |entry| sink.push(entry));
        agentdock_core::applog::set_enabled(enabled);
        log
    }
    fn writer(&self) -> Option<mpsc::Sender<String>> {
        let mut slot = self.tx.lock().unwrap();
        if slot.is_none() {
            let (tx, rx) = mpsc::channel::<String>();
            let path = self.path.clone();
            std::thread::Builder::new()
                .name("agentdock-log".into())
                .spawn(move || write_loop(&path, rx))
                .ok()?;
            *slot = Some(tx);
        }
        slot.clone()
    }
    fn push(&self, entry: Entry) {
        if let Ok(line) = serde_json::to_string(&entry) {
            if let Some(tx) = self.writer() {
                let _ = tx.send(line);
            }
        }
        let mut ring = self.ring.lock().unwrap();
        if ring.len() >= RING {
            ring.pop_front();
        }
        ring.push_back(entry);
    }
    /// Newest first, filtered by minimum level, source prefix and text.
    pub fn query(&self, filter: &Value) -> Value {
        let min = match filter["level"].as_str() {
            Some("info") => Level::Info,
            Some("warn") => Level::Warn,
            Some("error") => Level::Error,
            _ => Level::Debug,
        };
        let source = filter["source"].as_str().unwrap_or("");
        let group = format!("{source}.");
        let text = filter["query"].as_str().unwrap_or("").to_lowercase();
        let limit = filter["limit"].as_u64().unwrap_or(500).min(RING as u64) as usize;
        let ring = self.ring.lock().unwrap();
        let mut sources: Vec<&str> = ring.iter().map(|e| e.source.as_str()).collect();
        sources.sort_unstable();
        sources.dedup();
        let entries: Vec<&Entry> = ring
            .iter()
            .rev()
            .filter(|e| e.level >= min)
            .filter(|e| source.is_empty() || e.source == source || e.source.starts_with(&group))
            .filter(|e| {
                text.is_empty()
                    || e.message.to_lowercase().contains(&text)
                    || e.fields.to_string().to_lowercase().contains(&text)
            })
            .take(limit)
            .collect();
        json!({
            "entries": entries,
            "total": ring.len(),
            "sources": sources,
            "file": self.path,
        })
    }
    pub fn clear(&self) {
        self.ring.lock().unwrap().clear();
        // Truncate in place: the writer keeps its append handle open.
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&self.path) {
            let _ = f.set_len(0);
        }
        let _ = std::fs::remove_file(self.path.with_extension("log.1"));
    }
}

fn write_loop(path: &Path, rx: mpsc::Receiver<String>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let open = || {
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(path).ok()
    };
    let mut file = open();
    let mut size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    while let Ok(first) = rx.recv() {
        // Drain whatever queued up meanwhile into one write.
        let mut batch = first;
        batch.push('\n');
        while let Ok(more) = rx.try_recv() {
            batch.push_str(&more);
            batch.push('\n');
        }
        if size + batch.len() as u64 > FILE_LIMIT {
            // The file may have been truncated by `clear` since the last write.
            size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        }
        if size + batch.len() as u64 > FILE_LIMIT {
            let _ = std::fs::rename(path, path.with_extension("log.1"));
            file = open();
            size = 0;
        }
        if file.is_none() {
            file = open();
        }
        if let Some(f) = file.as_mut() {
            if f.write_all(batch.as_bytes()).is_ok() {
                size += batch.len() as u64;
            } else {
                file = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(level: Level, source: &str, message: &str) -> Entry {
        Entry { seq: 0, time: 1, level, source: source.into(), message: message.into(), fields: json!({"model": "glm"}) }
    }
    #[test]
    fn ring_is_bounded_and_filters_by_level_source_and_text() {
        let dir = std::env::temp_dir().join(format!("agentdock-applog-{}", uuid::Uuid::new_v4().simple()));
        let log = AppLog::new(&dir);
        for i in 0..RING + 5 {
            log.push(entry(Level::Debug, "gateway", &format!("r{i}")));
        }
        log.push(entry(Level::Warn, "workbuddy.agent", "rejected"));
        log.push(entry(Level::Info, "workbuddy.sdk", "token ok"));
        let all = log.query(&json!({}));
        assert_eq!(all["total"], RING);
        assert_eq!(all["entries"][0]["message"], "token ok");
        let wb = log.query(&json!({"source": "workbuddy"}));
        assert_eq!(wb["entries"].as_array().unwrap().len(), 2);
        let warn = log.query(&json!({"level": "warn"}));
        assert_eq!(warn["entries"].as_array().unwrap().len(), 1);
        let text = log.query(&json!({"query": "GLM", "limit": 3}));
        assert_eq!(text["entries"].as_array().unwrap().len(), 3);
        log.clear();
        assert_eq!(log.query(&json!({}))["total"], 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
