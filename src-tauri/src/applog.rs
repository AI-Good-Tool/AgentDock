//! Runtime log storage: a bounded in-memory ring for the Logs page plus an
//! append-only JSONL file written by one background thread, so callers never
//! wait on disk. Nothing is recorded while logging is switched off.
use agentdock_core::applog::{Entry, Level};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

const RING: usize = 3000;
const FILE_LIMIT: u64 = 5 * 1024 * 1024;

pub struct AppLog {
    ring: Mutex<VecDeque<Entry>>,
    tx: Mutex<Option<mpsc::Sender<WriteCommand>>>,
    path: PathBuf,
}

enum WriteCommand {
    Append(String),
    Clear(mpsc::Sender<Result<(), String>>),
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
    fn writer(&self) -> Option<mpsc::Sender<WriteCommand>> {
        let mut slot = self.tx.lock().unwrap();
        if slot.is_none() {
            let (tx, rx) = mpsc::channel();
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
        // Order disk messages with ring mutations, including clear barriers.
        let mut ring = self.ring.lock().unwrap();
        if let Ok(line) = serde_json::to_string(&entry) {
            if let Some(tx) = self.writer() {
                let _ = tx.send(WriteCommand::Append(line));
            }
        }
        if ring.len() >= RING {
            ring.pop_front();
        }
        ring.push_back(entry);
    }
    /// Called once before provider tasks start. Persisted Trae events already
    /// have durable storage; seed the UI without appending them to app.log again.
    pub fn restore_trae_events(&self, events: &Value) {
        let mut seen = HashSet::new();
        let mut entries: Vec<_> = events.as_array().into_iter().flatten().filter_map(|event| {
            let id = event["id"].as_str()?;
            if !seen.insert(id) { return None; }
            let time = chrono::DateTime::parse_from_rfc3339(event["time"].as_str()?).ok()?.timestamp_millis();
            let kind = event["type"].as_str()?;
            let message = event["message"].as_str()?;
            Some(Entry::new(time, if kind == "error" { Level::Warn } else { Level::Info }, "trae", format!("[{kind}] {message}"), Value::Null))
        }).collect();
        entries.sort_by_key(|e| e.time);
        let mut ring = self.ring.lock().unwrap();
        ring.extend(entries);
        while ring.len() > RING { ring.pop_front(); }
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
    pub fn clear(&self) -> Result<(), String> {
        let mut ring = self.ring.lock().unwrap();
        let (tx, rx) = mpsc::channel();
        self.writer().ok_or("无法启动日志写入线程")?
            .send(WriteCommand::Clear(tx)).map_err(|_| "无法清空日志文件")?;
        rx.recv().map_err(|_| "清空日志任务失败")??;
        ring.clear();
        Ok(())
    }
}

fn write_loop(path: &Path, rx: mpsc::Receiver<WriteCommand>) {
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
    while let Ok(command) = rx.recv() {
        let mut batch = match command {
            WriteCommand::Append(line) => line,
            WriteCommand::Clear(done) => {
                drop(file.take());
                let result = (|| {
                    std::fs::OpenOptions::new().write(true).open(path)
                        .and_then(|f| f.set_len(0)).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })?;
                    std::fs::remove_file(path.with_extension("log.1"))
                        .or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
                })().map_err(|_| "无法清空日志文件".to_owned());
                file = open();
                size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                let _ = done.send(result);
                continue;
            }
        };
        batch.push('\n');
        if size + batch.len() as u64 > FILE_LIMIT {
            // The file may have been truncated by `clear` since the last write.
            size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        }
        if size + batch.len() as u64 > FILE_LIMIT {
            drop(file.take()); // Windows cannot rename an open append handle.
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
        log.clear().unwrap();
        assert_eq!(log.query(&json!({}))["total"], 0);
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn restored_events_are_deduplicated_sorted_and_do_not_rewrite_disk() {
        let dir = std::env::temp_dir().join(format!("agentdock-log-history-{}", uuid::Uuid::new_v4()));
        let log = AppLog::new(&dir);
        let one = json!({"id":"a","time":"2026-10-20T01:00:00Z","type":"refresh","message":"older"});
        let two = json!({"id":"b","time":"2026-10-20T02:00:00Z","type":"error","message":"newer"});
        log.restore_trae_events(&json!([two, one, two, {"id":"bad","time":"invalid"}]));
        let result = log.query(&json!({}));
        assert_eq!(result["total"], 2);
        assert_eq!(result["entries"][0]["message"], "[error] newer");
        assert_ne!(result["entries"][0]["seq"], result["entries"][1]["seq"]);
        assert!(!log.path.exists());
    }
    #[test]
    fn clearing_queued_writes_cannot_resurrect_pre_clear_disk_records() {
        let dir = std::env::temp_dir().join(format!("agentdock-log-clear-{}", uuid::Uuid::new_v4()));
        let log = AppLog::new(&dir);
        for i in 0..100 { log.push(entry(Level::Info, "trae", &format!("old-{i}"))); }
        log.clear().unwrap();
        assert!(std::fs::read(&log.path).unwrap().is_empty());
        assert_eq!(log.query(&json!({}))["total"], 0);
        log.push(entry(Level::Info, "trae", "after-clear"));
        assert_eq!(log.query(&json!({}))["entries"][0]["message"], "after-clear");
        log.clear().unwrap();
        assert!(std::fs::read(&log.path).unwrap().is_empty());
        drop(log);
        let _ = std::fs::remove_dir_all(dir);
    }
}
