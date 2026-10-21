//! Thin adapter around the MIT Switch core; credentials never leave Rust.
use crate::pool::{self, Account, ModelLimit, Shared};
use serde_json::{json, Value};
use std::collections::HashMap;
use wb_switch_core::modules as switch;

fn source_account(a: &Account) -> Result<Value, String> {
    let rows = switch::account::load_accounts();
    let uid = a.id.split_once(':').map(|(_, uid)| uid).unwrap_or("");
    rows.into_iter()
        .find(|r| {
            (a.source == "switch" && r["id"] == a.source_id)
                || (r["uid"] == uid && switch::account::variant_of(r).as_str() == a.variant)
        })
        .ok_or("Switch 来源中找不到此账号".into())
}

pub fn refresh_local(state: &Shared) -> Result<(), String> {
    let accounts = state.store.lock().unwrap().accounts.clone();
    let source = switch::limits::get_rate_limits_read_only();
    let cli = switch::codebuddy_cli::status();
    let cn_ide = switch::codebuddy_cn_ide::status();
    let ai_ide = switch::codebuddy_ide::status();
    let vscode = switch::vscode_ext::status();
    let mut external = HashMap::new();
    let mut presentations = HashMap::new();
    let current_cn = pool::auth_file("cn")
        .ok()
        .and_then(|p| pool::read_json(&p).ok())
        .unwrap_or(Value::Null);
    let current_ai = pool::auth_file("ai")
        .ok()
        .and_then(|p| pool::read_json(&p).ok())
        .unwrap_or(Value::Null);
    for a in accounts {
        let raw = source_account(&a).unwrap_or(json!({}));
        let meta = switch::account::account_meta(&raw);
        let source_id = raw["id"].as_str().unwrap_or(&a.source_id);
        let current = if a.variant == "ai" {
            &current_ai
        } else {
            &current_cn
        };
        let ide = if a.variant == "ai" { &ai_ide } else { &cn_ide };
        let limits = source["accounts"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["accountId"] == source_id)
            .and_then(|row| row["limited"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|l| {
                Some(ModelLimit {
                    model: l["model"].as_str().map(String::from),
                    reset_at: l["resetAt"].as_i64()?,
                    first_seen_at: l["firstSeenAt"].as_i64().unwrap_or(0),
                    source: "switch-log".into(),
                })
            })
            .collect::<Vec<_>>();
        external.insert(a.id.clone(), limits);
        let name = if meta["displayField"] == "note" {
            meta["note"].as_str()
        } else if meta["displayField"] == "phone" {
            meta["phoneNumber"].as_str()
        } else {
            meta["nickname"].as_str()
        };
        presentations.insert(a.id.clone(),json!({"uid":raw["uid"],"nickname":meta["nickname"],"email":meta["email"],
            "phoneNumber":meta["phoneNumber"],"note":meta["note"],"displayName":name.filter(|s|!s.is_empty()).unwrap_or(&a.name),
            "needsRelogin":meta["needsRelogin"],"travel":switch::travel::travel_display(source_id),
            "clients":{"workbuddy":raw["uid"].is_string()&&current["account"]["uid"]==raw["uid"],
                "cli":cli["activeAccountId"]==source_id,"ide":ide["activeAccountId"]==source_id,"vscode":vscode["activeAccountId"]==source_id}}));
    }
    let mut rt = state.runtime.lock().unwrap();
    rt.external_limits = external;
    rt.presentations = presentations;
    rt.clients = json!({"cli":{"available":cli["configured"]==true,"migrationRequired":cli["migrationRequired"]},
        "cnIde":{"available":cn_ide["installed"]==true&&cn_ide["dbExists"]==true},
        "aiIde":{"available":ai_ide["installed"]==true&&ai_ide["dbExists"]==true},
        "vscode":{"available":vscode["installed"]==true&&vscode["extensionInstalled"]==true},"scannedAt":source["scannedAt"]});
    Ok(())
}

pub async fn refresh_credit(state: Shared, id: String) -> Result<Value, String> {
    let (a, mut lease) = state.acquire("credits", Some(&id))?;
    let c = pool::credentials(&a)?;
    // This sanitized request object is memory-only; no refresh token is passed.
    let raw = json!({"id":a.source_id,"uid":c.uid,"nickname":a.name,"variant":a.variant,
        "domain":c.domain,"access_token":c.token,"enterpriseId":c.enterprise});
    let mut credit = switch::credits::get_credit_expiry_read_only(&raw).await;
    credit["accountId"] = json!(a.id);
    lease.status = if credit["ok"] == true { 200 } else { 502 };
    lease.outcome = if credit["ok"] == true {
        "completed"
    } else {
        "credit_query_failed"
    }
    .into();
    let cached = credit.clone();
    tokio::task::spawn_blocking(move || state.cache_credit(&id, cached))
        .await
        .map_err(|e| e.to_string())??;
    Ok(credit)
}

pub fn switch_client(
    state: &Shared,
    id: &str,
    target: &str,
    confirmed: bool,
) -> Result<Value, String> {
    if !confirmed {
        return Err("请先确认客户端重启和登录态改写".into());
    }
    let a = state
        .store
        .lock()
        .unwrap()
        .accounts
        .iter()
        .find(|a| a.id == id)
        .cloned()
        .ok_or("账号不存在")?;
    let raw = source_account(&a)?;
    let source_id = raw["id"].as_str().ok_or("缺少来源账号 ID")?;
    let result = match target {
        "workbuddy" => switch::switch::switch_account(None, source_id, true, false, &[], &[]),
        "cli" => switch::codebuddy_cli::switch_active_account(source_id),
        "ide" if a.variant == "cn" => switch::codebuddy_cn_ide::switch_account(source_id, true),
        "ide" => switch::codebuddy_ide::switch_account(source_id, true),
        "vscode" => switch::vscode_ext::switch_account(source_id, true),
        _ => Err("不支持的客户端".into()),
    }?;
    refresh_local(state)?;
    // Core switch reports may include profile metadata: keep the frontend result minimal.
    Ok(json!({"ok":true,"target":target,"status":result["status"]}))
}

pub fn observe_limit(state: &Shared, id: &str, model: &str, data: &Value) -> bool {
    let message = data.to_string();
    if !(message.contains("超出频率限制")
        || message
            .to_lowercase()
            .contains("usage exceeds frequency limit"))
    {
        return false;
    }
    let Some(reset) = switch::limits::official_reset_at(&message) else {
        return false;
    };
    if reset <= chrono::Utc::now().timestamp_millis() {
        return false;
    }
    let model = if ["auto", "undefined", "null", ""].contains(&model) {
        None
    } else {
        Some(model.to_owned())
    };
    state.record_model_limit(id, model, reset).is_ok()
}

pub async fn run_local_monitor(state: Shared) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tick.tick().await;
        let state = state.clone();
        let _ = tokio::task::spawn_blocking(move || refresh_local(&state)).await;
    }
}
