//! Single/batch account JSON export and import. Records keep each platform's
//! native shape (WorkBuddy Switch
//! library record, Qoder Switch account, Trae vault account) so files also
//! work with the original tools.
use crate::app::{write_private, Shared};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const FORMAT: &str = "agentdock-account";
pub const BATCH_FORMAT: &str = "agentdock-accounts";

#[derive(serde::Deserialize)]
pub struct AccountRef { pub provider: String, pub id: String }

pub fn export_batch(hub: &Shared, accounts: &[AccountRef]) -> Result<Value, String> {
    if accounts.is_empty() { return Err("请至少选择一个账号".into()); }
    let mut seen = HashSet::new();
    let mut records = vec![];
    for a in accounts {
        if seen.insert((&a.provider, &a.id)) { records.push(export(hub, &a.provider, &a.id)?); }
    }
    Ok(json!({"format": BATCH_FORMAT, "version": 1, "exportedAt": chrono::Utc::now().to_rfc3339(), "accounts": records}))
}

pub fn save_batch(hub: &Shared, accounts: &[AccountRef]) -> Result<Value, String> {
    let doc = export_batch(hub, accounts)?;
    let dir = dirs::download_dir().or_else(dirs::home_dir).ok_or("找不到下载目录")?;
    let path = dir.join(format!("agentdock-accounts-{}-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S"), uuid::Uuid::new_v4().simple()));
    write_private(&path, &serde_json::to_vec_pretty(&doc).unwrap())?;
    Ok(json!({"ok":true,"count":doc["accounts"].as_array().unwrap().len(),"path":path,"message":format!("已保存到 {}",path.display())}))
}

/// Full record of one account, credentials included.
pub fn export(hub: &Shared, provider: &str, id: &str) -> Result<Value, String> {
    let (region, record) = match provider {
        "workbuddy" => {
            let a = hub
                .wb
                .store
                .lock()
                .unwrap()
                .accounts
                .iter()
                .find(|a| a.id == id)
                .cloned()
                .ok_or("账号不存在")?;
            return workbuddy::account_transfer::export_account(&a);
        }
        "qoder" => {
            let a = hub.qoder.accounts()
                .into_iter()
                .find(|a| a.id() == id)
                .ok_or("账号不存在")?;
            (a.edition.clone(), json!(a))
        }
        "trae" => {
            let a = hub.trae.store.account(id)?;
            (a["variant"].as_str().unwrap_or("cn").to_owned(), a)
        }
        _ => return Err("未知的平台".into()),
    };
    Ok(json!({
        "format": FORMAT,
        "version": 1,
        "provider": provider,
        "region": region,
        "exportedAt": chrono::Utc::now().to_rfc3339(),
        "account": record,
    }))
}

/// Accepts one wrapped/raw record, an array of either, or `{accounts: [...]}`.
fn records(input: &Value, provider: &str) -> Result<Vec<(Option<String>, Value)>, String> {
    if input["format"].as_str().is_some_and(|f| f != FORMAT && f != BATCH_FORMAT) {
        return Err("不支持的账号 JSON 格式".into());
    }
    if input["format"] == BATCH_FORMAT && !input["accounts"].is_array() {
        return Err("批量账号 JSON 缺少 accounts 数组".into());
    }
    let list = if let Some(a) = input.as_array() {
        a.clone()
    } else if let Some(a) = input["accounts"].as_array() {
        a.clone()
    } else {
        vec![input.clone()]
    };
    let mut out = vec![];
    for item in list {
        if item["format"] == FORMAT {
            if item["provider"] != provider {
                return Err(format!("这是 {} 的账号 JSON，请在对应平台导入", item["provider"].as_str().unwrap_or("其他平台")));
            }
            out.push((item["region"].as_str().map(str::to_owned), item["account"].clone()));
        } else if item.is_object() && item.get("format").is_none() {
            out.push((None, item));
        } else {
            return Err("账号记录必须是有效对象，且使用受支持的格式".into());
        }
    }
    if out.is_empty() {
        return Err("JSON 中没有账号记录".into());
    }
    Ok(out)
}

fn prepare(provider: &str, region: Option<&str>, input: &Value) -> Result<Vec<(Option<String>, Value)>, String> {
    let mut items = records(input, provider)?;
    for (r, rec) in &mut items {
        match provider {
            "workbuddy" => {
                if !rec["uid"].as_str().is_some_and(|s| !s.is_empty()) || !rec["access_token"].as_str().is_some_and(|s| !s.is_empty()) {
                    return Err("WorkBuddy 账号 JSON 需要 uid 与 access_token".into());
                }
                use wb_switch_core::modules::{account::variant_of, variant::WbVariant};
                if rec["variant"].as_str().is_none_or(str::is_empty) {
                    let variant = if rec["domain"].as_str().is_some_and(|s| !s.is_empty()) { variant_of(rec).as_str() }
                        else if r.as_deref().or(region) == Some("intl") { "ai" } else { "cn" };
                    rec["variant"] = json!(variant);
                }
                let variant = match rec["variant"].as_str() { Some("cn") => WbVariant::Cn, Some("ai") => WbVariant::Ai, _ => return Err("WorkBuddy 账号版本无效".into()) };
                if !variant.matches_domain(rec["domain"].as_str().unwrap_or(""))
                    || r.as_deref().is_some_and(|r| r != if variant == WbVariant::Ai { "intl" } else { "cn" }) {
                    return Err("WorkBuddy 账号的地区与 domain / 导出地区不一致".into());
                }
            }
            "qoder" => {
                let a: qoder::qcore::Account = serde_json::from_value(rec.clone()).map_err(|_| "Qoder 账号 JSON 字段格式无效")?;
                if a.uid.is_empty() || !["cn","intl"].contains(&a.edition.as_str()) { return Err("Qoder 账号 JSON 需要有效 edition 与 uid".into()); }
            }
            "trae" => {
                let variant = rec["variant"].as_str().or(r.as_deref()).or(region).unwrap_or("cn").to_owned();
                if !["cn","intl"].contains(&variant.as_str()) { return Err("Trae 账号版本无效".into()); }
                trae::store::validate_user(if rec["user"].is_object() { &rec["user"] } else { rec })?;
                *r = Some(variant);
            }
            _ => return Err("未知的平台".into()),
        }
    }
    Ok(items)
}

fn import_plan(provider: &str, region: Option<&str>, input: &Value) -> Result<BTreeMap<String, Vec<(Option<String>, Value)>>, String> {
    if provider != "auto" {
        let items = prepare(provider, region, input)?;
        if provider == "workbuddy" { validate_workbuddy_ids(&items)?; }
        return Ok(BTreeMap::from([(provider.into(), items)]));
    }
    let list = input.as_array().or_else(|| input["accounts"].as_array()).cloned().unwrap_or_else(|| vec![input.clone()]);
    if list.is_empty() { return Err("JSON 中没有账号记录".into()); }
    if input["format"].as_str().is_some_and(|f| f != FORMAT && f != BATCH_FORMAT) { return Err("不支持的账号 JSON 格式".into()); }
    let mut groups: BTreeMap<String, Vec<(Option<String>, Value)>> = BTreeMap::new();
    for (index, item) in list.iter().enumerate() {
        let p = if item["format"] == FORMAT { item["provider"].as_str() }
            else if item["access_token"].is_string() && item["uid"].is_string() { Some("workbuddy") }
            else if item["edition"].is_string() && item["uid"].is_string() { Some("qoder") }
            else if item["user"].is_object() || item["userId"].is_string() { Some("trae") }
            else { None }.ok_or_else(|| format!("第 {} 条账号无法识别平台",index+1))?;
        let prepared = prepare(p, region, item).map_err(|e| format!("第 {} 条账号：{e}", index+1))?;
        groups.entry(p.into()).or_default().extend(prepared);
    }
    if let Some(items) = groups.get("workbuddy") { validate_workbuddy_ids(items)?; }
    Ok(groups)
}

fn validate_workbuddy_ids(items: &[(Option<String>, Value)]) -> Result<(), String> {
    let mut identities = HashMap::new();
    for (_, record) in items {
        if let Some(id) = record["id"].as_str().filter(|s| !s.is_empty()) {
            let identity = (&record["uid"], &record["variant"]);
            if identities.insert(id, identity).is_some_and(|old| old != identity) {
                return Err("多个 WorkBuddy 账号使用了相同记录 ID，请修正 JSON 后重试".into());
            }
        }
    }
    Ok(())
}

pub fn import(hub: &Shared, provider: &str, region: Option<&str>, input: &Value) -> Result<Value, String> {
    // Validate every record across every provider before the first write.
    let groups = import_plan(provider, region, input)?;
    let mut count = 0;
    let mut providers = serde_json::Map::new();
    for (provider, items) in groups {
        let n = import_records(hub, &provider, region, items).map_err(|e| format!("已完成 {count} 条；{provider} 导入中断，部分记录可能已写入：{e}"))?;
        count += n;
        providers.insert(provider, json!(n));
    }
    Ok(json!({"ok":true,"count":count,"providers":providers,"message":format!("已导入 {count} 个账号")}))
}

fn import_records(hub: &Shared, provider: &str, region: Option<&str>, items: Vec<(Option<String>, Value)>) -> Result<usize, String> {
    let count = items.len();
    let kind = agentdock_core::provider::ProviderKind::parse(provider).ok_or("未知的平台")?;
    let before = hub.credential_revisions(kind);
    match provider {
        "workbuddy" => {
            let mut ids = vec![];
            let mut list = vec![];
            for (_, mut rec) in items {
                if rec["id"].as_str().is_none_or(str::is_empty) {
                    rec["id"] = json!(uuid::Uuid::new_v4().to_string());
                }
                ids.push(rec["id"].as_str().unwrap().to_owned());
                list.push(rec);
            }
            crate::backup::merge_switch_records(&hub.dir, &list)?;
            for id in &ids {
                hub.wb.import_account_source("switch", Some(id))?;
            }
        }
        "qoder" => {
            let accounts: Vec<Value> = items.into_iter().map(|(_, v)| v).collect();
            let n = qoder::qcore::import_accounts_value(json!(accounts))?;
            if n == 0 {
                return Err("Qoder 账号 JSON 需要 edition 与 uid".into());
            }
        }
        "trae" => {
            for (r, rec) in items {
                let variant = rec["variant"]
                    .as_str()
                    .map(str::to_owned)
                    .or(r)
                    .or(region.map(str::to_owned))
                    .unwrap_or_else(|| "cn".into());
                hub.trae.import(&variant, rec)?;
            }
        }
        _ => return Err("未知的平台".into()),
    }
    hub.restore_changed_logins(kind, &before)?;
    agentdock_core::applog::info("accounts", || (format!("已通过 JSON 导入 {count} 个 {provider} 账号"), Value::Null));
    Ok(count)
}

/// Save an exported account JSON to Downloads (owner-only).
pub fn save(hub: &Shared, provider: &str, id: &str) -> Result<Value, String> {
    let doc = export(hub, provider, id)?;
    let dir = dirs::download_dir().or_else(dirs::home_dir).ok_or("找不到下载目录")?;
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).take(40).collect();
    let path = dir.join(format!("{provider}-account-{safe}.json"));
    write_private(&path, &serde_json::to_vec_pretty(&doc).unwrap())?;
    Ok(json!({"ok": true, "path": path, "message": format!("已保存到 {}", path.display())}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workbuddy_legacy_domain_preserves_region_and_conflicting_regions_are_rejected() {
        let legacy = json!({"uid":"u","access_token":"fixture","domain":"www.workbuddy.ai"});
        assert_eq!(import_plan("auto",Some("cn"),&legacy).unwrap()["workbuddy"][0].1["variant"],"ai");
        let conflicting = json!({"variant":"cn","uid":"u","access_token":"fixture","domain":"www.workbuddy.ai"});
        assert!(import_plan("auto",None,&conflicting).is_err());
        let wrapper = json!({"format":FORMAT,"provider":"workbuddy","region":"cn","account":legacy});
        assert!(import_plan("auto",None,&wrapper).is_err());
    }
    #[test]
    fn records_accept_wrapped_raw_and_lists() {
        let wrapped = json!({"format": FORMAT, "provider": "qoder", "region": "intl", "account": {"uid": "u"}});
        let r = records(&wrapped, "qoder").unwrap();
        assert_eq!((r[0].0.as_deref(), &r[0].1["uid"]), (Some("intl"), &json!("u")));
        assert!(records(&wrapped, "trae").is_err());
        assert_eq!(records(&json!([{"uid": "a"}, {"uid": "b"}]), "trae").unwrap().len(), 2);
        assert_eq!(records(&json!({"accounts": [{"uid": "a"}]}), "qoder").unwrap().len(), 1);
        assert!(records(&json!([]), "qoder").is_err());
        assert!(records(&json!([{"uid":"a"}, null]), "qoder").is_err());
    }

    #[test]
    fn batch_plan_routes_mixed_exports_and_preserves_credentials_and_regions() {
        let input = json!({"format":BATCH_FORMAT,"version":1,"accounts":[
            {"format":FORMAT,"provider":"workbuddy","region":"intl","account":{"uid":"w","access_token":"fixture-w"}},
            {"format":FORMAT,"provider":"qoder","region":"cn","account":{"edition":"cn","uid":"q","token":"fixture-q"}},
            {"format":FORMAT,"provider":"trae","region":"intl","account":{"user":{"userId":"t","token":"fixture-t","expiredAt":"2099-01-01T00:00:00Z","account":{}}}}
        ]});
        let plan = import_plan("auto", Some("cn"), &input).unwrap();
        assert_eq!(plan.len(),3);
        assert_eq!(plan["workbuddy"][0].1["variant"],"ai");
        assert_eq!(plan["qoder"][0].1["token"],"fixture-q");
        assert_eq!(plan["trae"][0].0.as_deref(),Some("intl"));
        assert!(import_plan("workbuddy", None, &input).is_err());
    }

    #[test]
    fn batch_plan_rejects_invalid_records_before_import() {
        let valid = json!({"edition":"cn","uid":"q","token":"fixture"});
        for invalid in [Value::Null,json!({}),json!({"edition":"other","uid":"q"}),json!({"format":FORMAT,"provider":"trae","account":{"user":{"userId":"x"}}}),json!({"format":"unknown","edition":"cn","uid":"q"})] {
            assert!(import_plan("auto",None,&json!([valid,invalid])).is_err());
        }
        assert!(import_plan("auto",None,&json!({"format":BATCH_FORMAT,"accounts":[]})).is_err());
    }

    #[test]
    fn batch_plan_rejects_conflicting_workbuddy_source_ids_across_wrappers() {
        let record = |uid: &str| json!({"format":FORMAT,"provider":"workbuddy","region":"cn","account":{"id":"same-source-id","uid":uid,"access_token":"fixture-only"}});
        let input = json!({"format":BATCH_FORMAT,"accounts":[record("first"), record("second")]});
        assert!(import_plan("auto",None,&input).is_err());
        assert!(import_plan("workbuddy",None,&input).is_err());
        // Repeated records for the same identity may still update that login.
        assert!(import_plan("auto",None,&json!([record("first"),record("first")])).is_ok());
    }
}
