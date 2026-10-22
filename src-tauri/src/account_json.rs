//! Per-account JSON: view/export one account's full record and import accounts
//! from JSON. Records keep each platform's native shape (WorkBuddy Switch
//! library record, Qoder Switch account, Trae vault account) so files also
//! work with the original tools.
use crate::app::{write_private, Shared};
use serde_json::{json, Value};

pub const FORMAT: &str = "agentdock-account";

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
            let record = if a.source == "switch" {
                std::fs::read(workbuddy::pool::switch_file()?)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .and_then(|v| v.as_array().and_then(|l| l.iter().find(|r| r["id"] == a.source_id.as_str()).cloned()))
                    .ok_or("WorkBuddy Switch 账号库中找不到该账号的登录记录")?
            } else {
                // Client-login account: rebuild a library-shaped record from the live login.
                let c = workbuddy::pool::credentials(&a)?;
                json!({
                    "uid": c.uid,
                    "nickname": a.name,
                    "access_token": c.token,
                    "domain": c.domain,
                    "enterpriseId": c.enterprise,
                    "variant": a.variant,
                    "expiresAt": a.expires_at,
                })
            };
            (if a.variant == "ai" { "intl" } else { "cn" }.to_owned(), record)
        }
        "qoder" => {
            let a = qoder::qcore::load_accounts()
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
        } else if item.is_object() {
            out.push((None, item));
        }
    }
    if out.is_empty() {
        return Err("JSON 中没有账号记录".into());
    }
    Ok(out)
}

pub fn import(hub: &Shared, provider: &str, region: Option<&str>, input: &Value) -> Result<Value, String> {
    let items = records(input, provider)?;
    let count = items.len();
    match provider {
        "workbuddy" => {
            let mut ids = vec![];
            let mut list = vec![];
            for (r, mut rec) in items {
                if !rec["uid"].as_str().is_some_and(|s| !s.is_empty())
                    || !rec["access_token"].as_str().is_some_and(|s| !s.is_empty())
                {
                    return Err("WorkBuddy 账号 JSON 需要 uid 与 access_token".into());
                }
                if rec["variant"].as_str().is_none_or(str::is_empty) {
                    let intl = r.as_deref().or(region) == Some("intl");
                    rec["variant"] = json!(if intl { "ai" } else { "cn" });
                }
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
    agentdock_core::applog::info("accounts", || (format!("已通过 JSON 导入 {count} 个 {provider} 账号"), Value::Null));
    Ok(json!({"ok": true, "count": count, "message": format!("已导入 {count} 个账号")}))
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
    fn records_accept_wrapped_raw_and_lists() {
        let wrapped = json!({"format": FORMAT, "provider": "qoder", "region": "intl", "account": {"uid": "u"}});
        let r = records(&wrapped, "qoder").unwrap();
        assert_eq!((r[0].0.as_deref(), &r[0].1["uid"]), (Some("intl"), &json!("u")));
        assert!(records(&wrapped, "trae").is_err());
        assert_eq!(records(&json!([{"uid": "a"}, {"uid": "b"}]), "trae").unwrap().len(), 2);
        assert_eq!(records(&json!({"accounts": [{"uid": "a"}]}), "qoder").unwrap().len(), 1);
        assert!(records(&json!([]), "qoder").is_err());
    }
}
