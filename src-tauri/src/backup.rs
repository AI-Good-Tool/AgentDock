//! One-file backup of every platform's accounts and settings.
//!
//! The bundle holds live login tokens, so it is written owner-only and the UI
//! warns before export. Import is additive: accounts are upserted by their
//! platform identity and nothing absent from the bundle is removed.
use crate::app::{write_private, Shared};
use crate::settings::Settings;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "agentdock-backup";

/// What to include / restore.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Parts {
    pub providers: Vec<String>,
    pub accounts: bool,
    pub settings: bool,
}
impl Default for Parts {
    fn default() -> Self {
        Self {
            providers: vec!["workbuddy".into(), "qoder".into(), "trae".into()],
            accounts: true,
            settings: true,
        }
    }
}
impl Parts {
    fn has(&self, provider: &str) -> bool {
        self.providers.iter().any(|p| p == provider)
    }
}

pub fn expand(path: &str) -> PathBuf {
    match path.trim().strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(path.trim()),
    }
}

fn workbuddy_section(hub: &Shared, parts: &Parts) -> Value {
    let store = hub.wb.store.lock().unwrap().clone();
    let mut out = json!({});
    if parts.accounts {
        // Credentials for Switch-library accounts live in ~/.wb-switch; carry those records.
        let wanted: Vec<&str> = store
            .accounts
            .iter()
            .filter(|a| a.source == "switch")
            .map(|a| a.source_id.as_str())
            .collect();
        let records: Vec<Value> = workbuddy::pool::switch_file()
            .ok()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r["id"].as_str().is_some_and(|id| wanted.contains(&id)))
            .collect();
        out["accounts"] = json!(store.accounts);
        out["switchRecords"] = json!(records);
    }
    if parts.settings {
        out["settings"] = json!(store.settings);
        out["accountPriority"] = json!(store.account_priority);
        out["modelPolicies"] = json!(store.model_policies);
        out["checkinConfig"] = json!(store.checkin_config);
    }
    out
}

fn qoder_section(parts: &Parts) -> Value {
    use qoder::qcore as qc;
    let mut out = json!({});
    if parts.accounts {
        out["accounts"] = json!(qc::load_accounts());
    }
    if parts.settings {
        out["settings"] = qc::load_settings();
    }
    out
}

fn trae_section(hub: &Shared, parts: &Parts) -> Value {
    let mut out = json!({});
    if parts.accounts {
        out["accounts"] = json!(hub.trae.store.accounts());
    }
    if parts.settings {
        out["settings"] = hub.trae.store.snapshot()["settings"].clone();
    }
    out
}

pub fn build(hub: &Shared, parts: &Parts) -> Value {
    let mut bundle = json!({
        "format": FORMAT,
        "version": 1,
        "app": env!("CARGO_PKG_VERSION"),
        "exportedAt": chrono::Utc::now().to_rfc3339(),
    });
    if parts.settings {
        bundle["agentdock"] = json!({"settings": hub.settings()});
    }
    if parts.has("workbuddy") {
        bundle["workbuddy"] = workbuddy_section(hub, parts);
    }
    if parts.has("qoder") {
        bundle["qoder"] = qoder_section(parts);
    }
    if parts.has("trae") {
        bundle["trae"] = trae_section(hub, parts);
    }
    bundle
}

/// Counts per platform, for the export result and the import preview.
pub fn summary(bundle: &Value) -> Value {
    let count = |p: &str| bundle[p]["accounts"].as_array().map(Vec::len);
    let settings = |p: &str| bundle[p]["settings"].is_object();
    json!({
        "exportedAt": bundle["exportedAt"],
        "app": bundle["app"],
        "agentdockSettings": bundle["agentdock"]["settings"].is_object(),
        "workbuddy": {"present": bundle["workbuddy"].is_object(), "accounts": count("workbuddy"), "settings": settings("workbuddy")},
        "qoder": {"present": bundle["qoder"].is_object(), "accounts": count("qoder"), "settings": settings("qoder")},
        "trae": {"present": bundle["trae"].is_object(), "accounts": count("trae"), "settings": settings("trae")},
    })
}

pub fn export(hub: &Shared, parts: &Parts) -> Result<Value, String> {
    if parts.providers.is_empty() && !parts.settings {
        return Err("请至少选择一项导出内容".into());
    }
    let bundle = build(hub, parts);
    let dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .ok_or("找不到下载目录")?;
    let name = format!(
        "agentdock-backup-{}.json",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    );
    let path = dir.join(name);
    write_private(&path, &serde_json::to_vec_pretty(&bundle).unwrap())?;
    agentdock_core::applog::info("backup", || {
        (format!("已导出备份到 {}", path.display()), summary(&bundle))
    });
    Ok(json!({"path": path, "summary": summary(&bundle)}))
}

pub fn read(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|_| format!("无法读取文件：{}", path.display()))?;
    let bundle: Value = serde_json::from_slice(&bytes).map_err(|_| "文件不是有效的 JSON")?;
    if bundle["format"] == FORMAT {
        return Ok(bundle);
    }
    // A Qoder Switch account backup restores as a Qoder-only bundle.
    if bundle["format"] == "qoder-switch-accounts" || bundle.is_array() {
        let accounts = if bundle.is_array() { bundle } else { bundle["accounts"].clone() };
        return Ok(json!({"format": FORMAT, "version": 1, "exportedAt": null, "qoder": {"accounts": accounts}}));
    }
    Err("不是 AgentDock 备份文件".into())
}

pub fn import(hub: &Shared, bundle: &Value, parts: &Parts) -> Result<Value, String> {
    let mut report = json!({});
    let saved = &bundle["agentdock"]["settings"];
    if parts.settings && saved.is_object() {
        let mut imported: Settings =
            serde_json::from_value(saved.clone()).map_err(|_| "AgentDock 设置格式无效")?;
        imported.upgrade();
        let running = hub.running();
        hub.update_settings(|s| {
            let port = s.port;
            *s = imported;
            if running {
                s.port = port;
            }
            Ok(())
        })?;
        report["agentdock"] = json!({"settings": true});
    }
    if parts.has("workbuddy") && bundle["workbuddy"].is_object() {
        report["workbuddy"] = import_workbuddy(hub, &bundle["workbuddy"], parts)?;
    }
    if parts.has("qoder") && bundle["qoder"].is_object() {
        use qoder::qcore as qc;
        let q = &bundle["qoder"];
        let mut r = json!({});
        if parts.accounts && q["accounts"].is_array() {
            r["accounts"] = json!(qc::import_accounts_value(q["accounts"].clone())?);
        }
        if parts.settings && q["settings"].is_object() {
            let mut current = qc::load_settings();
            for (k, v) in q["settings"].as_object().unwrap() {
                current[k] = v.clone();
            }
            qc::save_settings(&current);
            r["settings"] = json!(true);
        }
        report["qoder"] = r;
    }
    if parts.has("trae") && bundle["trae"].is_object() {
        let t = &bundle["trae"];
        let mut r = json!({});
        if parts.accounts {
            let (mut ok, mut failed) = (0, 0);
            for account in t["accounts"].as_array().into_iter().flatten() {
                match hub.trae.store.upsert(account.clone()) {
                    Ok(_) => ok += 1,
                    Err(_) => failed += 1,
                }
            }
            r["accounts"] = json!(ok);
            r["failed"] = json!(failed);
        }
        if parts.settings && t["settings"].is_object() {
            let settings = t["settings"].clone();
            hub.trae.store.update(|data| {
                data["settings"] = settings;
                Ok(())
            })?;
            r["settings"] = json!(true);
        }
        report["trae"] = r;
    }
    agentdock_core::applog::info("backup", || ("已导入备份".into(), report.clone()));
    Ok(report)
}

fn import_workbuddy(hub: &Shared, section: &Value, parts: &Parts) -> Result<Value, String> {
    let mut r = json!({});
    if parts.accounts && section["accounts"].is_array() {
        let records = section["switchRecords"].as_array().cloned().unwrap_or_default();
        if !records.is_empty() {
            merge_switch_records(&hub.dir, &records)?;
        }
        let accounts: Vec<workbuddy::pool::Account> =
            serde_json::from_value(section["accounts"].clone()).map_err(|_| "WorkBuddy 账号数据格式无效")?;
        let n = accounts.len();
        let local = accounts.iter().filter(|a| a.source != "switch").count();
        hub.wb.update_store(|store| {
            for account in accounts {
                match store.accounts.iter_mut().find(|a| a.id == account.id) {
                    Some(old) => *old = account,
                    None => store.accounts.push(account),
                }
            }
            Ok(())
        })?;
        r["accounts"] = json!(n);
        // Client-login accounts read the local WorkBuddy client; they need that login here.
        r["needsClientLogin"] = json!(local);
    }
    if parts.settings {
        let settings = section["settings"].clone();
        let priority = section["accountPriority"].clone();
        let policies = section["modelPolicies"].clone();
        let checkin = section["checkinConfig"].clone();
        hub.wb.update_store(|store| {
            if let Ok(v) = serde_json::from_value(settings) {
                store.settings = v;
            }
            if let Ok(v) = serde_json::from_value(priority) {
                store.account_priority = v;
            }
            if let Ok(v) = serde_json::from_value(policies) {
                store.model_policies = v;
            }
            if let Ok(v) = serde_json::from_value(checkin) {
                store.checkin_config = v;
            }
            Ok(())
        })?;
        r["settings"] = json!(true);
    }
    Ok(r)
}

/// Upsert login records into the shared Switch account library by id,
/// keeping a copy of the previous file under `<data>/backups`.
pub(crate) fn merge_switch_records(data_dir: &Path, records: &[Value]) -> Result<(), String> {
    let path = workbuddy::pool::switch_file()?;
    let mut list: Vec<Value> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    if path.exists() {
        let copy = data_dir.join("backups").join(format!(
            "wb-switch-accounts-{}.json",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ));
        write_private(&copy, &std::fs::read(&path).map_err(|_| "无法读取 WorkBuddy 账号库")?)?;
    }
    for record in records {
        let Some(id) = record["id"].as_str() else { continue };
        match list.iter_mut().find(|r| r["id"] == id) {
            Some(old) => *old = record.clone(),
            None => list.push(record.clone()),
        }
    }
    write_private(&path, &serde_json::to_vec_pretty(&list).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_bundles_and_legacy_qoder_backups() {
        let dir = std::env::temp_dir().join(format!("agentdock-backup-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = dir.join("q.json");
        std::fs::write(&legacy, r#"{"format":"qoder-switch-accounts","version":1,"accounts":[{"uid":"u1","edition":"cn"}]}"#).unwrap();
        let b = read(&legacy).unwrap();
        let s = summary(&b);
        assert_eq!(s["qoder"]["accounts"], 1);
        assert_eq!(s["workbuddy"]["present"], false);
        let other = dir.join("x.json");
        std::fs::write(&other, r#"{"hello":1}"#).unwrap();
        assert!(read(&other).is_err());
        let full = dir.join("b.json");
        std::fs::write(&full, r#"{"format":"agentdock-backup","agentdock":{"settings":{}},"trae":{"accounts":[],"settings":{}}}"#).unwrap();
        let s = summary(&read(&full).unwrap());
        assert_eq!(s["agentdockSettings"], true);
        assert_eq!(s["trae"]["accounts"], 0);
        assert_eq!(s["trae"]["settings"], true);
        assert_eq!(expand("~/a.json"), dirs::home_dir().unwrap().join("a.json"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
