//! Unified request log (bounded, persisted metadata only) and the numeric
//! usage ledger. Prompts, completions, tool arguments and credentials are
//! never written: request details pass through the allowlisted diagnostics.
use agentdock_core::diagnostics;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub const MAX_LOGS: usize = 1000;
const USAGE_RETENTION_MS: i64 = 400 * 86_400_000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RequestLog {
    pub id: String,
    pub time: String,
    pub protocol: String,
    pub path: String,
    /// Model name the client sent.
    pub requested_model: String,
    /// Public alias at request time (never rewritten afterwards).
    pub model: String,
    pub route_id: String,
    pub real_model: String,
    pub model_name: String,
    pub provider: String,
    pub region: String,
    pub account_id: String,
    pub account_name: String,
    /// HTTP status returned to the client (499 = client gone).
    pub status: u16,
    pub outcome: String,
    pub duration_ms: u128,
    pub first_byte_ms: Option<u128>,
    pub usage: Value,
    pub message: String,
    pub upstream_code: Option<String>,
    pub strategy: String,
    pub stream: bool,
    /// 1-based account attempt within one client request (0 = legacy log).
    pub attempt: u32,
    /// Id of the first attempt; shared by every attempt of one request.
    pub request_group: String,
    /// Local API key that made the request: id, abbreviated value and the
    /// alias at request time (empty for internal and legacy records).
    pub key_id: String,
    pub key_hint: String,
    pub key_alias: String,
    pub details: Value,
}
impl RequestLog {
    pub fn success(&self) -> bool {
        self.outcome == "completed"
    }
    pub fn with_key(mut self, key: &crate::api_key::ApiKey) -> Self {
        self.key_id = key.id.clone();
        self.key_hint = key.hint();
        self.key_alias = key.alias.clone();
        self
    }
    fn key_label(&self) -> &str {
        if self.key_alias.is_empty() { &self.key_hint } else { &self.key_alias }
    }
}

pub struct LogStore {
    dir: PathBuf,
    logs: Mutex<VecDeque<RequestLog>>,
    io: Mutex<()>,
}

fn append_private(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")
}
fn rewrite_private(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&tmp)?.write_all(text.as_bytes())?;
    fs::rename(tmp, path)
}

impl LogStore {
    pub fn open(dir: &Path) -> Self {
        let mut logs = VecDeque::new();
        if let Ok(text) = fs::read_to_string(dir.join("requests.jsonl")) {
            let mut seen = HashSet::new();
            for line in text.lines().rev() {
                if let Ok(mut log) = serde_json::from_str::<RequestLog>(line) {
                    if !seen.insert(log.id.clone()) {
                        continue;
                    }
                    if log.outcome == "running" {
                        log.outcome = "cancelled".into();
                        log.message = "应用退出时请求尚未完成".into();
                    }
                    logs.push_back(log);
                    if logs.len() >= MAX_LOGS {
                        break;
                    }
                }
            }
        }
        Self {
            dir: dir.to_owned(),
            logs: Mutex::new(logs),
            io: Mutex::new(()),
        }
    }
    pub fn list(&self) -> Vec<RequestLog> {
        self.logs.lock().unwrap().iter().cloned().collect()
    }
    pub fn clear(&self) -> Result<(), String> {
        let _io = self.io.lock().unwrap();
        self.logs.lock().unwrap().clear();
        let path = self.dir.join("requests.jsonl");
        if path.exists() {
            fs::remove_file(path).map_err(|_| "无法清空请求记录")?;
        }
        Ok(())
    }
    /// Newest first in memory; persisted as an append-only, periodically compacted file.
    pub fn push(&self, log: RequestLog) {
        agentdock_core::applog::emit(
            if log.status >= 500 {
                agentdock_core::applog::Level::Error
            } else if log.status >= 400 {
                agentdock_core::applog::Level::Warn
            } else {
                agentdock_core::applog::Level::Info
            },
            if log.protocol == "internal" { "internal" } else { "gateway" },
            || {
                (
                    format!(
                        "{} {} → {} · {} ms{}",
                        if log.model.is_empty() { &log.requested_model } else { &log.model },
                        log.provider,
                        log.status,
                        log.duration_ms,
                        if log.key_id.is_empty() { String::new() } else { format!(" · Key {}", log.key_label()) },
                    ),
                    serde_json::json!({
                        "requestId": log.id,
                        "apiKey": log.key_label(),
                        "account": log.account_name,
                        "region": log.region,
                        "realModel": log.real_model,
                        "outcome": log.outcome,
                        "message": log.message,
                    }),
                )
            },
        );
        {
            let mut logs = self.logs.lock().unwrap();
            logs.retain(|l| l.id != log.id);
            logs.push_front(log.clone());
            logs.truncate(MAX_LOGS);
        }
        let _io = self.io.lock().unwrap();
        let path = self.dir.join("requests.jsonl");
        if let Ok(line) = serde_json::to_string(&log) {
            let _ = append_private(&path, &line);
        }
        if fs::metadata(&path).is_ok_and(|m| m.len() > 6 * 1024 * 1024) {
            let logs = self.logs.lock().unwrap().clone();
            let text = logs
                .iter()
                .rev()
                .filter_map(|l| serde_json::to_string(l).ok())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            let _ = rewrite_private(&path, &text);
        }
        if log.usage.as_object().is_some_and(|u| !u.is_empty()) || log.protocol != "internal" {
            self.record_usage(&log);
        }
    }
    fn usage_path(&self) -> PathBuf {
        self.dir.join("usage.jsonl")
    }
    pub fn record_usage(&self, log: &RequestLog) {
        if log.protocol == "internal" {
            return;
        }
        let at = chrono::DateTime::parse_from_rfc3339(&log.time)
            .map(|d| d.timestamp_millis())
            .unwrap_or(0);
        let row = json!({"id":log.id,"timestamp":at,"model":diagnostics::bounded(&log.model,128),
            "realModel":diagnostics::bounded(&log.real_model,128),"modelName":diagnostics::bounded(&log.model_name,128),
            "provider":log.provider,"region":log.region,"protocol":log.protocol,
            "accountId":log.account_id,"accountName":diagnostics::bounded(&log.account_name,128),
            "keyId":log.key_id,"keyHint":log.key_hint,"keyAlias":log.key_alias,
            "status":log.status,"outcome":log.outcome,"durationMs":log.duration_ms,
            "usage":diagnostics::usage(&log.usage)});
        let _ = append_private(&self.usage_path(), &row.to_string());
    }
    /// Import historical rows (migration); duplicate ids are ignored on read.
    pub fn import_usage(&self, rows: &[Value]) {
        let _io = self.io.lock().unwrap();
        for row in rows {
            let _ = append_private(&self.usage_path(), &row.to_string());
        }
    }
    pub fn import_logs(&self, mut rows: Vec<RequestLog>) {
        rows.sort_by(|a, b| a.time.cmp(&b.time));
        for log in rows {
            let mut logs = self.logs.lock().unwrap();
            if logs.iter().any(|l| l.id == log.id) {
                continue;
            }
            logs.push_front(log.clone());
            logs.truncate(MAX_LOGS);
            drop(logs);
            if let Ok(line) = serde_json::to_string(&log) {
                let _ = append_private(&self.dir.join("requests.jsonl"), &line);
            }
        }
    }
    pub fn usage_rows(&self) -> Vec<Value> {
        let _io = self.io.lock().unwrap();
        let text = fs::read_to_string(self.usage_path()).unwrap_or_default();
        let cutoff = chrono::Utc::now().timestamp_millis() - USAGE_RETENTION_MS;
        let mut ids = HashSet::new();
        let mut rows: Vec<Value> = text
            .lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|r| {
                r["timestamp"].as_i64().is_some_and(|ts| ts >= cutoff)
                    && r["id"].as_str().is_some_and(|id| ids.insert(id.to_owned()))
            })
            .take(100_000)
            .collect();
        rows.reverse();
        rows
    }
}

fn add(group: &mut Value, row: &Value) {
    let u = &row["usage"];
    let input = u["prompt_tokens"].as_u64().unwrap_or(0);
    let output = u["completion_tokens"].as_u64().unwrap_or(0);
    let read = u["prompt_tokens_details"]["cached_tokens"]
        .as_u64()
        .or_else(|| u["cache_read_input_tokens"].as_u64())
        .unwrap_or(0)
        .min(input);
    let write = u["cache_creation_input_tokens"]
        .as_u64()
        .or_else(|| u["prompt_cache_write_tokens"].as_u64())
        .unwrap_or(0);
    let thinking = u["completion_tokens_details"]["reasoning_tokens"]
        .as_u64()
        .unwrap_or(0);
    let known = u["prompt_tokens"].is_number() || u["completion_tokens"].is_number();
    for (key, value) in [
        ("input", input),
        ("output", output),
        ("thinking", thinking),
        ("cacheRead", read),
        ("cacheWrite", write),
        ("uncachedInput", input.saturating_sub(read)),
        // Cached input is already part of prompt tokens: never add it twice.
        ("total", input.saturating_add(output).saturating_add(write)),
        ("records", 1),
        ("success", u64::from(row["outcome"] == "completed" || row["status"] == 200)),
        ("withUsage", u64::from(known)),
    ] {
        group[key] = json!(group[key].as_u64().unwrap_or(0).saturating_add(value));
    }
    group["cacheHitRate"] = if group["input"].as_u64().unwrap_or(0) > 0 {
        json!(group["cacheRead"].as_f64().unwrap_or(0.0) / group["input"].as_f64().unwrap())
    } else {
        Value::Null
    };
}

/// Aggregate proxy usage with optional provider/region/model/account filters,
/// grouped by day in the requested UTC offset (minutes).
pub fn statistics(rows: &[Value], filter: &Value, offset_minutes: i64) -> Value {
    let offset = chrono::FixedOffset::east_opt((offset_minutes.clamp(-1440, 1440) * 60) as i32)
        .unwrap_or(chrono::FixedOffset::east_opt(8 * 3600).unwrap());
    let pass = |row: &Value, key: &str| {
        filter[key]
            .as_str()
            .filter(|f| !f.is_empty() && *f != "all")
            .is_none_or(|f| row[key] == f)
    };
    let from = filter["from"].as_i64();
    let to = filter["to"].as_i64();
    let mut summary = json!({});
    let mut groups: [BTreeMap<String, Value>; 5] = Default::default();
    let mut daily = BTreeMap::<String, Value>::new();
    let mut hours = BTreeMap::<String, Value>::new();
    let mut api_keys = BTreeMap::<String, Value>::new();
    let mut requests = vec![];
    for r in rows.iter().filter(|r| {
        ["provider", "region", "model", "accountId", "keyId"].iter().all(|k| pass(r, k))
            && from.is_none_or(|f| r["timestamp"].as_i64().unwrap_or(0) >= f)
            && to.is_none_or(|t| r["timestamp"].as_i64().unwrap_or(0) < t)
    }) {
        let Some(at) = r["timestamp"]
            .as_i64()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|t| t.with_timezone(&offset))
        else {
            continue;
        };
        let day = at.format("%Y-%m-%d").to_string();
        use chrono::{Datelike, Timelike};
        let hour = format!("{}-{}", at.weekday().num_days_from_monday(), at.hour());
        add(&mut summary, r);
        let keys = [
            r["model"].as_str().unwrap_or("unknown").to_owned(),
            r["accountName"].as_str().unwrap_or("unknown").to_owned(),
            r["provider"].as_str().unwrap_or("unknown").to_owned(),
            format!("{}:{}", r["provider"].as_str().unwrap_or(""), r["region"].as_str().unwrap_or("")),
            r["realModel"].as_str().unwrap_or("unknown").to_owned(),
        ];
        for (g, key) in groups.iter_mut().zip(keys) {
            add(g.entry(key.clone()).or_insert_with(|| json!({"key":key})), r);
        }
        if let Some(id) = r["keyId"].as_str().filter(|id| !id.is_empty()) {
            // Rows are chronological: the newest alias and hint win.
            let label = r["keyAlias"].as_str().filter(|a| !a.is_empty()).or(r["keyHint"].as_str()).unwrap_or(id);
            let entry = api_keys.entry(id.to_owned()).or_insert_with(|| json!({"key":id}));
            add(entry, r);
            entry["keyHint"] = r["keyHint"].clone();
            entry["keyAlias"] = r["keyAlias"].clone();
            entry["label"] = json!(label);
        }
        add(daily.entry(day.clone()).or_insert_with(|| json!({"key":day})), r);
        add(hours.entry(hour.clone()).or_insert_with(|| json!({"key":hour})), r);
        let mut detail = json!({"id":r["id"],"timestamp":r["timestamp"],"time":at.to_rfc3339(),"model":r["model"],"realModel":r["realModel"],
            "provider":r["provider"],"region":r["region"],"protocol":r["protocol"],"accountName":r["accountName"],"accountId":r["accountId"],
            "keyId":r["keyId"],"keyHint":r["keyHint"],"keyAlias":r["keyAlias"],
            "status":r["status"],"outcome":r["outcome"],"durationMs":r["durationMs"],"usageKnown":r["usage"]["prompt_tokens"].is_number()});
        add(&mut detail, r);
        requests.push(detail);
    }
    let sort = |g: BTreeMap<String, Value>| {
        let mut v: Vec<Value> = g.into_values().collect();
        v.sort_by_key(|r| std::cmp::Reverse(r["total"].as_u64().unwrap_or(0)));
        v
    };
    let [models, accounts, providers, regions, real] = groups;
    requests.reverse();
    let total = requests.len();
    requests.truncate(2000);
    json!({"summary":summary,"daily":daily.into_values().collect::<Vec<_>>(),"hours":hours.into_values().collect::<Vec<_>>(),
        "models":sort(models),"accounts":sort(accounts),"providers":sort(providers),"regions":sort(regions),"realModels":sort(real),
        "apiKeys":sort(api_keys),
        "requests":requests,"requestCount":total,"offsetMinutes":offset_minutes})
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentdock_core::diagnostics::RequestDetails;
    fn log(id: &str, provider: &str, usage: Value) -> RequestLog {
        RequestLog {
            id: id.into(),
            time: "2026-10-20T17:00:00Z".into(),
            protocol: "anthropic-messages".into(),
            model: "alias".into(),
            real_model: "real".into(),
            provider: provider.into(),
            region: "cn".into(),
            account_name: "A".into(),
            status: 200,
            outcome: "completed".into(),
            usage,
            details: serde_json::to_value(RequestDetails::default()).unwrap(),
            ..RequestLog::default()
        }
    }
    #[test]
    fn logs_persist_dedupe_and_usage_never_double_counts_cache() {
        let dir = std::env::temp_dir().join(format!("agentdock-logs-{}", uuid::Uuid::new_v4()));
        let store = LogStore::open(&dir);
        store.push(log("1", "workbuddy", json!({"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":60},"cache_creation_input_tokens":5,"text":"PRIVATE"})));
        store.push(log("2", "qoder", json!({})));
        let mut running = log("3", "trae", Value::Null);
        running.outcome = "running".into();
        store.push(running);
        let reopened = LogStore::open(&dir);
        assert_eq!(reopened.list().len(), 3);
        assert_eq!(reopened.list()[0].outcome, "cancelled");
        let rows = reopened.usage_rows();
        assert!(!serde_json::to_string(&rows).unwrap().contains("PRIVATE"));
        let stats = statistics(&rows, &json!({}), 480);
        assert_eq!(stats["summary"]["total"], 125);
        assert_eq!(stats["summary"]["uncachedInput"], 40);
        assert_eq!(stats["summary"]["records"], 3);
        assert_eq!(stats["summary"]["withUsage"], 1);
        assert_eq!(stats["daily"][0]["key"], "2026-10-21");
        let wb = statistics(&rows, &json!({"provider":"workbuddy"}), 0);
        assert_eq!(wb["summary"]["records"], 1);
        assert_eq!(wb["daily"][0]["key"], "2026-10-20");
        assert_eq!(statistics(&rows, &json!({"provider":"qoder"}), 0)["summary"]["total"], 0);
        reopened.clear().unwrap();
        assert!(LogStore::open(&dir).list().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}
