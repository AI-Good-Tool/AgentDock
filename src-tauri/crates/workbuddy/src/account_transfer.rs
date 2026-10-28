//! Account-pool removal and portable, owner-only credential exports.
use crate::pool::{credentials, read_json, switch_file, Account, AppState};
use serde_json::{json, Value};
use std::{collections::HashSet, io::Write, path::Path};

pub fn selected_ids(ids: &[String]) -> Result<HashSet<&str>, String> {
    if ids.is_empty() || ids.iter().any(|id| id.trim().is_empty()) {
        return Err("请至少选择一个有效账号".into());
    }
    Ok(ids.iter().map(String::as_str).collect())
}

fn select(accounts: &[Account], ids: &[String]) -> Result<Vec<Account>, String> {
    let ids = selected_ids(ids)?;
    let selected: Vec<_> = accounts.iter().filter(|a| ids.contains(a.id.as_str())).cloned().collect();
    if selected.len() != ids.len() {
        return Err("部分账号已移除，请刷新后重新选择".into());
    }
    Ok(selected)
}

pub fn export_account_at(account: &Account, source: &Path) -> Result<Value, String> {
    let mut record = if account.source == "switch" {
        let records = read_json(&source.to_owned())?;
        records.as_array().ok_or("账号库格式错误")?.iter().find(|r| {
            r["id"] == account.source_id
                && r["uid"].as_str() == account.id.split_once(':').map(|(_, uid)| uid)
                && wb_switch_core::modules::account::variant_of(r).as_str() == account.variant
        }).cloned().ok_or("来源账号已移除或身份已改变，请重新导入")?
    } else {
        let c = credentials(account)?;
        json!({"uid":c.uid,"nickname":account.name,"access_token":c.token,"domain":c.domain,
            "enterpriseId":c.enterprise,"variant":account.variant,"expiresAt":account.expires_at})
    };
    if !record["access_token"].as_str().is_some_and(|s| !s.trim().is_empty()) {
        return Err("账号缺少可导出的明文登录凭据，请先重新登录".into());
    }
    record["note"] = json!(account.note);
    record["displayField"] = json!(account.display_field);
    record["variant"] = json!(account.variant);
    Ok(json!({"format":"agentdock-account","version":1,"provider":"workbuddy",
        "region":if account.variant == "ai" {"intl"} else {"cn"},"account":record}))
}

pub fn export_account(account: &Account) -> Result<Value, String> {
    export_account_at(account, &switch_file()?)
}

pub fn export_at(state: &AppState, ids: &[String], source: &Path) -> Result<Value, String> {
    let accounts = select(&state.store.lock().unwrap().accounts, ids)?;
    let records: Result<Vec<_>, _> = accounts.iter().map(|a| export_account_at(a, source)).collect();
    Ok(json!({"format":"agentdock-accounts","version":1,
        "exportedAt":chrono::Utc::now().to_rfc3339(),"accounts":records?}))
}

pub fn save_to(state: &AppState, ids: &[String], dir: &Path, confirmed: bool) -> Result<Value, String> {
    if !confirmed { return Err("请确认导出包含完整登录凭据的账号文件".into()); }
    // Resolve every selected record before creating a file; never silently skip an account.
    let doc = export_at(state, ids, &switch_file()?)?;
    write_export(&doc, dir)
}

fn write_export(doc: &Value, dir: &Path) -> Result<Value, String> {
    std::fs::create_dir_all(dir).map_err(|_| "无法创建导出目录")?;
    let path = dir.join(format!("workbuddy-accounts-{}-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S"), uuid::Uuid::new_v4().simple()));
    let data = serde_json::to_vec_pretty(doc).map_err(|_| "无法生成账号 JSON")?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|_| "无法创建账号导出文件")?;
    if file.write_all(&data).and_then(|_| file.sync_all()).is_err() {
        let _ = std::fs::remove_file(&path);
        return Err("保存账号文件失败，请检查磁盘空间".into());
    }
    Ok(json!({"ok":true,"count":doc["accounts"].as_array().map_or(0, Vec::len),"path":path}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_preserves_credentials_region_and_private_file_permissions() {
        let dir = std::env::temp_dir().join(format!("wb-export-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = &dir.join("source.json");
        std::fs::write(&source, r#"[{"id":"src","uid":"u","variant":"ai","access_token":"fixture-only","refresh_token":"fixture-refresh"}]"#).unwrap();
        let a: Account = serde_json::from_value(json!({"id":"ai:u","source":"switch","sourceId":"src","variant":"ai","name":"fixture","enabled":false,"note":"备注","displayField":"note","expiresAt":null})).unwrap();
        let record = export_account_at(&a, &source).unwrap();
        assert_eq!(record["region"], "intl");
        assert_eq!(record["account"]["refresh_token"], "fixture-refresh");
        assert_eq!(record["account"]["note"], "备注");
        let result = write_export(&json!({"accounts":[record]}), &dir).unwrap();
        let path = Path::new(result["path"].as_str().unwrap());
        let saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["accounts"][0]["account"]["access_token"], "fixture-only");
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let mut changed = a.clone(); changed.id = "ai:other".into();
        assert!(export_account_at(&changed, &source).is_err());
        assert!(select(&[a.clone()], &[a.id.clone(), "missing".into()]).is_err());
        assert_eq!(select(&[a.clone()], &[a.id.clone(),a.id.clone()]).unwrap().len(),1);
        assert!(selected_ids(&[]).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
