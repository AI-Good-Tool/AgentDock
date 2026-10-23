//! The AgentDock hub: providers, settings, the routing table, the scheduler
//! runtime and the request log, shared by the HTTP gateway and the desktop UI.
use crate::{
    catalog::{self, Route},
    logs::{LogStore, RequestLog},
    scheduler::{self, Exec},
    settings::{RouteOverride, Settings},
};
use agentdock_core::provider::{PoolAccount, ProviderKind, UpstreamModel};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub type Shared = Arc<Hub>;

pub struct ServerHandle {
    pub port: u16,
    pub stop: tokio::sync::oneshot::Sender<()>,
}

pub struct Hub {
    pub dir: PathBuf,
    settings: Mutex<Settings>,
    settings_writer: Mutex<()>,
    /// Keep native switches, durable suspension reasons and runtime marks in step.
    account_writer: Mutex<()>,
    pub key: crate::api_key::LocalKey,
    pub wb: workbuddy::pool::Shared,
    pub qoder: Arc<qoder::gateway::Provider>,
    pub trae: Arc<trae::Provider>,
    pub runtime: Mutex<scheduler::Runtime>,
    pub logs: Arc<LogStore>,
    /// Requests currently streaming (for the overview and request list).
    pub active: Mutex<HashMap<String, RequestLog>>,
    pub server: Mutex<Option<ServerHandle>>,
    pub error: Mutex<String>,
    pub qoder_scheduler: Mutex<Value>,
    pub applog: Arc<crate::applog::AppLog>,
    /// Unified check-in scheduler status (last/next run, per-platform results).
    pub checkin: Mutex<Value>,
    /// Wakes the check-in scheduler after its settings change.
    pub checkin_wake: tokio::sync::Notify,
}

pub fn read_settings(path: &Path) -> Settings {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "无法创建数据目录")?;
    }
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut f = options.open(&tmp).map_err(|_| "无法写入设置")?;
        f.write_all(data).map_err(|_| "无法写入设置")?;
        f.sync_all().map_err(|_| "无法同步设置")?;
        std::fs::rename(&tmp, path).map_err(|_| "无法保存设置".to_owned())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

impl Hub {
    /// Open every provider under `dir` (created and migrated by `paths`).
    pub fn open(dir: &Path) -> Result<Shared, String> {
        qoder::qcore::set_data_dir(dir.join("qoder"));
        let settings_path = dir.join("settings.json");
        let existed = settings_path.exists();
        let mut settings = read_settings(&settings_path);
        let migrate_checkin_switches = existed && settings.version < 2;
        let key_path = dir.join("api-key");
        let key = match std::fs::read_to_string(&key_path) {
            Ok(k) if crate::api_key::validate(k.trim()).is_ok() => k.trim().to_owned(),
            _ => {
                let k = crate::api_key::generate();
                write_private(&key_path, k.as_bytes())?;
                k
            }
        };
        let wb = workbuddy::pool::AppState::new(dir.join("workbuddy/store.json"))?;
        let qoder = qoder::gateway::Provider::new()?;
        let trae = trae::Provider::open(&dir.join("trae"))?;
        if settings.upgrade() {
            if migrate_checkin_switches {
                // Carry the per-app automatic check-in switches into the unified schedule.
                settings.checkin.workbuddy = wb.store.lock().unwrap().auto_checkin;
                settings.checkin.qoder = qoder::qcore::load_settings()["autoClaim"]
                    .as_bool()
                    .unwrap_or(true);
            }
            // Upgraded values still apply in memory if the file cannot be rewritten.
            let _ = write_private(&settings_path, &serde_json::to_vec_pretty(&settings).unwrap());
        }
        let applog = crate::applog::AppLog::install(dir, settings.logging);
        let logs = Arc::new(LogStore::open(dir));
        let sink_logs = logs.clone();
        *wb.log_sink.lock().unwrap() = Some(Arc::new(move |log: &workbuddy::pool::RequestLog| {
            sink_logs.push(from_workbuddy(log));
        }));
        Ok(Self::wire_callbacks(Arc::new(Self {
            dir: dir.to_owned(),
            settings: Mutex::new(settings),
            settings_writer: Mutex::new(()),
            account_writer: Mutex::new(()),
            key: crate::api_key::LocalKey::new(key),
            wb,
            qoder,
            trae,
            runtime: Mutex::default(),
            logs,
            active: Mutex::default(),
            server: Mutex::new(None),
            error: Mutex::new(String::new()),
            qoder_scheduler: Mutex::new(json!({"lastRun":null,"lastResults":[]})),
            applog,
            checkin: Mutex::new(json!({})),
            checkin_wake: tokio::sync::Notify::new(),
        })))
    }
    /// Test assembly with fixture providers and no migration.
    #[cfg(test)]
    pub fn assemble(
        dir: &Path,
        settings: Settings,
        key: &str,
        qoder: Arc<qoder::gateway::Provider>,
        trae: Arc<trae::Provider>,
    ) -> Shared {
        let wb = workbuddy::pool::AppState::new(dir.join("workbuddy/store.json")).unwrap();
        Self::wire_callbacks(Arc::new(Self {
            dir: dir.to_owned(),
            settings: Mutex::new(settings),
            settings_writer: Mutex::new(()),
            account_writer: Mutex::new(()),
            key: crate::api_key::LocalKey::new(key.into()),
            wb,
            qoder,
            trae,
            runtime: Mutex::default(),
            logs: Arc::new(LogStore::open(dir)),
            active: Mutex::default(),
            server: Mutex::new(None),
            error: Mutex::new(String::new()),
            qoder_scheduler: Mutex::new(json!({})),
            applog: crate::applog::AppLog::new(dir),
            checkin: Mutex::new(json!({})),
            checkin_wake: tokio::sync::Notify::new(),
        }))
    }
    fn wire_callbacks(hub: Shared) -> Shared {
        hub.applog.restore_trae_events(&hub.trae.store.snapshot()["events"]);
        let weak = Arc::downgrade(&hub);
        *hub.wb.login_status_sink.lock().unwrap() = Some(Arc::new(move |id, failed, revision| {
            if let Some(hub) = weak.upgrade() {
                if failed { hub.disable_account_auth_observed(ProviderKind::Workbuddy, id, revision); }
                else if hub.restore_login_observed(ProviderKind::Workbuddy, id, revision).is_err() {
                    agentdock_core::applog::warn("accounts", || ("登录验证成功，但保存账号恢复状态失败".into(), json!({"accountId":id})));
                }
            }
        }));
        let weak = Arc::downgrade(&hub);
        *hub.trae.store.login_sink.lock().unwrap() = Some(Arc::new(move |id, revision| {
            if let Some(hub) = weak.upgrade() {
                if hub.restore_login_observed(ProviderKind::Trae, id, Some(revision)).is_err() {
                    agentdock_core::applog::warn("accounts", || ("登录已更新，但保存账号恢复状态失败".into(), json!({"provider":"trae","accountId":id})));
                }
            }
        }));
        let ids: Vec<_> = {
            let store = hub.wb.store.lock().unwrap();
            store.accounts.iter().filter(|a| store.resources.get(&a.id).is_some_and(|c| c["authFailed"] == true)).map(|a| a.id.clone()).collect()
        };
        for id in ids {
            if !hub.settings().auth_disabled_accounts.contains(&format!("workbuddy/{id}")) {
                hub.disable_account_auth(ProviderKind::Workbuddy, &id);
            }
        }
        hub
    }
    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }
    pub fn running(&self) -> bool {
        self.server.lock().unwrap().is_some()
    }
    pub fn api_key(&self) -> String {
        self.key.get()
    }
    pub fn set_api_key(&self, value: String, expected: &str) -> Result<(), String> {
        self.key.replace(&self.dir.join("api-key"), value, expected)
    }
    /// Validate, persist, then publish. Port changes require a stopped gateway.
    pub fn update_settings(&self, change: impl FnOnce(&mut Settings) -> Result<(), String>) -> Result<Settings, String> {
        let _w = self.settings_writer.lock().unwrap();
        let mut next = self.settings();
        let previous = next.clone();
        change(&mut next)?;
        next.validate()?;
        if next.port != previous.port && self.running() {
            return Err("请先停止服务再修改端口".into());
        }
        write_private(
            &self.dir.join("settings.json"),
            &serde_json::to_vec_pretty(&next).unwrap(),
        )?;
        *self.settings.lock().unwrap() = next.clone();
        agentdock_core::applog::set_enabled(next.logging);
        if next.checkin != previous.checkin {
            // Keep the per-app switches (WorkBuddy store, Qoder settings) in step.
            if self.wb.store.lock().unwrap().auto_checkin != next.checkin.workbuddy {
                let _ = self.wb.set_auto_checkin(next.checkin.workbuddy);
            }
            let mut q = qoder::qcore::load_settings();
            if q["autoClaim"].as_bool() != Some(next.checkin.qoder) {
                q["autoClaim"] = json!(next.checkin.qoder);
                qoder::qcore::save_settings(&q);
            }
        }
        if next.checkin != previous.checkin || next.timezone != previous.timezone {
            self.checkin_wake.notify_one();
        }
        Ok(next)
    }

    pub fn upstream_models(&self) -> Vec<UpstreamModel> {
        let mut models = self.wb.upstream_models();
        models.extend(self.qoder.upstream_models());
        models.extend(self.trae.upstream_models());
        models
    }
    pub fn routes(&self, settings: &Settings) -> Vec<Route> {
        catalog::build(&self.upstream_models(), settings)
    }
    pub fn save_route(&self, route_id: &str, o: RouteOverride) -> Result<(), String> {
        let models = self.upstream_models();
        self.update_settings(|s| {
            catalog::validate_override(&models, s, route_id, &o)?;
            s.routes.insert(route_id.to_owned(), o);
            Ok(())
        })
        .map(|_| ())
    }
    pub fn reset_route(&self, route_id: &str) -> Result<(), String> {
        self.update_settings(|s| {
            s.routes.remove(route_id);
            Ok(())
        })
        .map(|_| ())
    }

    /// All accounts in global stable order: WorkBuddy, Qoder, Trae.
    pub fn pool_accounts(&self) -> Vec<PoolAccount> {
        let mut out = self.wb.pool_accounts();
        out.extend(self.qoder.pool_accounts());
        out.extend(self.trae.pool_accounts());
        let disabled = self.settings().auth_disabled_accounts;
        for account in &mut out {
            if disabled.contains(&account.key()) {
                account.enabled = false;
            }
        }
        out
    }
    pub fn eligibility(
        &self,
        account: &PoolAccount,
        target: &UpstreamModel,
        body: &Value,
        headers: &http::HeaderMap,
    ) -> Result<Exec, &'static str> {
        match account.provider {
            ProviderKind::Workbuddy => {
                let allow = self.wb.store.lock().unwrap().settings.allow_auxiliary_agents;
                workbuddy::official::purpose(headers, allow).map_err(|_| "辅助 Agent 未获允许")?;
                self.wb
                    .chat_eligibility(&account.id, &target.id, body)
                    .map(Exec::Workbuddy)
            }
            ProviderKind::Qoder => self
                .qoder
                .chat_eligibility(&account.id, &target.id)
                .map(Exec::Qoder),
            ProviderKind::Trae => {
                trae::gateway::validate(body)?;
                self.trae
                    .chat_eligibility(&account.id, &target.id)
                    .map(Exec::Trae)
            }
        }
    }
    /// Resolve an `x-account-id` value: a full key or an unambiguous provider-local id.
    pub fn resolve_account(&self, value: &str) -> Result<String, String> {
        let accounts = self.pool_accounts();
        if accounts.iter().any(|a| a.key() == value) {
            return Ok(value.to_owned());
        }
        let matches: Vec<_> = accounts.iter().filter(|a| a.id == value).collect();
        match matches.len() {
            1 => Ok(matches[0].key()),
            0 => Err("指定账号不存在或已移除；未向上游发送请求".into()),
            _ => Err("指定账号 ID 在多个端重复，请使用「端/账号 ID」格式".into()),
        }
    }
    pub fn set_account_enabled(&self, provider: ProviderKind, id: &str, enabled: bool) -> Result<(), String> {
        let _writer = self.account_writer.lock().unwrap();
        self.set_provider_enabled(provider, id, enabled)?;
        if enabled {
            // Re-enabling acknowledges a repaired login.
            let key = agentdock_core::provider::account_key(provider, id);
            if self.settings().auth_disabled_accounts.contains(&key) {
                self.update_settings(|s| {
                    s.auth_disabled_accounts.remove(&key);
                    Ok(())
                })?;
            }
            let mut rt = self.runtime.lock().unwrap();
            rt.auth_failed.remove(&key);
            rt.cooldown.remove(&key);
        }
        Ok(())
    }
    /// Called only after completed OAuth or a successful authenticated query.
    /// Clear obsolete auth evidence without opting the account back into proxying.
    pub fn restore_workbuddy_login(&self, id: &str) -> Result<(), String> {
        self.restore_login_observed(ProviderKind::Workbuddy, id, None)
    }
    pub fn credential_revisions(&self, provider: ProviderKind) -> HashMap<String, u64> {
        let accounts = match provider {
            ProviderKind::Workbuddy => self.wb.pool_accounts(),
            ProviderKind::Qoder => self.qoder.pool_accounts(),
            ProviderKind::Trae => self.trae.pool_accounts(),
        };
        accounts.into_iter().filter_map(|a| self.credential_revision(provider, &a.id).map(|r| (a.id, r))).collect()
    }
    /// Imports are not authenticated queries. Only replacement credentials with
    /// a future expiry can invalidate an earlier failure; reimporting an old
    /// backup must not dismiss it.
    pub fn restore_changed_logins(&self, provider: ProviderKind, before: &HashMap<String, u64>) -> Result<(), String> {
        if provider == ProviderKind::Workbuddy { self.wb.refresh_login_metadata(); }
        let now = chrono::Utc::now().timestamp_millis();
        let accounts = match provider {
            ProviderKind::Workbuddy => self.wb.pool_accounts(),
            ProviderKind::Qoder => self.qoder.pool_accounts(),
            ProviderKind::Trae => self.trae.pool_accounts(),
        };
        for a in accounts {
            if !a.expires_at.is_some_and(|exp| exp > now) { continue; }
            if let Some(revision) = self.credential_revision(provider, &a.id) {
                if before.get(&a.id).is_some_and(|old| *old != revision) {
                    self.restore_login_observed(provider, &a.id, Some(revision))?;
                }
            }
        }
        Ok(())
    }
    fn credential_revision(&self, provider: ProviderKind, id: &str) -> Option<u64> {
        let token = match provider {
            ProviderKind::Workbuddy => {
                let account = self.wb.store.lock().unwrap().accounts.iter().find(|a| a.id == id).cloned();
                account.and_then(|a| workbuddy::pool::credentials(&a).ok()).map(|c| c.token)
            }
            ProviderKind::Qoder => self.qoder.accounts().into_iter().find(|a| a.id() == id).map(|a| a.token),
            ProviderKind::Trae => self.trae.store.account(id).ok().and_then(|a| a["user"]["token"].as_str().map(str::to_owned)),
        };
        token.filter(|t| !t.is_empty()).map(|t| agentdock_core::provider::credential_revision(&t))
    }
    fn current_credential_revision(&self, provider: ProviderKind, id: &str, revision: Option<u64>) -> bool {
        revision.is_none_or(|expected| self.credential_revision(provider, id) == Some(expected))
    }
    pub fn restore_login_observed(&self, provider: ProviderKind, id: &str, revision: Option<u64>) -> Result<(), String> {
        let _writer = self.account_writer.lock().unwrap();
        if !self.current_credential_revision(provider, id, revision) { return Ok(()); }
        let key = agentdock_core::provider::account_key(provider, id);
        let suspended = self.settings().auth_disabled_accounts.contains(&key)
            || self.runtime.lock().unwrap().auth_failed.contains(&key);
        let provider_failed = if provider == ProviderKind::Workbuddy {
            // Never hold runtime while acquiring store: snapshots take store
            // before runtime. Keep the two reads in separate lock scopes.
            let runtime_failed = self.wb.runtime.lock().unwrap().auth_failed.get(id).copied().unwrap_or(false);
            let cached_failed = self.wb.store.lock().unwrap().resources.get(id).is_some_and(|c| c["authFailed"] == true);
            runtime_failed || cached_failed
        } else { false };
        if !suspended && !provider_failed { return Ok(()); }
        if suspended { self.set_provider_enabled(provider, id, false)?; }
        if provider == ProviderKind::Workbuddy { self.wb.clear_login_failure(id)?; }
        if self.settings().auth_disabled_accounts.contains(&key) {
            self.update_settings(|s| { s.auth_disabled_accounts.remove(&key); Ok(()) })?;
        }
        let mut rt = self.runtime.lock().unwrap();
        rt.auth_failed.remove(&key);
        rt.cooldown.remove(&key);
        Ok(())
    }
    fn set_provider_enabled(&self, provider: ProviderKind, id: &str, enabled: bool) -> Result<(), String> {
        match provider {
            ProviderKind::Workbuddy => self.wb.set_enabled(id, enabled)?,
            ProviderKind::Qoder => self.qoder.set_enabled(id, enabled)?,
            ProviderKind::Trae => {
                self.trae
                    .update_account(id, &json!({"enabled": enabled}))?;
            }
        }
        Ok(())
    }
    /// Exclude immediately, then persist both the reason and the provider switch.
    /// Persistence errors never replace the original upstream failure.
    pub fn disable_account_auth(&self, provider: ProviderKind, id: &str) {
        self.disable_account_auth_observed(provider, id, None);
    }
    pub fn disable_account_auth_observed(&self, provider: ProviderKind, id: &str, revision: Option<u64>) {
        let _writer = self.account_writer.lock().unwrap();
        if !self.current_credential_revision(provider, id, revision) { return; }
        self.disable_account_auth_locked(provider, id);
    }
    pub fn disable_expired_account(&self, provider: ProviderKind, id: &str) {
        let _writer = self.account_writer.lock().unwrap();
        if provider == ProviderKind::Workbuddy { self.wb.refresh_login_metadata(); }
        let current = self.pool_accounts().into_iter().find(|a| a.provider == provider && a.id == id);
        if current.is_some_and(|a| a.enabled && (a.expires_at.is_some_and(|at| at <= chrono::Utc::now().timestamp_millis()) || a.blocked.is_some())) {
            self.disable_account_auth_locked(provider, id);
        }
    }
    fn disable_account_auth_locked(&self, provider: ProviderKind, id: &str) {
        let key = agentdock_core::provider::account_key(provider, id);
        self.runtime.lock().unwrap().auth_failed.insert(key.clone());
        let saved = self.update_settings(|s| {
            s.auth_disabled_accounts.insert(key.clone());
            Ok(())
        });
        let switched = self.set_provider_enabled(provider, id, false);
        if saved.is_err() || switched.is_err() {
            agentdock_core::applog::error("gateway.auth", || (
                "账号登录已失效，已排除调度；保存自动停用状态失败".into(),
                json!({"provider": provider.key(), "accountId": id}),
            ));
        } else {
            agentdock_core::applog::warn("gateway.auth", || (
                "账号登录已过期或鉴权失效，已自动停用反代；修复登录后可手动开启".into(),
                json!({"provider": provider.key(), "accountId": id}),
            ));
        }
    }
    /// Runtime state per account key for the UI.
    pub fn account_runtime(&self) -> Value {
        let settings = self.settings();
        let rt = self.runtime.lock().unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let mut out = serde_json::Map::new();
        for a in self.pool_accounts() {
            let key = a.key();
            out.insert(key.clone(), json!({
                "inFlight": rt.in_flight.get(&key).copied().unwrap_or(0),
                "concurrency": settings.scope(a.provider).concurrency,
                "cooldownUntil": rt.cooldown.get(&key).filter(|t| **t > now),
                "authFailed": rt.auth_failed.contains(&key) || settings.auth_disabled_accounts.contains(&key),
                "enabled": a.enabled && !rt.auth_failed.contains(&key),
                "credit": a.credit,
                "expiresAt": a.expires_at,
                "blocked": a.blocked,
                "name": a.name,
                "region": a.region,
                "provider": a.provider,
                "id": a.id,
            }));
        }
        Value::Object(out)
    }
}

/// WorkBuddy management records (catalog, credits, check-in) in the unified log.
fn from_workbuddy(log: &workbuddy::pool::RequestLog) -> RequestLog {
    RequestLog {
        id: log.id.clone(),
        time: log.time.clone(),
        protocol: log.details.protocol.clone(),
        path: log.details.client_path.clone(),
        requested_model: log.model.clone(),
        model: log.model.clone(),
        real_model: log.real_model.clone(),
        provider: "workbuddy".into(),
        region: workbuddy::pool::unified_region(&log.account_variant).into(),
        account_id: log.account_id.clone(),
        account_name: log.account_name.clone(),
        status: log.status,
        outcome: log.outcome.clone(),
        duration_ms: log.duration_ms,
        first_byte_ms: log.details.first_byte_ms,
        usage: log.details.usage.clone(),
        message: log.details.error["message"].as_str().unwrap_or("").to_owned(),
        details: serde_json::to_value(&log.details).unwrap_or_default(),
        ..RequestLog::default()
    }
}
