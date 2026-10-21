//! Data directory and one-time migration from the standalone WorkBuddy API,
//! Qoder Switch and Trae Switch apps. Sources are copied, never modified;
//! existing AgentDock data is never overwritten.
use crate::{
    logs::{LogStore, RequestLog},
    settings::{RouteOverride, Settings, Strategy},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("AGENTDOCK_HOME") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agentdock")
}

fn private_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|_| format!("无法创建数据目录 {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn copy_file(from: &Path, to: &Path) -> bool {
    if !from.is_file() || to.exists() {
        return false;
    }
    if let Some(parent) = to.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let ok = fs::copy(from, to).is_ok();
    #[cfg(unix)]
    if ok {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(to, fs::Permissions::from_mode(0o600));
    }
    ok
}
fn copy_dir(from: &Path, to: &Path) -> usize {
    let Ok(entries) = fs::read_dir(from) else {
        return 0;
    };
    let mut n = 0;
    for e in entries.flatten() {
        let path = e.path();
        let target = to.join(e.file_name());
        if path.is_dir() {
            let _ = private_dir(&target);
            n += copy_dir(&path, &target);
        } else if copy_file(&path, &target) {
            n += 1;
        }
    }
    n
}
fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Prepare `dir` and import legacy data once. Returns a human-readable report.
pub fn prepare(dir: &Path) -> Result<Value, String> {
    private_dir(dir)?;
    for sub in ["workbuddy", "qoder", "trae"] {
        private_dir(&dir.join(sub))?;
    }
    let marker = dir.join("migration.json");
    let mut report = read_json(&marker).unwrap_or_else(|| json!({}));
    if std::env::var_os("AGENTDOCK_SKIP_MIGRATION").is_some() {
        return Ok(report);
    }
    let fresh_settings = !dir.join("settings.json").exists();
    let mut settings = crate::app::read_settings(&dir.join("settings.json"));
    let logs = LogStore::open(dir);
    let mut changed = false;
    let home = dirs::home_dir().unwrap_or_default();

    // WorkBuddy API (Tauri data_local_dir/workbuddy-api).
    if report["workbuddy"].is_null() {
        let source = dirs::data_local_dir().unwrap_or_default().join("workbuddy-api");
        let target = dir.join("workbuddy");
        let mut copied = 0;
        for (from, to) in [
            ("store.json", "store.json"),
            ("store.official-usage.json", "store.official-usage.json"),
        ] {
            copied += usize::from(copy_file(&source.join(from), &target.join(to)));
        }
        let mut usage = 0;
        if let Ok(text) = fs::read_to_string(source.join("store.usage.jsonl")) {
            let rows: Vec<Value> = text
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .map(|mut r| {
                    r["provider"] = json!("workbuddy");
                    r["region"] = json!(if r["accountVariant"] == "ai" { "intl" } else { "cn" });
                    r["outcome"] = json!(if r["status"] == 200 { "completed" } else { "failed" });
                    r
                })
                .collect();
            usage = rows.len();
            logs.import_usage(&rows);
        }
        if let Some(store) = read_json(&target.join("store.json")) {
            if fresh_settings {
                settings.strategy = match store["account_priority"].as_str() {
                    Some("roundRobin") => Strategy::RoundRobin,
                    Some("creditFirst") => Strategy::CreditFirst,
                    _ => Strategy::ExpiryFirst,
                };
            }
            // Local aliases / region narrowing become explicit WorkBuddy routes.
            for (id, policy) in store["model_policies"].as_object().into_iter().flatten() {
                let alias = policy["alias"].as_str().unwrap_or("").trim();
                let regions: Vec<&str> = policy["regions"]
                    .as_array()
                    .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_else(|| vec!["cn", "ai"]);
                if alias.is_empty() && policy["regions"].is_null() {
                    continue;
                }
                let targets: Vec<String> = regions
                    .iter()
                    .filter(|r| {
                        store["model_catalogs"][**r]["models"]
                            .as_array()
                            .is_some_and(|m| m.iter().any(|m| m["id"] == *id))
                    })
                    .map(|r| format!("workbuddy:{}:{id}", if *r == "ai" { "intl" } else { "cn" }))
                    .collect();
                if targets.is_empty() {
                    continue;
                }
                settings.routes.entry(format!("workbuddy-legacy:{id}")).or_insert(RouteOverride {
                    alias: if alias.is_empty() { id.clone() } else { alias.into() },
                    targets: Some(targets),
                    disabled: false,
                    custom: true,
                });
            }
        }
        report["workbuddy"] = json!({"at":chrono::Utc::now().to_rfc3339(),"source":source,"files":copied,"usageRows":usage});
        changed = true;
    }

    // Qoder Switch (~/.qoder-switch).
    if report["qoder"].is_null() {
        let source = home.join(".qoder-switch");
        let target = dir.join("qoder");
        let mut copied = 0;
        for name in ["accounts.json", "claim-log.json", "settings.json", "session-links.json"] {
            copied += usize::from(copy_file(&source.join(name), &target.join(name)));
        }
        for name in ["settings.json", "models-cn.json", "models-intl.json"] {
            copied += usize::from(copy_file(
                &source.join("gateway").join(name),
                &target.join("gateway").join(name),
            ));
        }
        copied += copy_dir(&source.join("backups"), &target.join("backups"));
        copied += copy_dir(&source.join("session-backups"), &target.join("session-backups"));
        let mut usage = 0;
        if let Ok(text) = fs::read_to_string(source.join("gateway/usage.jsonl")) {
            let rows: Vec<Value> = text
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .filter_map(|r| {
                    let ts = chrono::DateTime::parse_from_rfc3339(r["time"].as_str()?).ok()?.timestamp_millis();
                    let mut usage = json!({});
                    if let Some(n) = r["in"].as_u64() {
                        usage["prompt_tokens"] = json!(n);
                    }
                    if let Some(n) = r["out"].as_u64() {
                        usage["completion_tokens"] = json!(n);
                    }
                    if let Some(n) = r["cached"].as_u64() {
                        usage["prompt_tokens_details"] = json!({"cached_tokens":n});
                    }
                    Some(json!({"id":r["id"],"timestamp":ts,"model":r["model"],"realModel":r["realModel"],"modelName":r["modelName"],
                        "provider":"qoder","region":r["edition"],"protocol":r["protocol"],"accountName":r["account"],
                        "status":if r["status"]=="completed"{200}else{502},"outcome":r["status"],"durationMs":r["durationMs"],"usage":usage}))
                })
                .collect();
            usage = rows.len();
            logs.import_usage(&rows);
        }
        if let Some(config) = read_json(&source.join("gateway/settings.json")) {
            for (id, route) in config["modelRoutes"].as_object().into_iter().flatten() {
                let targets: Vec<String> = [("cn", &route["cn"]), ("intl", &route["intl"])]
                    .into_iter()
                    .filter_map(|(r, v)| v.as_str().map(|m| format!("qoder:{r}:{m}")))
                    .collect();
                if targets.is_empty() {
                    continue;
                }
                settings.routes.entry(format!("qoder-legacy:{id}")).or_insert(RouteOverride {
                    alias: route["alias"].as_str().unwrap_or(id).into(),
                    targets: Some(targets),
                    disabled: false,
                    custom: true,
                });
            }
            if fresh_settings && report["workbuddy"]["files"] == 0 && config["strategy"] == "expiryFirst" {
                settings.strategy = Strategy::ExpiryFirst;
            }
            if let Some(edition) = config["edition"].as_str().filter(|e| *e != "both") {
                if let Some(scope) = settings.providers.get_mut(&agentdock_core::provider::ProviderKind::Qoder) {
                    scope.regions = vec![edition.to_owned()];
                }
            }
        }
        if let Some(prefs) = read_json(&source.join("settings.json")) {
            if fresh_settings {
                for (key, field) in [("theme", "theme"), ("lang", "lang"), ("timezone", "timezone"), ("tokenFmt", "tokenFormat")] {
                    if let Some(v) = prefs[key].as_str() {
                        match field {
                            "theme" => settings.theme = v.into(),
                            "lang" => settings.lang = v.into(),
                            "timezone" => settings.timezone = v.into(),
                            _ => settings.token_format = v.into(),
                        }
                    }
                }
            }
        }
        report["qoder"] = json!({"at":chrono::Utc::now().to_rfc3339(),"source":source,"files":copied,"usageRows":usage});
        changed = true;
    }

    // Trae Switch (~/.trae-switch, or TRAE_SWITCH_DATA).
    if report["trae"].is_null() {
        let source = std::env::var_os("TRAE_SWITCH_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".trae-switch"));
        let target = dir.join("trae");
        let mut copied = 0;
        // Copy the vault and its key together, or neither.
        if source.join("vault.enc").is_file()
            && source.join("vault.key").is_file()
            && !target.join("vault.enc").exists()
            && !target.join("vault.key").exists()
        {
            copied += usize::from(copy_file(&source.join("vault.key"), &target.join("vault.key")));
            copied += usize::from(copy_file(&source.join("vault.enc"), &target.join("vault.enc")));
        }
        copied += copy_dir(&source.join("backups"), &target.join("backups"));
        let mut imported = 0;
        if copied > 0 {
            if let Ok(store) = trae::store::Store::open(&target) {
                let data = store.snapshot();
                let s = &data["settings"];
                for (real, rule) in s["modelRules"].as_object().into_iter().flatten() {
                    let alias = rule["alias"].as_str().unwrap_or(real);
                    let regions: Vec<&str> = rule["regions"]
                        .as_array()
                        .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
                        .unwrap_or_else(|| vec!["cn", "intl"]);
                    if alias == real && regions.len() == 2 {
                        continue;
                    }
                    settings.routes.entry(format!("trae-legacy:{real}")).or_insert(RouteOverride {
                        alias: alias.into(),
                        targets: Some(regions.iter().map(|r| format!("trae:{r}:{real}")).collect()),
                        disabled: false,
                        custom: true,
                    });
                }
                for (scope, id) in s["gatewayAccountIds"].as_object().into_iter().flatten() {
                    if let Some(id) = id.as_str().filter(|s| !s.is_empty()) {
                        let key = if scope == "all" { "trae".to_owned() } else { format!("trae/{scope}") };
                        settings.pinned.insert(key, format!("trae/{id}"));
                    }
                }
                if fresh_settings && report["workbuddy"]["files"] == 0 && report["qoder"]["files"] == 0 {
                    settings.strategy = match s["gatewayMode"].as_str() {
                        Some("priority") => Strategy::Ordered,
                        Some("random") => Strategy::Random,
                        _ => Strategy::RoundRobin,
                    };
                }
                let rows: Vec<RequestLog> = data["requests"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| {
                        let mut usage = json!({});
                        if let Some(n) = r["inputTokens"].as_u64() {
                            usage["prompt_tokens"] = json!(n);
                        }
                        if let Some(n) = r["outputTokens"].as_u64() {
                            usage["completion_tokens"] = json!(n);
                        }
                        RequestLog {
                            id: r["id"].as_str().unwrap_or("").into(),
                            time: r["time"].as_str().unwrap_or("").into(),
                            protocol: "openai-chat".into(),
                            path: r["path"].as_str().unwrap_or("").into(),
                            requested_model: r["requestedModel"].as_str().or(r["model"].as_str()).unwrap_or("").into(),
                            model: r["model"].as_str().unwrap_or("").into(),
                            real_model: r["realModel"].as_str().unwrap_or("").into(),
                            provider: "trae".into(),
                            region: r["variant"].as_str().unwrap_or("").into(),
                            account_id: r["accountId"].as_str().unwrap_or("").into(),
                            account_name: r["accountName"].as_str().unwrap_or("").into(),
                            status: r["httpStatus"].as_u64().unwrap_or(0) as u16,
                            outcome: match r["status"].as_str() {
                                Some("success") => "completed",
                                Some("cancelled") => "client_disconnected",
                                _ => "failed",
                            }
                            .into(),
                            duration_ms: r["duration"].as_u64().unwrap_or(0) as u128,
                            usage,
                            message: r["error"].as_str().unwrap_or("").into(),
                            upstream_code: r["errorCode"].as_str().map(str::to_owned),
                            stream: r["stream"] == true,
                            details: json!({"protocol":"openai-chat","clientPath":r["path"],"legacy":"trae-switch"}),
                            ..RequestLog::default()
                        }
                    })
                    .filter(|l| !l.id.is_empty())
                    .collect();
                imported = rows.len();
                for log in &rows {
                    logs.record_usage(log);
                }
                logs.import_logs(rows);
            }
        }
        report["trae"] = json!({"at":chrono::Utc::now().to_rfc3339(),"source":source,"files":copied,"requests":imported});
        changed = true;
    }

    if changed {
        crate::app::write_private(&dir.join("settings.json"), &serde_json::to_vec_pretty(&Settings { ..settings }).unwrap())?;
        crate::app::write_private(&marker, &serde_json::to_vec_pretty(&report).unwrap())?;
    }
    Ok(report)
}
