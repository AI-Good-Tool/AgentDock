//! Switch statistics adapters plus a numeric-only local proxy usage ledger.
use crate::{
    diagnostics,
    pool::{Account, RequestLog, Shared},
};
use chrono::{Datelike, TimeZone, Timelike};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    io::Write,
    sync::Mutex,
};
use wb_switch_core::modules as switch;

const RETENTION_MS: i64 = 90 * 86_400_000;
const MAX_USAGE: usize = 20_000;
static USAGE_WRITE: Mutex<()> = Mutex::new(());
static OFFICIAL_REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn observe_credit(rows: &mut Vec<Value>, account: &Account, credit: &Value) {
    let (Some(total), Some(remaining), Some(ts)) = (
        credit["totalCapacity"].as_f64(),
        credit["totalRemaining"].as_f64(),
        credit["updatedAt"].as_i64(),
    ) else {
        return;
    };
    if !total.is_finite() || !remaining.is_finite() {
        return;
    }
    if rows
        .iter()
        .rev()
        .find(|r| r["accountId"] == account.id)
        .is_some_and(|last| {
            last["ts"]
                .as_i64()
                .is_some_and(|old| ts >= old && ts - old < 300_000)
                && last["total"] == json!(total)
                && last["remaining"] == json!(remaining)
        })
    {
        return;
    }
    rows.push(json!({"accountId":account.id,"accountName":account.name,"variant":account.variant,"ts":ts,"total":total,"remaining":remaining}));
    rows.retain(|r| r["ts"].as_i64().is_some_and(|at| at >= ts - RETENTION_MS));
    if rows.len() > 5000 {
        rows.drain(..rows.len() - 5000);
    }
}

fn accounts_and_aliases(state: &Shared) -> (Vec<Value>, BTreeMap<String, String>) {
    let raw = switch::account::load_accounts();
    let store = state.store.lock().unwrap();
    let mut aliases = BTreeMap::new();
    let mut accounts = Vec::new();
    for a in &store.accounts {
        let uid = a.id.split_once(':').map(|(_, uid)| uid).unwrap_or("");
        let original = raw.iter().find(|r| {
            (a.source == "switch" && r["id"] == a.source_id)
                || (r["uid"] == uid && switch::account::variant_of(r).as_str() == a.variant)
        });
        let key = original
            .and_then(|r| r["id"].as_str())
            .unwrap_or(&a.id)
            .to_owned();
        aliases.insert(a.id.clone(), key.clone());
        // Only display metadata is passed to the pure projection, never tokens.
        accounts.push(json!({"id":key,"nickname":a.name,"variant":a.variant}));
    }
    (accounts, aliases)
}

pub fn credit_statistics(state: &Shared) -> Value {
    let (accounts, aliases) = accounts_and_aliases(state);
    let mut snapshots = switch::credit_usage::load_snapshots();
    {
        let store = state.store.lock().unwrap();
        let mut own = store.credit_snapshots.clone();
        // Older versions saved only the latest resource result. Preserve its
        // actual observation time; never pretend it was queried just now.
        for a in &store.accounts {
            if let Some(c) = store
                .resources
                .get(&a.id)
                .filter(|c| c["ok"] == true && c["stale"] != true)
            {
                observe_credit(&mut own, a, c);
            }
        }
        for mut row in own {
            if let Some(id) = row["accountId"].as_str().and_then(|id| aliases.get(id)) {
                row["accountId"] = json!(id);
            }
            snapshots.push(row);
        }
    }
    let now = chrono::Utc::now().timestamp_millis();
    let mut result = switch::credit_usage::project_statistics(
        &snapshots,
        &switch::config::load_checkin_logs(),
        &accounts,
        now,
    );
    // Crucially, do not use upstream get_statistics(false): a cache miss there
    // performs production requests. Reading this page is strictly local.
    let cached = switch::official_usage::read_cached_statistics(
        &state.path.with_extension("official-usage.json"),
    )
    .or_else(|| {
        switch::official_usage::read_cached_statistics(&switch::config::official_usage_cache_file())
    });
    result["officialUsage"] = cached.unwrap_or_else(||json!({"status":"unavailable","summary":{},"daily":[],"accounts":[],"models":[],"requests":[],"errors":[],"detailLimitPerAccount":1000}));
    result["source"] = json!("switch-and-local-observations");
    result["timezone"] = json!("Asia/Shanghai");
    result
}

pub async fn refresh_official(state: &Shared, confirmed: bool) -> Result<Value, String> {
    if !confirmed {
        return Err("查询官方生产用量接口前需明确确认".into());
    }
    let _guard = OFFICIAL_REFRESH.lock().await;
    let (_, aliases) = accounts_and_aliases(state);
    let accounts = state.store.lock().unwrap().accounts.clone();
    let mut request_accounts = Vec::new();
    let mut missing = Vec::new();
    for a in accounts {
        let id = aliases.get(&a.id).cloned().unwrap_or_else(|| a.id.clone());
        match crate::pool::credentials(&a) {
            Ok(c)=>request_accounts.push(json!({"id":id,"nickname":a.name,"variant":a.variant,"uid":c.uid,"domain":c.domain,"access_token":c.token,"enterpriseId":c.enterprise})),
            Err(e)=>missing.push(json!({"accountId":id,"accountName":a.name,"ok":false,"error":e,"usageToday":null,"usage7Days":null,"usageThisMonth":null,"models":[],"daily":[]})),
        }
    }
    let mut usage = switch::official_usage::collect_official_usage_read_only(
        &request_accounts,
        chrono::Utc::now().timestamp_millis(),
    )
    .await;
    let secrets = request_accounts
        .iter()
        .filter_map(|a| a["access_token"].as_str())
        .collect::<Vec<_>>();
    redact_credentials(&mut usage, &secrets);
    if !missing.is_empty() {
        for account in missing {
            usage["errors"].as_array_mut().unwrap().push(json!({"accountId":account["accountId"],"accountName":account["accountName"],"error":account["error"]}));
            usage["accounts"].as_array_mut().unwrap().push(account);
        }
        usage["status"] = json!(if usage["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["ok"] == true)
        {
            "partial"
        } else {
            "unavailable"
        });
    }
    let target = state.path.with_extension("official-usage.json");
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    switch::config::atomic_write(&target, &json!({"payload":usage}).to_string())
        .map_err(|e| e.to_string())?;
    Ok(credit_statistics(state))
}

fn redact_credentials(value: &mut Value, secrets: &[&str]) {
    match value {
        Value::String(text) => {
            for secret in secrets.iter().filter(|s| !s.is_empty()) {
                *text = text.replace(secret, "[REDACTED]");
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_credentials(item, secrets);
            }
        }
        Value::Object(fields) => {
            for field in fields.values_mut() {
                redact_credentials(field, secrets);
            }
        }
        _ => {}
    }
}

pub fn record_usage(state: &Shared, log: &RequestLog) {
    if log.details.usage.as_object().is_none_or(|v| v.is_empty()) {
        return;
    }
    let _guard = USAGE_WRITE.lock().unwrap();
    let target = state.path.with_extension("usage.jsonl");
    if let Some(parent) = target.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if std::fs::metadata(&target).is_ok_and(|m| m.len() > 8 * 1024 * 1024) {
        let rows = read_usage(state);
        let text = rows
            .iter()
            .rev()
            .take(MAX_USAGE - 1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|r| format!("{r}\n"))
            .collect::<String>();
        if switch::config::atomic_write(&target, &text).is_err() {
            return;
        }
    }
    let at = chrono::DateTime::parse_from_rfc3339(&log.time)
        .map(|d| d.timestamp_millis())
        .unwrap_or(0);
    let row = json!({"id":log.id,"timestamp":at,"model":diagnostics::bounded(&log.model,128),"realModel":diagnostics::bounded(&log.real_model,128),"accountVariant":log.account_variant,"accountId":log.account_id,"accountName":diagnostics::bounded(&log.account_name,128),"status":log.status,"usage":diagnostics::usage(&log.details.usage)});
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(target) {
        let _ = writeln!(file, "{row}");
    }
}

fn read_usage(state: &Shared) -> Vec<Value> {
    let text =
        std::fs::read_to_string(state.path.with_extension("usage.jsonl")).unwrap_or_default();
    let cutoff = chrono::Utc::now().timestamp_millis() - RETENTION_MS;
    let mut ids = HashSet::new();
    let mut rows = text
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|r| {
            r["timestamp"].as_i64().is_some_and(|ts| ts >= cutoff)
                && r["id"].as_str().is_some_and(|id| ids.insert(id.to_owned()))
        })
        .take(MAX_USAGE)
        .collect::<Vec<_>>();
    rows.reverse();
    rows
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
    for (key, value) in [
        ("input", input),
        ("output", output),
        ("cacheRead", read),
        ("cacheWrite", write),
        ("uncachedInput", input.saturating_sub(read)),
        ("total", input.saturating_add(output).saturating_add(write)),
        ("records", 1),
    ] {
        group[key] = json!(group[key].as_u64().unwrap_or(0).saturating_add(value));
    }
    group["cacheHitRate"] = if group["input"].as_u64().unwrap_or(0) > 0 {
        json!(group["cacheRead"].as_f64().unwrap_or(0.0) / group["input"].as_f64().unwrap())
    } else {
        Value::Null
    };
}

pub fn proxy_statistics(rows: &[Value]) -> Value {
    let mut summary = json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"uncachedInput":0,"total":0,"records":0,"cacheHitRate":null});
    let mut models = BTreeMap::new();
    let mut accounts = BTreeMap::new();
    let mut daily = BTreeMap::new();
    let mut hours = BTreeMap::new();
    let mut daily_models = BTreeMap::<String, BTreeMap<String, Value>>::new();
    let mut requests = Vec::new();
    for r in rows {
        let Some(at) = r["timestamp"].as_i64().and_then(|at| {
            switch::config::stats_timezone()
                .timestamp_millis_opt(at)
                .single()
        }) else {
            continue;
        };
        let day = at.format("%Y-%m-%d").to_string();
        let model = r["model"].as_str().unwrap_or("unknown");
        let account = r["accountName"].as_str().unwrap_or("unknown");
        let hour = format!("{}-{}", at.weekday().num_days_from_monday(), at.hour());
        add(&mut summary, r);
        for (groups, key) in [
            (&mut models, model.to_owned()),
            (&mut accounts, account.to_owned()),
            (&mut daily, day.clone()),
            (&mut hours, hour),
        ] {
            add(
                groups
                    .entry(key.clone())
                    .or_insert_with(|| json!({"key":key})),
                r,
            );
        }
        add(
            daily_models
                .entry(model.to_owned())
                .or_default()
                .entry(day.clone())
                .or_insert_with(|| json!({"key":day})),
            r,
        );
        let mut detail = json!({"timestamp":r["timestamp"],"model":model,"project":account,"sessionId":r["id"],"accountId":r["accountId"],"thinking":r["usage"]["completion_tokens_details"]["reasoning_tokens"].as_u64().unwrap_or(0),"credit":r["usage"]["credit"]});
        add(&mut detail, r);
        requests.push(detail);
    }
    let sort = |groups: BTreeMap<String, Value>| {
        let mut rows = groups.into_values().collect::<Vec<_>>();
        rows.sort_by_key(|r| std::cmp::Reverse(r["total"].as_u64().unwrap_or(0)));
        rows
    };
    requests.reverse();
    requests.truncate(500);
    json!({"source":"proxy","summary":summary,"models":sort(models),"projects":sort(accounts),"sessions":[],"daily":daily.into_values().collect::<Vec<_>>(),"dailyByModel":daily_models.into_iter().map(|(k,v)|(k,v.into_values().collect::<Vec<_>>())).collect::<BTreeMap<_,_>>(),"hours":hours.into_values().collect::<Vec<_>>(),"requests":requests,"filesScanned":1,"parseErrors":0,"coverageStartAt":rows.first().map(|r|&r["timestamp"]),"coverageEndAt":rows.last().map(|r|&r["timestamp"])})
}

pub fn token_statistics(state: &Shared) -> Value {
    let mut result = switch::token_stats::get_statistics(None);
    result["sources"]
        .as_array_mut()
        .unwrap()
        .push(proxy_source(state));
    result["timezone"] = json!("Asia/Shanghai");
    result
}

/// Lightweight local-only refresh; does not scan client transcripts or query an API.
pub fn proxy_source(state: &Shared) -> Value {
    let mut proxy = proxy_statistics(&read_usage(state));
    proxy["filesScanned"] = json!(usize::from(
        state.path.with_extension("usage.jsonl").is_file()
    ));
    proxy
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_error_projection_cannot_echo_a_known_credential() {
        let mut data = json!({"errors":[{"error":"bad token-secret"}],"accounts":[{"error":"token-secret"}],"summary":{"usageToday":12}});
        redact_credentials(&mut data, &["token-secret"]);
        assert!(!data.to_string().contains("token-secret"));
        assert_eq!(data["summary"]["usageToday"], 12);
    }
    #[test]
    fn proxy_totals_do_not_double_count_cached_input_and_group_in_shanghai() {
        let ts = chrono::DateTime::parse_from_rfc3339("2026-10-18T17:00:00Z")
            .unwrap()
            .timestamp_millis();
        let stats = proxy_statistics(&[
            json!({"id":"test","timestamp":ts,"model":"demo","accountName":"a","usage":{"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":60},"cache_creation_input_tokens":5}}),
        ]);
        assert_eq!(stats["summary"]["total"], 125);
        assert_eq!(stats["summary"]["uncachedInput"], 40);
        assert_eq!(stats["daily"][0]["key"], "2026-10-19");
    }
    #[tokio::test]
    async fn official_refresh_without_confirmation_never_reads_credentials_or_network() {
        let state = crate::pool::AppState::new(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()))
        .unwrap();
        assert!(refresh_official(&state, false)
            .await
            .unwrap_err()
            .contains("确认"));
    }

    #[test]
    fn usage_ledger_is_persistent_numeric_only_and_deduplicated() {
        let state = crate::pool::AppState::new(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()))
        .unwrap();
        let log = RequestLog {
            id: "request-1".into(),
            time: chrono::Utc::now().to_rfc3339(),
            model: "demo".into(),
            real_model: "demo-real".into(),
            account_id: "account-a".into(),
            account_name: "A".into(),
            account_variant: "cn".into(),
            status: 200,
            duration_ms: 1,
            outcome: "completed".into(),
            details: crate::diagnostics::RequestDetails {
                usage: json!({"prompt_tokens":100,"completion_tokens":20,"token":"PRIVATE_TOKEN"}),
                request_summary: json!({"content":"PRIVATE_PROMPT"}),
                ..Default::default()
            },
        };
        record_usage(&state, &log);
        record_usage(&state, &log);
        let text = std::fs::read_to_string(state.path.with_extension("usage.jsonl")).unwrap();
        assert!(!text.contains("PRIVATE_"));
        let rows = read_usage(&state);
        assert_eq!(rows.len(), 1);
        assert_eq!(proxy_statistics(&rows)["summary"]["total"], 120);
    }
    #[test]
    fn completed_lease_usage_is_available_in_requests_ledger_and_lightweight_statistics() {
        let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let state = crate::pool::AppState::new(path.clone()).unwrap();
        state.store.lock().unwrap().accounts.push(Account {
            id: "fixture-account".into(),
            name: "Test account".into(),
            source: "fixture".into(),
            source_id: "fixture".into(),
            variant: "cn".into(),
            enabled: true,
            checkin_enabled: false,
            note: String::new(),
            display_field: String::new(),
            expires_at: None,
        });
        let (_, mut lease) = state
            .acquire("deepseek-v4.1-flash", Some("fixture-account"))
            .unwrap();
        let usage = json!({"prompt_tokens":142058,"completion_tokens":988,"total_tokens":143046,"prompt_tokens_details":{"cached_tokens":11008},"completion_tokens_details":{"reasoning_tokens":897}});
        let mut observer = crate::protocol::SseErrors::default();
        let wire = format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[],"usage":usage})
        );
        for bytes in wire.as_bytes().chunks(7) {
            observer.feed(bytes);
        }
        lease.details.usage = observer.usage;
        lease.status = 200;
        lease.outcome = "completed".into();
        drop(lease);
        let log = serde_json::to_value(&state.runtime.lock().unwrap().logs[0]).unwrap();
        let stats = proxy_source(&state);
        assert_eq!(stats["requests"].as_array().unwrap().len(), 1);
        assert_eq!(stats["summary"]["total"], 143046);
        assert_eq!(
            stats["requests"][0]["input"],
            log["details"]["usage"]["prompt_tokens"]
        );
        assert_eq!(
            stats["requests"][0]["output"],
            log["details"]["usage"]["completion_tokens"]
        );
        assert_eq!(stats["requests"][0]["thinking"], 897);
        assert_eq!(stats["requests"][0]["cacheRead"], 11008);
        let restarted = crate::pool::AppState::new(path).unwrap();
        assert_eq!(proxy_source(&restarted)["summary"]["total"], 143046);
    }
}
