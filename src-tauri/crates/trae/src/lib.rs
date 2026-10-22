//! Trae provider for AgentDock (Rust port of Trae Switch): local login
//! capture and switching, browser OAuth, encrypted account vault, official
//! usage/credit packs, daily check-in and the native chat gateway transport.
pub mod platform;
pub mod api;
pub mod client;
pub mod credits;
pub mod gateway;
mod images;
pub mod keys;
pub mod oauth;
pub mod store;

use agentdock_core::{
    diagnostics::RequestDetails,
    provider::{PoolAccount, ProviderKind, SendFailure, SendInput, SendOk, UpstreamModel},
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

pub struct Provider {
    pub store: Arc<store::Store>,
    pub login: oauth::Login,
    busy: Mutex<HashSet<String>>,
    checkins: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Test-only upstream origin (never set in production builds).
    pub test_host: Mutex<Option<String>>,
}

/// Released when an exclusive account operation finishes.
pub struct Busy(Arc<Provider>, String);
impl Drop for Busy {
    fn drop(&mut self) {
        self.0.busy.lock().unwrap().remove(&self.1);
    }
}

pub fn checkin_day(time: chrono::DateTime<chrono::Utc>) -> String {
    time.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
        .format("%Y-%m-%d")
        .to_string()
}
pub fn checked_today(account: &Value) -> bool {
    account["checkin"]["checked_in"] == true
        && account["checkin"]["time"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .is_some_and(|t| {
                checkin_day(t.with_timezone(&chrono::Utc)) == checkin_day(chrono::Utc::now())
            })
}
fn expires_ms(account: &Value) -> Option<i64> {
    account["user"]["expiredAt"]
        .as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp_millis())
}
fn display_name(a: &Value) -> String {
    a["note"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| a["user"]["account"]["username"].as_str())
        .unwrap_or("未命名账号")
        .to_owned()
}

/// Credential-free account view for the UI.
pub fn public_account(a: &Value) -> Value {
    let now = chrono::Utc::now().timestamp_millis();
    json!({
        "id": a["id"],
        "variant": a["variant"],
        "username": a["user"]["account"]["username"],
        "email": a["user"]["account"]["email"],
        "note": a["note"].as_str().unwrap_or(""),
        // Already masked by the server (e.g. 138******34); the full number is never sent.
        "mobile": a["user"]["account"]["nonPlainTextMobile"].as_str().unwrap_or(""),
        "userId": a["user"]["userId"],
        "region": a["user"]["account"]["storeRegion"],
        "expiredAt": a["user"]["expiredAt"],
        "refreshExpiredAt": a["user"]["refreshExpiredAt"],
        "capturedAt": a["capturedAt"],
        "plan": a["cached"]["entitlementInfo"]["identityStr"].as_str().unwrap_or("未知"),
        "tokenPresent": a["user"]["token"].as_str().is_some_and(|s| !s.is_empty()),
        "refreshPresent": a["user"]["refreshToken"].as_str().is_some_and(|s| !s.is_empty()),
        "tokenExpired": expires_ms(a).is_none_or(|t| t <= now),
        "snapshot": a.get("snapshot").cloned().unwrap_or(Value::Null),
        "creditPacks": credits::credit_packs(a, now),
        "balance": credits::balance(a),
        "autoCheckin": a["autoCheckin"] == true,
        "checkin": a.get("checkin").cloned().unwrap_or(Value::Null),
        "checkedToday": checked_today(a),
        "enabled": a["enabled"] != false,
        "models": a["models"].as_array().cloned().unwrap_or_default(),
    })
}

impl Provider {
    pub fn open(dir: &Path) -> Result<Arc<Self>, String> {
        let store = store::Store::open(dir)?;
        Ok(Arc::new(Self {
            login: oauth::Login::new(store.clone()),
            store,
            busy: Mutex::new(HashSet::new()),
            checkins: tokio::sync::Mutex::new(HashMap::new()),
            test_host: Mutex::new(None),
        }))
    }
    pub fn busy_ids(&self) -> Vec<String> {
        self.busy.lock().unwrap().iter().cloned().collect()
    }
    pub fn exclusive(self: &Arc<Self>, id: &str) -> Result<Busy, String> {
        if !self.busy.lock().unwrap().insert(id.to_owned()) {
            return Err("该账号正在操作，请稍后重试".into());
        }
        Ok(Busy(self.clone(), id.to_owned()))
    }
    pub fn clients(&self) -> Vec<Value> {
        client::VARIANTS
            .iter()
            .map(|v| match client::profile(v) {
                Ok(p) => json!({"variant":v,"name":p.name,"installed":true,"version":p.product["appVersion"],"running":client::is_running(v)}),
                Err(_) => json!({"variant":v,"name":client::variant_name(v),"installed":false}),
            })
            .collect()
    }
    pub fn capture(&self, variant: &str) -> Result<Value, String> {
        let account = self.store.upsert(client::read_local(variant)?)?;
        self.store
            .event("capture", &format!("{} 当前账号已采集", client::variant_name(variant)));
        Ok(public_account(&account))
    }
    pub fn import(&self, variant: &str, imported: Value) -> Result<Value, String> {
        if !client::VARIANTS.contains(&variant) {
            return Err("请选择版本".into());
        }
        if !imported.is_object() {
            return Err("请输入完整账号 JSON".into());
        }
        let user = if imported["user"].is_object() {
            imported["user"].clone()
        } else {
            imported
        };
        let account = self.store.upsert(
            json!({"variant":variant,"user":user,"capturedAt":chrono::Utc::now().to_rfc3339()}),
        )?;
        Ok(public_account(&account))
    }
    pub fn update_account(&self, id: &str, patch: &Value) -> Result<Value, String> {
        let a = self.store.patch_account(id, |a| {
            if let Some(note) = patch["note"].as_str() {
                a["note"] = json!(note.chars().take(120).collect::<String>());
            }
            if let Some(auto) = patch["autoCheckin"].as_bool() {
                a["autoCheckin"] = json!(a["variant"] == "cn" && auto);
            }
            if let Some(enabled) = patch["enabled"].as_bool() {
                a["enabled"] = json!(enabled);
            }
        })?;
        Ok(public_account(&a))
    }
    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.store.remove_account(id)
    }
    pub fn switch(self: &Arc<Self>, id: &str) -> Result<Value, String> {
        let _busy = self.exclusive(id)?;
        let a = self.store.account(id)?;
        let result = client::switch_account(&a, &self.store.dir.join("backups"))?;
        self.store.event(
            "switch",
            &format!(
                "{} 账号已切换，原始存储已备份",
                client::variant_name(a["variant"].as_str().unwrap_or(""))
            ),
        );
        Ok(result)
    }
    pub async fn refresh(self: &Arc<Self>, id: &str) -> Result<Value, String> {
        let _busy = self.exclusive(id)?;
        self.refresh_unlocked(id).await
    }
    async fn refresh_unlocked(&self, id: &str) -> Result<Value, String> {
        let a = self.store.account(id)?;
        // Model lists from the old native catalog do not match Remote Agent names: replace them once.
        let stale = a["models"].as_array().is_none_or(|m| m.is_empty() || m.iter().any(|x| x["remote"] != true));
        if stale {
            if let Ok(models) = gateway::model_catalog(&a).await {
                self.store.patch_account(id, |a| a["models"] = json!(models))?;
            }
        }
        let snapshot = api::refresh_snapshot(&a).await;
        let a = self.store.patch_account(id, |a| {
            if snapshot["checkin"].is_object() {
                let mut c = snapshot["checkin"].clone();
                c["time"] = snapshot["time"].clone();
                a["checkin"] = c;
            }
            a["snapshot"] = snapshot.clone();
        })?;
        self.store.event(
            "refresh",
            &format!(
                "{} 用量刷新完成",
                client::variant_name(a["variant"].as_str().unwrap_or(""))
            ),
        );
        Ok(snapshot)
    }
    pub async fn refresh_token(self: &Arc<Self>, id: &str) -> Result<Value, String> {
        let _busy = self.exclusive(id)?;
        let a = self.store.account(id)?;
        let user = api::refresh_token(&a).await?;
        let a = self.store.patch_account(id, |a| a["user"] = user.clone())?;
        self.store.event(
            "token",
            &format!(
                "{} 登录凭证已刷新",
                client::variant_name(a["variant"].as_str().unwrap_or(""))
            ),
        );
        Ok(json!({"expiredAt": user["expiredAt"]}))
    }
    pub async fn sync_models(self: &Arc<Self>, id: &str) -> Result<Vec<Value>, String> {
        let _busy = self.exclusive(id)?;
        let a = self.store.account(id)?;
        let models = gateway::model_catalog(&a).await?;
        self.store
            .patch_account(id, |a| a["models"] = json!(models.clone()))?;
        Ok(models)
    }
    /// Concurrent check-ins for one account are merged; history is bounded and secret-free.
    pub async fn checkin(self: &Arc<Self>, id: &str, source: &str) -> Result<Value, String> {
        let gate = self
            .checkins
            .lock()
            .await
            .entry(id.to_owned())
            .or_default()
            .clone();
        let Ok(_guard) = gate.try_lock() else {
            // Another check-in for this account is running: wait for it, then report state.
            let _wait = gate.lock().await;
            let a = self.store.account(id)?;
            return Ok(json!({"already": checked_today(&a), "merged": true}));
        };
        let a = self.store.account(id)?;
        if a["variant"] != "cn" {
            return Err("国际版未提供签到接口".into());
        }
        let mut record = json!({"accountId":id,"accountName":display_name(&a),"time":chrono::Utc::now().to_rfc3339(),"source":source});
        let result = api::checkin(&a).await;
        let outcome = match &result {
            Ok((value, field)) => {
                let _ = self
                    .store
                    .patch_account(id, |a| a["checkin"] = field.clone());
                record["status"] = json!(if value["already"] == true {
                    "already"
                } else {
                    "success"
                });
                record["credits"] = field["credits"].clone();
                Ok(value.clone())
            }
            Err(_) => {
                record["status"] = json!("error");
                record["error"] = json!("签到未完成，请检查凭证或官方签到开放状态");
                Err("签到未完成，请检查凭证或官方签到开放状态".to_owned())
            }
        };
        let _ = self.store.update(|d| {
            let history = d["checkinHistory"].as_array_mut().unwrap();
            history.insert(0, record);
            history.truncate(300);
            Ok(())
        });
        if outcome.is_ok() {
            self.store.event("checkin", "签到状态已确认");
        }
        outcome
    }
    /// Startup catch-up and periodic automatic check-in for opted-in CN accounts.
    /// Check in every opted-in CN account not yet checked today.
    pub async fn auto_checkin_tick(self: &Arc<Self>) -> Vec<Value> {
        let due: Vec<(String, String)> = self
            .store
            .accounts()
            .iter()
            .filter(|a| a["autoCheckin"] == true && a["variant"] == "cn" && !checked_today(a))
            .filter_map(|a| a["id"].as_str().map(|id| (id.to_owned(), display_name(a))))
            .collect();
        let mut results = vec![];
        for (id, name) in due {
            let Ok(_busy) = self.exclusive(&id) else {
                results.push(json!({"id": id, "name": name, "ok": false, "message": "账号正忙，稍后重试"}));
                continue;
            };
            match self.checkin(&id, "auto").await {
                Ok(result) => {
                    let already = result["already"] == true;
                    if !already {
                        let _ = self.refresh_unlocked(&id).await;
                        self.store.event("checkin", "自动签到成功");
                    }
                    results.push(json!({"id": id, "name": name, "ok": true,
                        "message": if already { "今日已签到" } else { "签到成功" }}));
                }
                Err(e) => {
                    self.store.event("error", &format!("自动签到：{e}"));
                    results.push(json!({"id": id, "name": name, "ok": false, "message": e.to_string()}));
                }
            }
        }
        results
    }

    pub fn pool_accounts(&self) -> Vec<PoolAccount> {
        let now = chrono::Utc::now().timestamp_millis();
        self.store
            .accounts()
            .iter()
            .enumerate()
            .map(|(order, a)| PoolAccount {
                provider: ProviderKind::Trae,
                id: a["id"].as_str().unwrap_or("").into(),
                region: a["variant"].as_str().unwrap_or("cn").into(),
                name: display_name(a),
                enabled: a["enabled"] != false,
                expires_at: expires_ms(a),
                credit: credits::rank(a, now),
                order,
                blocked: (!a["user"]["token"].as_str().is_some_and(|s| !s.is_empty()))
                    .then(|| "缺少登录令牌".into()),
            })
            .collect()
    }
    /// Union of per-account model lists, keyed by region and real model name.
    pub fn upstream_models(&self) -> Vec<UpstreamModel> {
        let mut out: Vec<UpstreamModel> = vec![];
        for a in self.store.accounts() {
            let region = a["variant"].as_str().unwrap_or("cn").to_owned();
            let mut push = |id: &str, name: &str, config: Value| {
                if !out.iter().any(|m| m.region == region && m.id == id) {
                    out.push(UpstreamModel {
                        provider: Some(ProviderKind::Trae),
                        region: region.clone(),
                        id: id.into(),
                        name: name.into(),
                        // Client tools run through the Remote text bridge.
                        supports_tools: Some(true),
                        supports_images: Some(true),
                        config,
                        ..UpstreamModel::default()
                    });
                }
            };
            push("auto", "auto", json!({"auto":true}));
            for m in a["models"].as_array().into_iter().flatten() {
                if let Some(real) = gateway::real_model(m) {
                    let name = m["name"].as_str().filter(|s| !s.is_empty()).unwrap_or(real);
                    push(real, name, m.clone());
                }
            }
        }
        out
    }
    /// Trae model lists are per account; returns that account's native config id.
    pub fn chat_eligibility(&self, account_id: &str, model: &str) -> Result<Option<String>, &'static str> {
        let a = self.store.account(account_id).map_err(|_| "账号不存在")?;
        if model == "auto" {
            return Ok(None);
        }
        a["models"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|m| gateway::real_model(m) == Some(model))
            .map(|m| m["id"].as_str().map(str::to_owned))
            .ok_or("账号不提供此模型")
    }
    pub async fn send(
        &self,
        account_id: &str,
        config_id: Option<String>,
        input: SendInput,
        details: &mut RequestDetails,
        started: Instant,
    ) -> Result<SendOk, SendFailure> {
        let a = self
            .store
            .account(account_id)
            .map_err(|_| SendFailure::local(503, "no_available_account", "账号已移除"))?;
        let host = if cfg!(any(test, feature = "fixtures")) {
            self.test_host.lock().unwrap().clone()
        } else {
            None
        };
        gateway::send(&a, config_id, input, details, started, host).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkin_day_uses_shanghai_calendar() {
        let t = chrono::DateTime::parse_from_rfc3339("2026-10-20T17:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(checkin_day(t), "2026-10-21");
        assert!(!checked_today(&json!({"checkin":{"checked_in":true,"time":"2000-01-01T00:00:00Z"}})));
        assert!(checked_today(
            &json!({"checkin":{"checked_in":true,"time":chrono::Utc::now().to_rfc3339()}})
        ));
    }
    #[test]
    fn pool_and_models_follow_vault_and_eligibility_is_per_account() {
        let dir = std::env::temp_dir().join(format!("agentdock-trae-p-{}", uuid::Uuid::new_v4()));
        let p = Provider::open(&dir).unwrap();
        for (variant, models) in [("cn", vec!["cn-only", "shared"]), ("intl", vec!["intl-only", "shared"])] {
            p.store
                .upsert(json!({"variant":variant,"user":{"userId":variant,"token":"t","expiredAt":"2030-01-01T00:00:00Z","account":{"username":variant}},
                    "models": models.iter().map(|m| json!({"id":format!("cfg-{variant}-{m}"),"model":m,"name":m})).collect::<Vec<_>>()}))
                .unwrap();
        }
        let pool = p.pool_accounts();
        assert_eq!(pool.len(), 2);
        let models = p.upstream_models();
        assert_eq!(models.iter().filter(|m| m.id == "shared").count(), 2);
        assert_eq!(models.iter().filter(|m| m.id == "auto").count(), 2);
        let cn = pool.iter().find(|a| a.region == "cn").unwrap();
        assert_eq!(
            p.chat_eligibility(&cn.id, "shared").unwrap().as_deref(),
            Some("cfg-cn-shared")
        );
        assert!(p.chat_eligibility(&cn.id, "intl-only").is_err());
        assert_eq!(p.chat_eligibility(&cn.id, "auto").unwrap(), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
