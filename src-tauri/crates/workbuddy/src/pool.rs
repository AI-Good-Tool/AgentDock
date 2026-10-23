use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
use uuid::Uuid;

pub type Shared = Arc<AppState>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub source_id: String,
    pub variant: String,
    pub enabled: bool,
    #[serde(default)]
    pub checkin_enabled: bool,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub display_field: String,
    pub expires_at: Option<i64>,
}

/// Switch credentials may be renewed independently of the imported account
/// metadata. Until the read-only monitor has scanned the source, its old
/// imported expiry must not permanently disable a freshly renewed login.
fn login_expiry(account: &Account, presentation: Option<&Value>) -> Option<i64> {
    if account.source == "switch" {
        presentation.and_then(|p| p["expiresAt"].as_i64())
    } else {
        account.expires_at
    }
}

fn restore_credit_auth_failures(store: &mut Store) {
    for credit in store.resources.values_mut() {
        // Older read-only queries kept only this normalized HTML error title.
        // Recover that explicit 401 without treating arbitrary stale data as expiry.
        let legacy_401 = credit["error"].as_str()
            .and_then(|e| e.strip_prefix("服务端返回 HTML 错误页："))
            .is_some_and(|title| title == "401" || title.starts_with("401 "));
        if credit.get("authFailed").is_none() && (credit["stale"] == true || credit["ok"] == false) && legacy_401 {
            credit["authFailed"] = json!(true);
        }
    }
}

fn apply_login_metadata(accounts: &[Account], records: &[Value], rt: &mut Runtime) {
    for account in accounts.iter().filter(|a| a.source == "switch") {
        let raw = records.iter().find(|r| r["id"] == account.source_id);
        let presentation = rt.presentations.entry(account.id.clone()).or_insert_with(|| json!({}));
        presentation["expiresAt"] = raw.and_then(|r| r.get("expiresAt")).filter(|v| v.is_i64()).cloned().unwrap_or(Value::Null);
        presentation["needsRelogin"] = json!(raw.is_some_and(|r| r["needs_relogin"] == true));
        if let Some(raw) = raw { presentation["phoneNumber"] = json!(wb_switch_core::modules::account::profile_phone_number(raw)); }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLog {
    pub id: String,
    pub time: String,
    pub model: String,
    pub real_model: String,
    pub account_id: String,
    pub account_name: String,
    pub account_variant: String,
    pub status: u16,
    pub duration_ms: u128,
    pub outcome: String,
    pub details: crate::diagnostics::RequestDetails,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Store {
    #[serde(default)]
    pub account_priority: crate::account_priority::AccountPriority,
    #[serde(default)]
    pub settings: crate::settings::Settings,
    pub accounts: Vec<Account>,
    pub models: Option<Vec<Value>>,
    pub active_models: Option<Vec<String>>,
    pub catalog_variant: Option<String>,
    #[serde(default)]
    pub model_catalogs: HashMap<String, crate::model_registry::Catalog>,
    #[serde(default)]
    pub model_policies: HashMap<String, crate::model_registry::Policy>,
    #[serde(default)]
    pub auto_checkin: bool,
    #[serde(default)]
    pub checkin_config: crate::checkin::Config,
    #[serde(default)]
    pub checkin_records: Vec<crate::checkin::Record>,
    #[serde(default)]
    pub checkin_states: HashMap<String, crate::checkin::AccountState>,
    #[serde(default)]
    pub resources: HashMap<String, Value>,
    #[serde(default)]
    pub model_limits: HashMap<String, Vec<ModelLimit>>,
    #[serde(default)]
    pub credit_snapshots: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelLimit {
    pub model: Option<String>,
    pub reset_at: i64,
    pub first_seen_at: i64,
    pub source: String,
}

#[derive(Default)]
pub struct Runtime {
    pub account_refresh: crate::settings::RefreshStatus,
    pub cursor: usize,
    pub in_flight: HashMap<String, usize>,
    pub management_in_flight: HashMap<String, usize>,
    pub cooldown: HashMap<String, i64>,
    pub auth_failed: HashMap<String, bool>,
    pub logs: Vec<RequestLog>,
    pub running: bool,
    pub checkins: HashMap<String, Value>,
    pub checkin_times: HashMap<String, i64>,
    pub checkin_errors: HashMap<String, String>,
    pub checkin_running: std::collections::HashSet<String>,
    pub auto_checkin_attempts: HashMap<String, crate::checkin::Attempt>,
    pub external_limits: HashMap<String, Vec<ModelLimit>>,
    pub presentations: HashMap<String, Value>,
    pub clients: Value,
}

pub struct AppState {
    pub conversations: Mutex<crate::conversation::Sessions>,
    pub official_transport: crate::official_transport::Transport,
    pub refresh_gate: tokio::sync::Mutex<()>,
    pub renewal: crate::auth::Renewal,
    pub store: Mutex<Store>,
    store_write: Mutex<()>,
    pub runtime: Mutex<Runtime>,
    pub client: reqwest::Client,
    pub path: PathBuf,
    /// Receives every finished management/catalog record for the unified log.
    pub log_sink: Mutex<Option<LogSink>>,
    /// Report confirmed authentication failure/recovery from management queries.
    pub login_status_sink: Mutex<Option<LoginStatusSink>>,
}
pub type LogSink = Arc<dyn Fn(&RequestLog) + Send + Sync>;
pub type LoginStatusSink = Arc<dyn Fn(&str, bool, Option<u64>) + Send + Sync>;

fn home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "无法定位用户目录".into())
}
pub fn switch_file() -> Result<PathBuf, String> {
    Ok(home()?.join(".wb-switch/accounts.json"))
}
pub fn auth_file(variant: &str) -> Result<PathBuf, String> {
    let home = home()?;
    let root = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(target_os = "windows") {
        home.join("AppData/Local")
    } else {
        home.join(".local/share")
    };
    Ok(root
        .join("CodeBuddyExtension/Data/Public/auth")
        .join(if variant == "ai" {
            "workbuddy-desktop-ai.info"
        } else {
            "workbuddy-desktop.info"
        }))
}

pub fn read_json(path: &PathBuf) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|_| "无法读取本地账号文件".to_string())?;
    serde_json::from_str(&text).map_err(|_| "账号文件不是有效 JSON".into())
}

// Own metadata only: tokens continue to reside in the user's existing login files.
fn save(path: &PathBuf, store: &Store) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(store).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}

impl AppState {
    // Serialize metadata writers, not routing readers. Disk I/O must never hold
    // the store/runtime locks; publish only after a successful atomic save.
    pub fn update_store<T>(
        &self,
        change: impl FnOnce(&mut Store) -> Result<T, String>,
    ) -> Result<T, String> {
        let _writer = self.store_write.lock().unwrap();
        let mut next = self.store.lock().unwrap().clone();
        let result = change(&mut next)?;
        save(&self.path, &next)?;
        *self.store.lock().unwrap() = next;
        Ok(result)
    }

    pub fn set_account_priority(
        &self,
        priority: crate::account_priority::AccountPriority,
    ) -> Result<(), String> {
        self.update_store(|store| {
            store.account_priority = priority;
            Ok(())
        })
    }
    pub fn log_rejection(
        &self,
        model: &str,
        status: u16,
        outcome: &str,
        details: crate::diagnostics::RequestDetails,
    ) {
        let (real, alias) = self
            .resolve_model(model)
            .unwrap_or_else(|| (model.into(), model.into()));
        let mut rt = self.runtime.lock().unwrap();
        let log = RequestLog {
            id: Uuid::new_v4().simple().to_string(),
            time: Utc::now().to_rfc3339(),
            model: crate::diagnostics::bounded(
                details.request_summary["modelAlias"]
                    .as_str()
                    .unwrap_or(&alias),
                128,
            ),
            real_model: crate::diagnostics::bounded(&real, 128),
            account_id: String::new(),
            account_name: String::new(),
            account_variant: String::new(),
            status,
            duration_ms: 0,
            outcome: outcome.into(),
            details,
        };
        rt.logs.insert(0, log.clone());
        rt.logs.truncate(200);
        drop(rt);
        self.emit_log(&log);
    }
    pub fn emit_log(&self, log: &RequestLog) {
        let sink = self.log_sink.lock().unwrap().clone();
        if let Some(sink) = sink {
            sink(log);
        }
    }
    pub fn update_display(&self, id: &str, note: String, field: String) -> Result<(), String> {
        if !["nickname", "phone", "note"].contains(&field.as_str()) {
            return Err("显示字段不受支持".into());
        }
        if note.chars().count() > 100 {
            return Err("备注最多 100 字".into());
        }
        self.update_store(|store| {
            let a = store
                .accounts
                .iter_mut()
                .find(|a| a.id == id)
                .ok_or("账号不存在")?;
            a.note = note;
            a.display_field = field;
            Ok(())
        })
    }
    pub fn set_checkin_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        self.update_store(|store| {
            let account = store
                .accounts
                .iter_mut()
                .find(|a| a.id == id)
                .ok_or("账号不存在")?;
            if account.variant != "cn" && enabled {
                return Err("国际版不支持签到".into());
            }
            account.checkin_enabled = enabled;
            Ok(())
        })
    }

    pub fn cache_credit(&self, id: &str, credit: Value) -> Result<(), String> {
        self.cache_credit_inner(id, credit, None)
    }
    pub fn cache_credit_observed(&self, id: &str, credit: Value, revision: u64) -> Result<bool, String> {
        self.cache_credit_observed_at(id, credit, revision, &switch_file()?)
    }
    fn cache_credit_observed_at(&self, id: &str, credit: Value, revision: u64, path: &std::path::Path) -> Result<bool, String> {
        let account = self.store.lock().unwrap().accounts.iter().find(|a| a.id == id).cloned().ok_or("账号不存在")?;
        // Serialize the credential check and its state update with credential writers.
        // Hub callbacks never acquire this vault lock, so lock order stays one-way.
        let _vault = if account.source == "switch" {
            Some(wb_switch_core::modules::account::lock_accounts_at(path).map_err(|_| "无法锁定账号库")?)
        } else { None };
        let current = if account.source == "switch" {
            read_json(&path.to_path_buf())?.as_array().and_then(|rows| rows.iter().find(|r| r["id"] == account.source_id && r["uid"].as_str() == account.id.split_once(':').map(|(_, uid)| uid))).cloned()
                .and_then(|r| r["access_token"].as_str().map(agentdock_core::provider::credential_revision))
        } else { credentials(&account).ok().map(|c| agentdock_core::provider::credential_revision(&c.token)) };
        if current != Some(revision) { return Ok(false); }
        self.cache_credit_inner(id, credit, Some(revision))?;
        Ok(true)
    }
    fn cache_credit_inner(&self, id: &str, credit: Value, revision: Option<u64>) -> Result<(), String> {
        let auth_failed = credit["authFailed"] == true;
        let authenticated = credit["ok"] == true;
        if auth_failed { self.runtime.lock().unwrap().auth_failed.insert(id.into(), true); }
        let result = self.update_store(|store| {
            if credit["ok"] == true {
                if let Some(account) = store.accounts.iter().find(|a| a.id == id).cloned() {
                    crate::statistics::observe_credit(
                        &mut store.credit_snapshots,
                        &account,
                        &credit,
                    );
                }
            }
            let value = if credit["ok"] == false {
                if let Some(old) = store.resources.get(id).filter(|v| v["ok"] == true) {
                    let mut cached = old.clone();
                    cached["stale"] = json!(true);
                    cached["error"] = credit["error"].clone();
                    if auth_failed { cached["authFailed"] = json!(true); }
                    cached
                } else {
                    credit
                }
            } else {
                credit
            };
            store.resources.insert(id.into(), value);
            Ok(())
        });
        // Run after releasing metadata locks; the hub reconciles login state.
        if auth_failed || (authenticated && result.is_ok()) {
            let sink = self.login_status_sink.lock().unwrap().clone();
            if let Some(sink) = sink { sink(id, auth_failed, revision); }
        }
        result
    }

    /// A completed OAuth login or authenticated query repairs login state,
    /// independently of the user's proxy switch and cached balance.
    pub fn clear_login_failure(&self, id: &str) -> Result<(), String> {
        self.update_store(|store| {
            if !store.accounts.iter().any(|a| a.id == id) { return Err("账号不存在".into()); }
            if let Some(credit) = store.resources.get_mut(id).filter(|c| c.is_object()) {
                if credit["authFailed"] == true {
                    credit.as_object_mut().unwrap().remove("error");
                    credit.as_object_mut().unwrap().remove("errorAt");
                }
                credit["authFailed"] = json!(false);
            }
            Ok(())
        })?;
        let mut rt = self.runtime.lock().unwrap();
        rt.auth_failed.remove(id);
        rt.cooldown.remove(id);
        if let Some(presentation) = rt.presentations.get_mut(id) { presentation["needsRelogin"] = json!(false); }
        Ok(())
    }

    pub fn record_model_limit(
        &self,
        id: &str,
        model: Option<String>,
        reset_at: i64,
    ) -> Result<(), String> {
        let now = Utc::now().timestamp_millis();
        if reset_at <= now {
            return Ok(());
        }
        self.update_store(|store| {
            let limits = store.model_limits.entry(id.into()).or_default();
            limits.retain(|l| l.reset_at > now);
            if let Some(old) = limits.iter_mut().find(|l| l.model == model) {
                old.reset_at = old.reset_at.max(reset_at);
            } else {
                limits.push(ModelLimit {
                    model,
                    reset_at,
                    first_seen_at: now,
                    source: "proxy".into(),
                });
            }
            Ok(())
        })
    }
    pub fn set_auto_checkin(&self, enabled: bool) -> Result<(), String> {
        self.update_store(|store| {
            store.auto_checkin = enabled;
            Ok(())
        })
    }
    pub fn set_settings(
        &self,
        settings: crate::settings::Settings,
        confirmed: bool,
    ) -> Result<(), String> {
        settings.validate()?;
        self.update_store(|store| {
            if settings.use_device_sdk && !store.settings.use_device_sdk && !confirmed {
                return Err("开启设备 SDK 需确认可能触发系统钥匙串授权".into());
            }
            if settings.account_refresh_enabled && settings != store.settings && !confirmed {
                return Err("开启或修改定时刷新需确认官方只读查询的账号范围与频率".into());
            }
            let plan_changed = store.settings.account_refresh_enabled
                != settings.account_refresh_enabled
                || store.settings.account_refresh_minutes != settings.account_refresh_minutes;
            store.settings = settings;
            Ok(plan_changed)
        })
        .map(|plan_changed| {
            if plan_changed {
                self.runtime.lock().unwrap().account_refresh.next_at = None;
            }
        })
    }
    pub fn new(path: PathBuf) -> Result<Shared, String> {
        let mut store = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|_| "元数据文件损坏，请检查本地 store.json".to_string())?
        } else {
            Store::default()
        };
        restore_credit_auth_failures(&mut store);
        Ok(Arc::new(Self {
            conversations: Mutex::default(),
            official_transport: Default::default(),
            refresh_gate: tokio::sync::Mutex::new(()),
            renewal: crate::auth::Renewal::default(),
            store: Mutex::new(store),
            store_write: Mutex::new(()),
            runtime: Mutex::new(Runtime::default()),
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .timeout(std::time::Duration::from_secs(180))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())?,
            path,
            log_sink: Mutex::new(None),
            login_status_sink: Mutex::new(None),
        }))
    }

    pub fn models(&self) -> Vec<Value> {
        crate::model_registry::models(&self.store.lock().unwrap())
    }
    pub fn active_models(&self) -> Vec<String> {
        crate::model_registry::active(&self.store.lock().unwrap())
    }
    pub fn resolve_model(&self, name: &str) -> Option<(String, String)> {
        crate::model_registry::resolve(&self.store.lock().unwrap(), name)
    }
    pub fn model_config(&self, id: &str, region: &str) -> Option<Value> {
        crate::model_registry::config(&self.store.lock().unwrap(), id, region)
    }
    pub fn set_model_policy(
        &self,
        id: String,
        policy: crate::model_registry::Policy,
    ) -> Result<(), String> {
        self.update_store(|store| {
            crate::model_registry::validate_policy(store, &id, &policy)?;
            store.model_policies.insert(id, policy);
            Ok(())
        })
    }

    pub fn import_accounts(&self, source: &str) -> Result<usize, String> {
        self.import_account_source(source, None)
    }
    pub fn import_account_source(
        &self,
        source: &str,
        only_id: Option<&str>,
    ) -> Result<usize, String> {
        self.import_source_at(source, only_id, &switch_file()?)
    }
    pub(crate) fn import_source_at(&self, source: &str, only_id: Option<&str>, path: &PathBuf) -> Result<usize, String> {
        let mut added = vec![];
        if source == "switch" {
            let data = read_json(path)?;
            let list = data.as_array().ok_or("参考账号库格式不受支持")?;
            for raw in list {
                if only_id.is_some_and(|id| raw["id"].as_str() != Some(id)) {
                    continue;
                }
                if let (Some(source_id), Some(uid)) = (
                    raw.get("id").and_then(Value::as_str),
                    raw.get("uid").and_then(Value::as_str),
                ) {
                    let variant = if raw
                        .get("domain")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .ends_with(".workbuddy.ai")
                        || raw["variant"] == "ai"
                    {
                        "ai"
                    } else {
                        "cn"
                    };
                    added.push(Account {
                        id: format!("{variant}:{uid}"),
                        name: raw
                            .get("nickname")
                            .and_then(Value::as_str)
                            .unwrap_or("未命名账号")
                            .into(),
                        source: "switch".into(),
                        source_id: source_id.into(),
                        variant: variant.into(),
                        enabled: true,
                        checkin_enabled: false,
                        note: String::new(),
                        display_field: String::new(),
                        expires_at: raw.get("expiresAt").and_then(Value::as_i64),
                    });
                }
            }
        } else if source == "local" {
            let data = read_json(&auth_file("cn")?)?;
            let uid = data["account"]["uid"].as_str().ok_or("没有当前登录账号")?;
            added.push(Account {
                id: format!("cn:{uid}"),
                name: data["account"]["nickname"]
                    .as_str()
                    .unwrap_or("当前账号")
                    .into(),
                source: "local".into(),
                source_id: uid.into(),
                variant: "cn".into(),
                enabled: true,
                checkin_enabled: false,
                note: String::new(),
                display_field: String::new(),
                expires_at: data["auth"]["expiresAt"].as_i64(),
            });
        } else {
            return Err("不受支持的导入来源".into());
        }
        let n = added.len();
        self.update_store(|store| {
            for account in added {
                if let Some(old) = store.accounts.iter_mut().find(|a| a.id == account.id) {
                    let enabled = old.enabled;
                    let checkin_enabled = old.checkin_enabled;
                    let note = old.note.clone();
                    let display_field = old.display_field.clone();
                    *old = account;
                    old.enabled = enabled;
                    old.checkin_enabled = checkin_enabled;
                    old.note = note;
                    old.display_field = display_field;
                } else {
                    store.accounts.push(account);
                }
            }
            Ok(n)
        })
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        self.update_store(|store| {
            let a = store
                .accounts
                .iter_mut()
                .find(|a| a.id == id)
                .ok_or("账号不存在")?;
            a.enabled = enabled;
            if enabled {
                if let Some(credit) = store.resources.get_mut(id) { credit["authFailed"] = json!(false); }
            }
            Ok(())
        })?;
        // Existing leases remain valid; only future acquisitions see the switch.
        // A user explicitly re-enabling an account acknowledges a repaired login.
        if enabled {
            self.runtime.lock().unwrap().auth_failed.remove(id);
        }
        Ok(())
    }

    pub fn update_models(
        &self,
        models: Vec<Value>,
        active_models: Vec<String>,
        variant: String,
    ) -> Result<(), String> {
        if !["cn", "ai"].contains(&variant.as_str()) {
            return Err("无效的模型目录地区".into());
        }
        let mut ids = std::collections::HashSet::new();
        if models.is_empty()
            || models.iter().any(|m| {
                m["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .is_none_or(|id| !ids.insert(id))
            })
        {
            return Err("目录包含空模型 ID、重复 ID 或无有效模型，保留旧目录".into());
        }
        self.update_store(|store| {
            if store.model_catalogs.is_empty() {
                store.model_catalogs = crate::model_registry::catalogs(store);
            }
            store.model_catalogs.insert(
                variant.clone(),
                crate::model_registry::Catalog {
                    models: models.clone(),
                    active: active_models.clone(),
                },
            );
            for (id, policy) in &store.model_policies {
                if crate::model_registry::models(store)
                    .iter()
                    .any(|m| m["id"] == *id)
                {
                    crate::model_registry::validate_policy(store, id, policy)
                        .map_err(|_| "新目录与现有模型别名冲突；请先编辑别名再同步")?;
                }
            }
            store.models = Some(models);
            store.active_models = Some(active_models);
            store.catalog_variant = Some(variant);
            Ok(())
        })
    }

    pub fn snapshot(&self) -> Value {
        let store = self.store.lock().unwrap();
        let rt = self.runtime.lock().unwrap();
        json!({ "accounts": store.accounts.iter().map(|a| {
            let mut v=serde_json::to_value(a).unwrap();
            v["expiresAt"]=json!(login_expiry(a, rt.presentations.get(&a.id)));
            v["inFlight"]=json!(rt.in_flight.get(&a.id).copied().unwrap_or(0));
            v["managementInFlight"]=json!(rt.management_in_flight.get(&a.id).copied().unwrap_or(0));
            v["cooldownUntil"]=json!(rt.cooldown.get(&a.id));
            v["authFailed"]=json!(rt.auth_failed.get(&a.id).copied().unwrap_or(false));
            let latest = store.checkin_records.iter().rev().find(|r| r.account_id == a.id);
            let success = store.checkin_records.iter().rev().find(|r| r.account_id == a.id && r.status.is_some());
            let state = store.checkin_states.get(&a.id);
            v["checkin"]=state.and_then(|s|s.status.clone()).or_else(||success.and_then(|r|r.status.clone())).unwrap_or(Value::Null);
            v["checkinUpdatedAt"]=json!(state.and_then(|s|s.updated_at).or_else(||success.map(|r|r.at)));
            v["checkinError"]=json!(state.and_then(|s|s.error.as_ref()).or_else(||if state.is_none(){latest.and_then(|r|r.error.as_ref())}else{None}));
            v["checkinRunning"]=json!(rt.checkin_running.contains(&a.id));
            v["checkinErrorAt"]=json!(state.and_then(|s|s.error_at).or_else(||if state.is_none(){latest.filter(|r|r.error.is_some()).map(|r|r.at)}else{None}));
            v["credit"]=store.resources.get(&a.id).cloned().unwrap_or(Value::Null);
            v["presentation"]=rt.presentations.get(&a.id).cloned().unwrap_or(Value::Null);
            v["modelLimits"]=json!(combined_limits(&store,&rt,&a.id,Utc::now().timestamp_millis()));v
        }).collect::<Vec<_>>(), "version": "5.8.3", "catalogVariant": store.catalog_variant.as_deref().unwrap_or("cn"), "autoCheckin":store.auto_checkin,"checkinConfig":store.checkin_config,"checkinRecords":store.checkin_records,"clients":rt.clients,
            "settings": store.settings, "accountPriority":store.account_priority, "accountRefresh": rt.account_refresh, "officialProfile": crate::official::profile_summary(store.settings.use_device_sdk) })
    }

    pub fn acquire(
        self: &Arc<Self>,
        model: &str,
        account_id: Option<&str>,
    ) -> Result<(Account, Lease), String> {
        self.acquire_for_body(model, account_id, None)
    }
    pub fn acquire_for_body(
        self: &Arc<Self>,
        model: &str,
        account_id: Option<&str>,
        body: Option<&Value>,
    ) -> Result<(Account, Lease), String> {
        let store = self.store.lock().unwrap();
        let mut rt = self.runtime.lock().unwrap();
        let now = Utc::now().timestamp_millis();
        let n = store.accounts.len();
        if account_id.is_some_and(|id| !store.accounts.iter().any(|a| a.id == id)) {
            return Err("指定账号不存在或已移除；未向官方发送请求".into());
        }
        let mut reasons = std::collections::BTreeMap::<&str, usize>::new();
        let management = management_request(model);
        let mut candidates = (0..n)
            .map(|offset| (rt.cursor + offset) % n)
            .collect::<Vec<_>>();
        if !management && account_id.is_none() {
            let ranks = store
                .accounts
                .iter()
                .map(|a| crate::account_priority::credit_rank(store.resources.get(&a.id), now))
                .collect::<Vec<_>>();
            // Stable sorting keeps round-robin fairness among equally ranked accounts.
            candidates.sort_by(|a, b| {
                crate::account_priority::compare(store.account_priority, ranks[*a], ranks[*b])
            });
        }
        for index in candidates {
            let a = &store.accounts[index];
            if account_id.is_some_and(|id| a.id != id) {
                continue;
            }
            let region_config = if management {
                None
            } else {
                crate::model_registry::config(&store, model, &a.variant)
            };
            let reason = if !management && region_config.is_none() {
                Some("地区不支持此模型")
            } else if !management
                && body.is_some_and(|b| {
                    !crate::model_registry::request_supported(b, region_config.as_ref().unwrap())
                })
            {
                Some("此地区模型不支持该请求")
            } else if !a.enabled && !["credits", "checkin"].contains(&model) {
                Some("未启用反代")
            } else if login_expiry(a, rt.presentations.get(&a.id)).is_some_and(|ts| ts <= now) && model != "credits" {
                Some("登录已过期")
            } else if rt.cooldown.get(&a.id).is_some_and(|ts| *ts > now) {
                Some("账号冷却中")
            } else if model != "credits" && rt.auth_failed.get(&a.id).copied().unwrap_or(false) {
                Some("鉴权已失败")
            } else if if management {
                rt.management_in_flight.get(&a.id).copied().unwrap_or(0) >= 1
            } else {
                rt.in_flight.get(&a.id).copied().unwrap_or(0) >= 2
            } {
                Some("并发已满")
            } else if !management_request(model)
                && combined_limits(&store, &rt, &a.id, now)
                    .iter()
                    .any(|l| l.model.as_deref() == Some(model))
            {
                Some("模型限额")
            } else {
                None
            };
            if let Some(reason) = reason {
                *reasons.entry(reason).or_default() += 1;
                continue;
            }
            if management {
                *rt.management_in_flight.entry(a.id.clone()).or_default() += 1;
            } else {
                *rt.in_flight.entry(a.id.clone()).or_default() += 1;
                rt.cursor = (index + 1) % n;
            }
            return Ok((
                a.clone(),
                Lease {
                    state: self.clone(),
                    account_id: a.id.clone(),
                    model: model.into(),
                    model_config: region_config,
                    id: Uuid::new_v4().simple().to_string(),
                    start: Instant::now(),
                    status: 499,
                    outcome: "cancelled".into(),
                    details: crate::diagnostics::RequestDetails::incoming(
                        "internal",
                        model,
                        &Value::Null,
                    ),
                    account_name: a.name.clone(),
                    account_variant: a.variant.clone(),
                },
            ));
        }
        if reasons.is_empty() {
            Err("没有可用账号：尚未导入账号".into())
        } else {
            Err(format!(
                "没有可用账号：{}（每个账号显示首个不满足条件）",
                reasons
                    .iter()
                    .map(|(reason, count)| format!("{reason} {count} 个"))
                    .collect::<Vec<_>>()
                    .join("；")
            ))
        }
    }
}

/// Unified regions: WorkBuddy's international edition is "ai" internally.
pub fn unified_region(variant: &str) -> &'static str {
    if variant == "ai" {
        "intl"
    } else {
        "cn"
    }
}
pub fn variant_of_region(region: &str) -> &'static str {
    if region == "intl" {
        "ai"
    } else {
        "cn"
    }
}

impl AppState {
    /// Read only the current credential metadata before expiry decisions.
    /// Client discovery stays on its slower monitor; no token is refreshed.
    pub fn refresh_login_metadata(&self) {
        let accounts = self.store.lock().unwrap().accounts.clone();
        if !accounts.iter().any(|a| a.source == "switch") { return; }
        let records = switch_file().ok().and_then(|path| read_json(&path).ok())
            .and_then(|v| v.as_array().cloned()).unwrap_or_default();
        apply_login_metadata(&accounts, &records, &mut self.runtime.lock().unwrap());
    }

    /// Accounts as seen by the unified scheduler. Ranking uses cached billing only.
    pub fn pool_accounts(&self) -> Vec<agentdock_core::provider::PoolAccount> {
        let store = self.store.lock().unwrap();
        let rt = self.runtime.lock().unwrap();
        let now = Utc::now().timestamp_millis();
        store
            .accounts
            .iter()
            .enumerate()
            .map(|(order, a)| {
                let credit = store.resources.get(&a.id);
                let (earliest, remaining) = crate::account_priority::credit_rank(credit, now);
                let name = rt
                    .presentations
                    .get(&a.id)
                    .and_then(|p| p["displayName"].as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&a.name)
                    .to_owned();
                agentdock_core::provider::PoolAccount {
                    provider: agentdock_core::provider::ProviderKind::Workbuddy,
                    id: a.id.clone(),
                    region: unified_region(&a.variant).into(),
                    name: if a.note.is_empty() || a.display_field != "note" {
                        name
                    } else {
                        a.note.clone()
                    },
                    enabled: a.enabled,
                    expires_at: login_expiry(a, rt.presentations.get(&a.id)),
                    credit: agentdock_core::provider::CreditRank {
                        remaining,
                        earliest_expiry: earliest,
                        unit: "credits".into(),
                        updated_at: credit.and_then(|c| c["updatedAt"].as_i64()),
                    },
                    order,
                    blocked: (rt.presentations.get(&a.id)
                        .is_some_and(|p| p["needsRelogin"] == true)
                        || credit.is_some_and(|c| c["authFailed"] == true)
                        || rt.auth_failed.get(&a.id).copied().unwrap_or(false))
                        .then(|| "登录凭证已失效".into()),
                }
            })
            .collect()
    }
    /// WorkBuddy-specific chat eligibility (region catalog, request capability,
    /// per-model limits). Concurrency, cooldown and auth isolation are owned by
    /// the unified gateway. Returns the exact region configuration on success.
    pub fn chat_eligibility(
        &self,
        account_id: &str,
        model: &str,
        body: &Value,
    ) -> Result<Value, &'static str> {
        let store = self.store.lock().unwrap();
        let rt = self.runtime.lock().unwrap();
        let a = store
            .accounts
            .iter()
            .find(|a| a.id == account_id)
            .ok_or("账号不存在")?;
        let config = crate::model_registry::region_config(&store, model, &a.variant)
            .ok_or("地区不支持此模型")?;
        if !crate::model_registry::request_supported(body, &config) {
            return Err("此地区模型不支持该请求");
        }
        let now = Utc::now().timestamp_millis();
        if combined_limits(&store, &rt, &a.id, now)
            .iter()
            .any(|l| l.model.as_deref() == Some(model))
        {
            return Err("模型限额");
        }
        Ok(config)
    }
    /// Region catalogs projected into the unified model list.
    pub fn upstream_models(&self) -> Vec<agentdock_core::provider::UpstreamModel> {
        let store = self.store.lock().unwrap();
        crate::model_registry::upstream_models(&store)
    }
}

fn management_request(model: &str) -> bool {
    ["catalog", "checkin", "credits"].contains(&model)
}

fn combined_limits(store: &Store, rt: &Runtime, id: &str, now: i64) -> Vec<ModelLimit> {
    let mut result: Vec<ModelLimit> = vec![];
    for item in store
        .model_limits
        .get(id)
        .into_iter()
        .flatten()
        .chain(rt.external_limits.get(id).into_iter().flatten())
        .filter(|l| l.reset_at > now)
    {
        if let Some(old) = result.iter_mut().find(|l| l.model == item.model) {
            if old.reset_at < item.reset_at {
                *old = item.clone();
            }
        } else {
            result.push(item.clone());
        }
    }
    result.sort_by_key(|l| l.reset_at);
    result
}

pub struct Lease {
    state: Shared,
    pub model_config: Option<Value>,
    pub account_id: String,
    pub model: String,
    pub id: String,
    start: Instant,
    pub status: u16,
    pub outcome: String,
    pub details: crate::diagnostics::RequestDetails,
    account_name: String,
    account_variant: String,
}

impl Lease {
    pub fn elapsed_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut rt = self.state.runtime.lock().unwrap();
        let counts = if management_request(&self.model) {
            &mut rt.management_in_flight
        } else {
            &mut rt.in_flight
        };
        if let Some(n) = counts.get_mut(&self.account_id) {
            *n = n.saturating_sub(1);
        }
        let log = RequestLog {
            id: self.id.clone(),
            time: Utc::now().to_rfc3339(),
            model: self.details.request_summary["modelAlias"]
                .as_str()
                .unwrap_or(&self.model)
                .into(),
            real_model: self.model.clone(),
            account_id: self.account_id.clone(),
            account_name: self.account_name.clone(),
            account_variant: self.account_variant.clone(),
            status: self.status,
            duration_ms: self.start.elapsed().as_millis(),
            outcome: self.outcome.clone(),
            details: self.details.clone(),
        };
        rt.logs.insert(0, log.clone());
        rt.logs.truncate(200);
        drop(rt);
        crate::statistics::record_usage(&self.state, &log);
        self.state.emit_log(&log);
    }
}

pub struct Credential {
    pub token: String,
    pub uid: String,
    pub domain: String,
    pub enterprise: Option<String>,
}

pub fn credentials(account: &Account) -> Result<Credential, String> {
    let raw = if account.source == "switch" {
        read_json(&switch_file()?)?
            .as_array()
            .ok_or("账号库格式错误")?
            .iter()
            .find(|r| r["id"] == account.source_id)
            .cloned()
            .ok_or("原始账号已移除，请重新导入")?
    } else if account.source == "local" {
        let login = read_json(&auth_file(&account.variant)?)?;
        let uid = login["account"]["uid"]
            .as_str()
            .ok_or("当前客户端已退出登录")?;
        if account.source_id != uid {
            return Err("客户端当前账号已改变，请重新导入".into());
        }
        let raw = json!({ "uid": uid, "access_token": login["auth"]["accessToken"], "domain": login["auth"]["domain"], "enterpriseId": login["account"]["enterpriseId"] });
        if !raw["access_token"].is_string() {
            return Err("本地登录态已加密，请点击“导入当前登录账号”读取本机新凭据".into());
        }
        raw
    } else { return Err("不支持的账号来源，请重新导入".into()); };
    if account.source == "switch" && (raw["uid"].as_str() != account.id.split_once(':').map(|(_, uid)| uid)
        || wb_switch_core::modules::account::variant_of(&raw).as_str() != account.variant) {
        return Err("原始账号身份或地区已变化，请重新导入，未使用其他账号的凭据".into());
    }
    let token = raw["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("该账号登录态不可直接使用，请在官方工具重新登录")?;
    Ok(Credential {
        token: token.into(),
        uid: raw["uid"].as_str().ok_or("缺少账号 UID")?.into(),
        domain: raw["domain"]
            .as_str()
            .unwrap_or(if account.variant == "ai" {
                "www.workbuddy.ai"
            } else {
                "www.workbuddy.cn"
            })
            .into(),
        enterprise: raw
            .get("enterpriseId")
            .or_else(|| raw.get("enterprise_id"))
            .and_then(Value::as_str)
            .filter(|x| !x.is_empty())
            .map(String::from),
    })
}

pub fn endpoint(account: &Account) -> &'static str {
    if account.variant == "ai" {
        "https://www.workbuddy.ai"
    } else {
        "https://www.workbuddy.cn"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_credit_results_cannot_replace_new_login_status_or_balance() {
        let dir = std::env::temp_dir().join(format!("agentdock-stale-credit-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap(); let path = dir.join("accounts.json");
        std::fs::write(&path, serde_json::to_vec(&json!([{"id":"source","uid":"u","variant":"cn","access_token":"fresh"}])).unwrap()).unwrap();
        let state = AppState::new(dir.join("metadata.json")).unwrap();
        state.import_source_at("switch", None, &path).unwrap();
        state.cache_credit("cn:u", json!({"ok":true,"totalRemaining":99})).unwrap();
        let old = agentdock_core::provider::credential_revision("old");
        assert!(!state.cache_credit_observed_at("cn:u",json!({"ok":false,"authFailed":true}),old,&path).unwrap());
        assert!(!state.cache_credit_observed_at("cn:u",json!({"ok":true,"totalRemaining":1}),old,&path).unwrap());
        assert_eq!(state.store.lock().unwrap().resources["cn:u"]["totalRemaining"],99);
        assert!(state.runtime.lock().unwrap().auth_failed.is_empty());
        let fresh = agentdock_core::provider::credential_revision("fresh");
        assert!(state.cache_credit_observed_at("cn:u",json!({"ok":true,"totalRemaining":100}),fresh,&path).unwrap());
        assert_eq!(state.store.lock().unwrap().resources["cn:u"]["totalRemaining"],100);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn legacy_stale_401_migration_does_not_override_manual_acknowledgement_or_network_failures() {
        let mut store = Store::default();
        store.resources.insert("auth".into(), json!({"ok":true,"stale":true,"error":"服务端返回 HTML 错误页：401 Authorization Required"}));
        store.resources.insert("acknowledged".into(), json!({"ok":true,"stale":true,"authFailed":false,"error":"服务端返回 HTML 错误页：401 Authorization Required"}));
        store.resources.insert("network".into(), json!({"ok":true,"stale":true,"error":"connection timeout"}));
        store.resources.insert("forbidden".into(), json!({"ok":true,"stale":true,"error":"服务端返回 HTML 错误页：403 Forbidden"}));
        restore_credit_auth_failures(&mut store);
        assert_eq!(store.resources["auth"]["authFailed"],true);
        assert_eq!(store.resources["acknowledged"]["authFailed"],false);
        assert!(store.resources["network"].get("authFailed").is_none());
        assert!(store.resources["forbidden"].get("authFailed").is_none());
    }
    fn state() -> Shared {
        let state = AppState::new(std::env::temp_dir().join(Uuid::new_v4().to_string()))
        .unwrap();
        state.store.lock().unwrap().accounts = vec![Account {
            id: "a".into(),
            name: "a".into(),
            source: "test".into(),
            source_id: "a".into(),
            variant: "cn".into(),
            enabled: true,
            checkin_enabled: false,
            note: String::new(),
            display_field: String::new(),
            expires_at: None,
        }];
        state
    }
    #[test]
    fn rejected_unknown_model_names_remain_bounded_in_logs() {
        let s = state();
        let name = "x".repeat(20_000);
        let details =
            crate::diagnostics::RequestDetails::incoming("fixture", "/fixture", &json!({}));
        s.log_rejection(&name, 400, "invalid_request", details);
        let rt = s.runtime.lock().unwrap();
        assert!(rt.logs[0].model.len() <= 140);
        assert!(rt.logs[0].real_model.len() <= 140);
    }
    #[test]
    fn shared_catalog_round_robin_and_exclusive_regions() {
        let s = state();
        {
            let mut store = s.store.lock().unwrap();
            let mut ai = store.accounts[0].clone();
            ai.id = "ai".into();
            ai.variant = "ai".into();
            store.accounts.push(ai);
            store.account_priority = crate::account_priority::AccountPriority::RoundRobin;
        }
        s.update_models(
            vec![
                json!({"id":"shared","maxOutputTokens":100}),
                json!({"id":"cn-only"}),
            ],
            vec!["shared".into(), "cn-only".into()],
            "cn".into(),
        )
        .unwrap();
        s.update_models(
            vec![
                json!({"id":"shared","maxOutputTokens":200}),
                json!({"id":"ai-only"}),
            ],
            vec!["shared".into(), "ai-only".into()],
            "ai".into(),
        )
        .unwrap();
        let mut selected = vec![];
        for _ in 0..4 {
            let (a, l) = s.acquire("shared", None).unwrap();
            selected.push(a.variant);
            drop(l);
        }
        assert_eq!(selected, vec!["cn", "ai", "cn", "ai"]);
        assert_eq!(s.acquire("cn-only", None).unwrap().0.variant, "cn");
        assert_eq!(s.acquire("ai-only", None).unwrap().0.variant, "ai");
        assert!(s.acquire("cn-only", Some("ai")).is_err());
        assert_eq!(
            s.model_config("shared", "cn").unwrap()["maxOutputTokens"],
            100
        );
        let now = Utc::now().timestamp_millis();
        {
            let mut store = s.store.lock().unwrap();
            store.resources.insert(
                "a".into(),
                json!({"ok":true,"resources":[{"remaining":10,"expireAt":now+60000}]}),
            );
            store.resources.insert(
                "ai".into(),
                json!({"ok":true,"resources":[{"remaining":100,"expireAt":now+600000}]}),
            );
            store.account_priority = crate::account_priority::AccountPriority::CreditFirst;
        }
        assert_eq!(s.acquire("shared", None).unwrap().0.variant, "ai");
        s.store.lock().unwrap().account_priority =
            crate::account_priority::AccountPriority::ExpiryFirst;
        assert_eq!(s.acquire("shared", None).unwrap().0.variant, "cn");
        s.set_model_policy(
            "shared".into(),
            crate::model_registry::Policy {
                alias: "public-shared".into(),
                regions: Some(vec!["ai".into()]),
            },
        )
        .unwrap();
        assert_eq!(s.resolve_model("public-shared").unwrap().0, "shared");
        assert_eq!(s.acquire("shared", None).unwrap().0.variant, "ai");
        let restored = AppState::new(s.path.clone()).unwrap();
        assert_eq!(restored.resolve_model("public-shared").unwrap().0, "shared");
        assert_eq!(
            restored
                .models()
                .iter()
                .find(|m| m["id"] == "shared")
                .unwrap()["regions"],
            json!(["ai"])
        );
    }
    #[test]
    fn region_capability_and_new_catalog_alias_collision_are_safe() {
        let s = state();
        {
            let mut store = s.store.lock().unwrap();
            let mut ai = store.accounts[0].clone();
            ai.id = "ai".into();
            ai.variant = "ai".into();
            store.accounts.push(ai);
        }
        s.update_models(
            vec![json!({"id":"shared","supportsImages":false})],
            vec!["shared".into()],
            "cn".into(),
        )
        .unwrap();
        s.update_models(
            vec![json!({"id":"shared","supportsImages":true})],
            vec!["shared".into()],
            "ai".into(),
        )
        .unwrap();
        let body = json!({"model":"shared","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.com/a.png"}}]}]});
        assert_eq!(
            s.models().iter().find(|m| m["id"] == "shared").unwrap()["supportsImages"],
            true
        );
        assert_eq!(
            s.acquire_for_body("shared", None, Some(&body))
                .unwrap()
                .0
                .variant,
            "ai"
        );
        s.set_model_policy(
            "shared".into(),
            crate::model_registry::Policy {
                alias: "future-real".into(),
                regions: None,
            },
        )
        .unwrap();
        assert!(s
            .update_models(
                vec![json!({"id":"future-real"})],
                vec!["future-real".into()],
                "cn".into()
            )
            .is_err());
        assert_eq!(s.resolve_model("future-real").unwrap().0, "shared");
    }
    #[test]
    fn concurrency_lease_releases_on_cancel() {
        let s = state();
        let (_, a) = s.acquire("hy3", None).unwrap();
        let (_, b) = s.acquire("hy3", None).unwrap();
        assert!(s.acquire("hy3", None).is_err());
        drop(a);
        assert!(s.acquire("hy3", None).is_ok());
        drop(b);
        assert_eq!(s.runtime.lock().unwrap().in_flight["a"], 0);
    }
    #[test]
    fn maintenance_has_bounded_independent_capacity_and_does_not_move_cursor() {
        let s = state();
        let (_, refresh) = s.acquire("credits", Some("a")).unwrap();
        let (_, first) = s.acquire("hy3", None).unwrap();
        let (_, second) = s.acquire("hy3", None).unwrap();
        assert!(s.acquire("hy3", None).is_err());
        assert!(s.acquire("credits", Some("a")).is_err());
        assert_eq!(s.snapshot()["accounts"][0]["inFlight"], 2);
        assert_eq!(s.snapshot()["accounts"][0]["managementInFlight"], 1);
        drop(refresh);
        assert_eq!(s.snapshot()["accounts"][0]["inFlight"], 2);
        assert!(s.acquire("credits", Some("a")).is_ok());
        drop((first, second));
        assert_eq!(s.snapshot()["accounts"][0]["inFlight"], 0);
    }

    #[test]
    fn maintenance_keeps_round_robin_cursor_unchanged() {
        let s = state();
        {
            let mut store = s.store.lock().unwrap();
            let mut b = store.accounts[0].clone();
            b.id = "b".into();
            store.accounts.push(b);
        }
        s.set_account_priority(crate::account_priority::AccountPriority::RoundRobin)
            .unwrap();
        let (first, lease) = s.acquire("hy3", None).unwrap();
        assert_eq!(first.id, "a");
        drop(lease);
        let (_, refresh) = s.acquire("credits", Some("a")).unwrap();
        drop(refresh);
        assert_eq!(s.acquire("hy3", None).unwrap().0.id, "b");
    }

    #[test]
    fn priority_is_persisted_and_drives_real_selection_with_safe_fallbacks() {
        use crate::account_priority::AccountPriority::*;
        let s = state();
        let now = Utc::now().timestamp_millis();
        {
            let mut store = s.store.lock().unwrap();
            let mut b = store.accounts[0].clone();
            b.id = "b".into();
            let mut unknown = b.clone();
            unknown.id = "unknown".into();
            store.accounts.extend([b, unknown]);
            store.resources.insert(
                "a".into(),
                json!({"ok":true,"resources":[{"remaining":3,"expireAt":now+60000}]}),
            );
            store.resources.insert("b".into(), json!({"ok":true,"resources":[{"remaining":500,"expireAt":now+120000},{"remaining":9999,"expireAt":now-1}]}));
        }
        let selected = |model: &str, pinned| {
            let (a, _lease) = s.acquire(model, pinned).unwrap();
            a.id
        };
        s.set_account_priority(ExpiryFirst).unwrap();
        assert_eq!(selected("hy3", None), "a");
        assert_eq!(selected("hy3", None), "a");
        let (_, full1) = s.acquire("hy3", None).unwrap();
        let (_, full2) = s.acquire("hy3", None).unwrap();
        assert_eq!(selected("hy3", None), "b");
        drop((full1, full2));
        s.set_account_priority(CreditFirst).unwrap();
        assert_eq!(selected("hy3", None), "b");
        assert_eq!(selected("hy3", Some("a")), "a");
        s.record_model_limit("b", Some("hy3".into()), now + 60000)
            .unwrap();
        assert_eq!(selected("hy3", None), "a");
        assert_eq!(selected("glm-5.3", None), "b");
        s.set_enabled("b", false).unwrap();
        assert_eq!(selected("glm-5.3", None), "a");
        let loaded = AppState::new(s.path.clone()).unwrap();
        assert_eq!(loaded.snapshot()["accountPriority"], "creditFirst");
        s.set_enabled("b", true).unwrap();
        s.set_account_priority(RoundRobin).unwrap();
        let round = (0..3)
            .map(|_| selected("glm-5.3", None))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(round.len(), 3);
        // Equal priority still rotates, rather than permanently choosing the first account.
        s.cache_credit(
            "b",
            json!({"ok":true,"resources":[{"remaining":3,"expireAt":now+60000}]}),
        )
        .unwrap();
        s.set_account_priority(ExpiryFirst).unwrap();
        assert_ne!(selected("glm-5.3", None), selected("glm-5.3", None));
    }

    #[test]
    fn metadata_writer_does_not_lock_routing_and_concurrent_writes_are_not_lost() {
        let s = state();
        let (started, ready) = std::sync::mpsc::channel();
        let (resume, wait) = std::sync::mpsc::channel();
        let writer_state = s.clone();
        let writer = std::thread::spawn(move || {
            writer_state.update_store(|store| {
                started.send(()).unwrap();
                wait.recv_timeout(std::time::Duration::from_secs(3))
                    .unwrap();
                store
                    .resources
                    .insert("a".into(), json!({"ok":true,"totalRemaining":90}));
                Ok(())
            })
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        // Reader locks must remain available even while the persistence transaction stalls.
        assert!(s.store.try_lock().is_ok());
        assert!(s.runtime.try_lock().is_ok());
        let (_, in_progress) = s.acquire("hy3", None).unwrap();
        let second_state = s.clone();
        let second = std::thread::spawn(move || second_state.set_enabled("a", false));
        resume.send(()).unwrap();
        writer.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        assert!(s.acquire("hy3", None).is_err());
        assert_eq!(s.snapshot()["accounts"][0]["inFlight"], 1);
        assert_eq!(s.snapshot()["accounts"][0]["credit"]["totalRemaining"], 90);
        drop(in_progress);
        assert_eq!(s.snapshot()["accounts"][0]["inFlight"], 0);
        let loaded = AppState::new(s.path.clone()).unwrap();
        assert_eq!(loaded.snapshot()["accounts"][0]["enabled"], false);
        assert_eq!(
            loaded.snapshot()["accounts"][0]["credit"]["totalRemaining"],
            90
        );
    }

    #[test]
    fn persistence_failure_keeps_the_previous_live_state() {
        let s = state();
        std::fs::create_dir_all(&s.path).unwrap(); // rename cannot replace a directory
        assert!(s.set_enabled("a", false).is_err());
        assert!(s.acquire("hy3", None).is_ok());
        assert_eq!(s.snapshot()["accounts"][0]["enabled"], true);
    }

    #[tokio::test]
    async fn live_http_stream_survives_credit_refresh_and_account_toggles() {
        use axum::{body::Body, routing::get};
        use futures_util::StreamExt;
        let s = state();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let receiver = Arc::new(tokio::sync::Mutex::new(Some(rx)));
        let stream_state = s.clone();
        let app = axum::Router::new().route("/health", get(|| async { "ok" })).route(
            "/fixture-stream",
            get(move || {
                let (state, receiver) = (stream_state.clone(), receiver.clone());
                async move {
                    let (_, lease) = state.acquire("hy3", Some("a")).unwrap();
                    let rx = receiver.lock().await.take().unwrap();
                    let stream =
                        futures_util::stream::unfold((rx, lease), |(mut rx, lease)| async move {
                            rx.recv()
                                .await
                                .map(|chunk| (Ok::<_, std::io::Error>(chunk), (rx, lease)))
                        });
                    axum::response::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(Body::from_stream(stream))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        tx.send("data: first\n\n".into()).unwrap();
        let response = client
            .get(format!("http://{address}/fixture-stream"))
            .send()
            .await
            .unwrap();
        let mut stream = response.bytes_stream();
        assert_eq!(
            &stream.next().await.unwrap().unwrap()[..],
            b"data: first\n\n"
        );
        let (_, maintenance) = s.acquire("credits", Some("a")).unwrap();
        let (_, other_chat) = s.acquire("hy3", Some("a")).unwrap();
        let update = s.clone();
        tokio::task::spawn_blocking(move || {
            update
                .cache_credit("a", json!({"ok":true,"totalRemaining":50}))
                .unwrap();
            update.set_enabled("a", false).unwrap();
        })
        .await
        .unwrap();
        assert!(s.acquire("hy3", Some("a")).is_err());
        assert_eq!(
            client
                .get(format!("http://{address}/health"))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        tx.send("data: last\n\ndata: [DONE]\n\n".into()).unwrap();
        drop(tx);
        let mut rest = vec![];
        while let Some(chunk) =
            tokio::time::timeout(std::time::Duration::from_secs(3), stream.next())
                .await
                .unwrap()
        {
            rest.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(rest, b"data: last\n\ndata: [DONE]\n\n");
        drop((maintenance, other_chat));
        assert_eq!(s.snapshot()["accounts"][0]["inFlight"], 0);
        assert_eq!(s.snapshot()["accounts"][0]["managementInFlight"], 0);
        s.set_enabled("a", true).unwrap();
        assert!(s.acquire("hy3", Some("a")).is_ok());
        server.abort();
    }
    #[test]
    fn account_selection_reports_missing_target_and_concrete_exclusion_reason() {
        let s = state();
        let missing = s.acquire("hy3", Some("missing")).err().unwrap();
        assert!(missing.contains("指定账号不存在"));
        assert!(s.runtime.lock().unwrap().in_flight.is_empty());
        s.store.lock().unwrap().accounts[0].enabled = false;
        assert!(s
            .acquire("hy3", None)
            .err()
            .unwrap()
            .contains("未启用反代 1 个"));
        s.store.lock().unwrap().accounts[0].enabled = true;
        let (_, first) = s.acquire("hy3", None).unwrap();
        let (_, second) = s.acquire("hy3", None).unwrap();
        assert!(s
            .acquire("hy3", None)
            .err()
            .unwrap()
            .contains("并发已满 1 个"));
        drop((first, second));
        assert!(s.acquire("hy3", None).is_ok());
    }
    #[test]
    fn logs_keep_the_account_name_at_request_time_and_detailed_errors() {
        let s = state();
        let (_, mut lease) = s.acquire("kimi-k3-1", None).unwrap();
        lease.status = 400;
        lease.outcome = "upstream_rejected".into();
        lease.details = crate::diagnostics::RequestDetails::incoming(
            "anthropic-messages",
            "/v1/messages",
            &json!({"messages":[{"role":"user","content":"private-prompt"}]}),
        );
        lease.details.upstream_error(
            &json!({"code":12345,"msg":"拒绝请求","requestId":"upstream-1"}),
            &[],
        );
        s.store.lock().unwrap().accounts[0].name = "renamed".into();
        drop(lease);
        let rt = s.runtime.lock().unwrap();
        let log = &rt.logs[0];
        assert_eq!(log.account_name, "a");
        assert_eq!(log.account_id, "a");
        assert_eq!(log.account_variant, "cn");
        assert_eq!(log.details.error["code"], 12345);
        assert!(!serde_json::to_string(log)
            .unwrap()
            .contains("private-prompt"));
    }
    #[test]
    fn cooldown_auth_disabled_and_expiry_are_excluded() {
        let s = state();
        s.runtime
            .lock()
            .unwrap()
            .cooldown
            .insert("a".into(), Utc::now().timestamp_millis() + 60000);
        assert!(s.acquire("hy3", None).is_err());
        s.runtime.lock().unwrap().cooldown.clear();
        s.runtime
            .lock()
            .unwrap()
            .auth_failed
            .insert("a".into(), true);
        assert!(s.acquire("hy3", None).is_err());
        s.runtime.lock().unwrap().auth_failed.clear();
        s.store.lock().unwrap().accounts[0].enabled = false;
        assert!(s.acquire("hy3", None).is_err());
        s.store.lock().unwrap().accounts[0].enabled = true;
        s.store.lock().unwrap().accounts[0].expires_at = Some(1);
        assert!(s.acquire("hy3", None).is_err());
    }

    #[test]
    fn renewed_source_expiry_overrides_imported_snapshot_for_ui_and_scheduling() {
        let s = state();
        {
            let mut store = s.store.lock().unwrap();
            store.accounts[0].source = "switch".into();
            store.accounts[0].expires_at = Some(1);
        }
        // Before the source scan, an old imported timestamp is not evidence
        // that the credential currently used for requests has expired.
        assert_eq!(s.pool_accounts()[0].expires_at, None);
        assert!(s.snapshot()["accounts"][0]["expiresAt"].is_null());
        drop(s.acquire("hy3", None).unwrap());
        let future = Utc::now().timestamp_millis() + 86_400_000;
        s.runtime.lock().unwrap().presentations.insert("a".into(), json!({"expiresAt":future}));
        assert_eq!(s.pool_accounts()[0].expires_at, Some(future));
        assert_eq!(s.snapshot()["accounts"][0]["expiresAt"], future);
        drop(s.acquire("hy3", None).unwrap());
        // The reverse case must also follow the latest source, not an old
        // still-valid imported timestamp.
        s.store.lock().unwrap().accounts[0].expires_at = Some(future);
        s.runtime.lock().unwrap().presentations.insert("a".into(), json!({"expiresAt":1}));
        assert_eq!(s.pool_accounts()[0].expires_at, Some(1));
        assert_eq!(s.snapshot()["accounts"][0]["expiresAt"], 1);
        assert!(s.acquire("hy3", None).is_err());
    }

    #[test]
    fn immediate_metadata_read_replaces_stale_monitor_state_without_touching_tokens() {
        let s = state();
        s.store.lock().unwrap().accounts[0].source = "switch".into();
        let accounts = s.store.lock().unwrap().accounts.clone();
        s.runtime.lock().unwrap().presentations.insert("a".into(), json!({"expiresAt":1,"needsRelogin":true,"displayName":"kept"}));
        let future = Utc::now().timestamp_millis() + 86_400_000;
        let records = vec![json!({"id":"a","expiresAt":future,"access_token":"fixture-secret"})];
        apply_login_metadata(&accounts, &records, &mut s.runtime.lock().unwrap());
        assert_eq!(s.pool_accounts()[0].expires_at, Some(future));
        assert!(s.pool_accounts()[0].blocked.is_none());
        let snapshot = s.snapshot();
        assert_eq!(snapshot["accounts"][0]["presentation"]["displayName"], "kept");
        assert!(!snapshot.to_string().contains("fixture-secret"));
        // Missing/unreadable source data is unknown, not proof of expiry.
        apply_login_metadata(&accounts, &[], &mut s.runtime.lock().unwrap());
        assert_eq!(s.pool_accounts()[0].expires_at, None);
        assert!(s.pool_accounts()[0].blocked.is_none());
    }

    #[test]
    fn cn_and_ai_accounts_are_not_mixed() {
        let s = state();
        let mut ai = s.store.lock().unwrap().accounts[0].clone();
        ai.id = "ai".into();
        ai.variant = "ai".into();
        s.store.lock().unwrap().accounts.push(ai);
        let (cn, lease) = s.acquire("hy3", None).unwrap();
        assert_eq!(cn.variant, "cn");
        drop(lease);
        let (cn, lease) = s.acquire("hy3", None).unwrap();
        assert_eq!(cn.variant, "cn");
        drop(lease);
        s.store.lock().unwrap().catalog_variant = Some("ai".into());
        let (ai, _lease) = s.acquire("hy3", None).unwrap();
        assert_eq!(ai.variant, "ai");
    }

    #[test]
    fn old_metadata_does_not_enable_background_requests() {
        let store: Store = serde_json::from_value(json!({"accounts":[],"models":null})).unwrap();
        assert!(!store.auto_checkin);
        assert!(store.catalog_variant.is_none());
        assert_eq!(
            store.account_priority,
            crate::account_priority::AccountPriority::ExpiryFirst
        );
    }
    #[test]
    fn k3_limit_does_not_disable_other_models_and_expires() {
        let s = state();
        let now = Utc::now().timestamp_millis();
        s.store.lock().unwrap().model_limits.insert(
            "a".into(),
            vec![ModelLimit {
                model: Some("kimi-k3-1".into()),
                reset_at: now + 60000,
                first_seen_at: now,
                source: "test".into(),
            }],
        );
        assert!(s.acquire("kimi-k3-1", None).is_err());
        assert!(s.acquire("glm-5.3", None).is_ok());
        s.store.lock().unwrap().model_limits.get_mut("a").unwrap()[0].reset_at = now - 1;
        assert!(s.acquire("kimi-k3-1", None).is_ok());
    }
    #[test]
    fn checkin_is_independent_and_does_not_consume_chat_capacity() {
        let s = state();
        s.store.lock().unwrap().accounts[0].enabled = false;
        assert!(s.acquire("glm-5.3", Some("a")).is_err());
        let (_, lease) = s.acquire("checkin", Some("a")).unwrap();
        assert_eq!(s.runtime.lock().unwrap().in_flight.get("a"), None);
        assert!(s.acquire("checkin", Some("a")).is_err());
        drop(lease);
        assert!(s.acquire("checkin", Some("a")).is_ok());
    }
    #[test]
    fn checkin_guard_covers_request_and_persistence_and_releases_on_drop() {
        let s = state();
        let guard = crate::checkin::RunGuard::acquire(s.clone(), "a").unwrap();
        assert!(crate::checkin::RunGuard::acquire(s.clone(), "a").is_err());
        assert!(crate::checkin::RunGuard::acquire(s.clone(), "b").is_ok());
        drop(guard);
        assert!(crate::checkin::RunGuard::acquire(s, "a").is_ok());
    }
    #[test]
    fn persisted_checkin_survives_runtime_reset() {
        let s = state();
        s.store
            .lock()
            .unwrap()
            .checkin_records
            .push(crate::checkin::Record {
                at: 123,
                account_id: "a".into(),
                submit: true,
                status: Some(json!({"today_checked_in":true,"today_credit":10})),
                error: None,
            });
        let snap = s.snapshot();
        assert_eq!(snap["accounts"][0]["checkin"]["today_checked_in"], true);
        assert_eq!(snap["accounts"][0]["checkinUpdatedAt"], 123);
        let serialized = serde_json::to_value(&*s.store.lock().unwrap()).unwrap();
        let restored: Store = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored.checkin_records.len(), 1);
        assert!(!restored.auto_checkin);
    }
    #[test]
    fn legacy_account_signin_is_off() {
        let a: Account = serde_json::from_value(
            json!({"id":"a","name":"a","source":"switch","variant":"cn","enabled":true}),
        )
        .unwrap();
        assert!(!a.checkin_enabled);
    }
    #[tokio::test]
    async fn disabled_signin_returns_before_credentials_or_network() {
        let s = state();
        let error = crate::proxy::checkin(s, "a".into(), true)
            .await
            .unwrap_err();
        assert!(error.contains("开关已关闭"));
    }
}
