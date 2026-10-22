//! Qoder upstream for the unified gateway: catalog sync through the installed
//! runtime, native-signed (legacy) or Model Server transport, and Qoder's SSE
//! envelopes normalized into chat chunks. Official tokens stay in the backend.
use crate::qcore;
use agentdock_core::{
    anthropic,
    diagnostics::RequestDetails,
    provider::{
        AccountEffect, CreditRank, PoolAccount, ProviderKind, SendFailure, SendInput, SendOk,
        UpstreamModel,
    },
    stream::{StreamFailure, UpstreamEvent, MAX_EVENT, MAX_OUTPUT},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;

const MAX_BODY: usize = agentdock_core::MAX_REQUEST_BODY_BYTES;
pub const CLIENT_VERSION: &str = "1.1.66";

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// "legacy" = Qoder native signing via the installed runtime; "modelServer" = direct HTTP.
    pub transport: String,
    pub cn_endpoint: String,
    pub intl_endpoint: String,
    pub disabled_accounts: Vec<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            transport: "legacy".into(),
            cn_endpoint: "https://gateway.qoder.com.cn".into(),
            intl_endpoint: "https://api2.qoder.sh".into(),
            disabled_accounts: vec![],
        }
    }
}
pub fn validate_config(c: &Config) -> Result<(), String> {
    if !["legacy", "modelServer"].contains(&c.transport.as_str())
        || !valid_endpoint(&c.cn_endpoint)
        || !valid_endpoint(&c.intl_endpoint)
    {
        return Err("配置无效：请选择有效传输协议和 Qoder 官方 HTTPS 地址".into());
    }
    Ok(())
}
pub fn valid_endpoint(s: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(s) else {
        return false;
    };
    let host = url.host_str().unwrap_or("");
    url.scheme() == "https"
        && (host.ends_with(".qoder.sh")
            || host.ends_with(".qoder.com.cn")
            || host.ends_with(".qoder.com"))
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path() == "/"
}

pub fn data_file(name: &str) -> PathBuf {
    qcore::switch_dir().join("gateway").join(name)
}
pub fn write_private(path: &PathBuf, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    fs::create_dir_all(path.parent().unwrap()).map_err(|_| "无法创建网关数据目录")?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|_| "无法保存网关配置")?;
    file.write_all(data).map_err(|_| "无法写入网关配置")?;
    file.sync_all().map_err(|_| "无法同步网关配置")?;
    fs::rename(temp, path).map_err(|_| "无法更新网关配置".into())
}

pub struct Provider {
    #[cfg(any(test, feature = "fixtures"))]
    pub fixture_accounts: Mutex<Option<Vec<qcore::Account>>>,
    config: Mutex<Config>,
    models: Mutex<HashMap<String, Vec<Value>>>,
    model_updated: Mutex<HashMap<String, String>>,
    credits: Mutex<HashMap<String, Value>>,
    client: reqwest::Client,
}

impl Provider {
    pub fn new() -> Result<Arc<Self>, String> {
        let config: Config = fs::read(data_file("settings.json"))
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default();
        let config = if validate_config(&config).is_ok() {
            config
        } else {
            Config {
                disabled_accounts: config.disabled_accounts,
                ..Config::default()
            }
        };
        let mut models = HashMap::new();
        let mut updated = HashMap::new();
        for ed in ["cn", "intl"] {
            let path = data_file(&format!("models-{ed}.json"));
            if let Ok(bytes) = fs::read(&path) {
                if let Ok(items) = serde_json::from_slice::<Vec<Value>>(&bytes) {
                    models.insert(ed.into(), items);
                    if let Ok(time) = fs::metadata(&path).and_then(|m| m.modified()) {
                        updated.insert(
                            ed.into(),
                            chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339(),
                        );
                    }
                }
            }
        }
        let credits = fs::read(data_file("credits.json"))
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default();
        Ok(Arc::new(Self {
            #[cfg(any(test, feature = "fixtures"))]
            fixture_accounts: Mutex::new(None),
            config: Mutex::new(config),
            models: Mutex::new(models),
            model_updated: Mutex::new(updated),
            credits: Mutex::new(credits),
            client: client()?,
        }))
    }
    #[cfg(any(test, feature = "fixtures"))]
    pub fn fixture(
        accounts: Vec<qcore::Account>,
        config: Config,
        models: HashMap<String, Vec<Value>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            fixture_accounts: Mutex::new(Some(accounts)),
            config: Mutex::new(config),
            models: Mutex::new(models),
            model_updated: Mutex::new(HashMap::new()),
            credits: Mutex::new(HashMap::new()),
            client: reqwest::Client::new(),
        })
    }
    pub fn accounts(&self) -> Vec<qcore::Account> {
        #[cfg(any(test, feature = "fixtures"))]
        if let Some(accounts) = self.fixture_accounts.lock().unwrap().as_ref() {
            return accounts.clone();
        }
        qcore::load_accounts()
    }
    pub fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }
    /// Connection settings require a stopped gateway; account switches apply live.
    pub fn save_config(&self, c: Config, running: bool) -> Result<(), String> {
        validate_config(&c)?;
        let mut current = self.config.lock().unwrap();
        if running
            && (current.transport != c.transport
                || current.cn_endpoint != c.cn_endpoint
                || current.intl_endpoint != c.intl_endpoint)
        {
            return Err("请先停止服务再修改 Qoder 传输协议或上游地址".into());
        }
        write_private(
            &data_file("settings.json"),
            &serde_json::to_vec_pretty(&c).unwrap(),
        )?;
        *current = c;
        Ok(())
    }
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let mut c = self.config();
        c.disabled_accounts.retain(|x| x != id);
        if !enabled {
            c.disabled_accounts.push(id.to_owned());
        }
        let mut current = self.config.lock().unwrap();
        write_private(
            &data_file("settings.json"),
            &serde_json::to_vec_pretty(&c).unwrap(),
        )?;
        *current = c;
        Ok(())
    }
    pub fn catalogs(&self) -> HashMap<String, Vec<Value>> {
        self.models.lock().unwrap().clone()
    }
    pub fn catalog_updated(&self) -> HashMap<String, String> {
        self.model_updated.lock().unwrap().clone()
    }
    pub fn credit_cache(&self) -> HashMap<String, Value> {
        self.credits.lock().unwrap().clone()
    }
    /// Remember the latest quota result for ranking and display across restarts.
    pub fn cache_credits(&self, id: &str, entry: Value) {
        let mut credits = self.credits.lock().unwrap();
        let value = if entry["ok"] == false {
            match credits.get(id).filter(|v| v["ok"] == true) {
                Some(old) => {
                    let mut cached = old.clone();
                    cached["stale"] = json!(true);
                    cached["error"] = entry["error"].clone();
                    cached
                }
                None => entry,
            }
        } else {
            entry
        };
        credits.insert(id.to_owned(), value);
        let _ = write_private(
            &data_file("credits.json"),
            &serde_json::to_vec(&*credits).unwrap_or_default(),
        );
    }
    pub fn forget_account(&self, id: &str) {
        let mut credits = self.credits.lock().unwrap();
        if credits.remove(id).is_some() {
            let _ = write_private(
                &data_file("credits.json"),
                &serde_json::to_vec(&*credits).unwrap_or_default(),
            );
        }
    }

    pub async fn refresh_models(&self, regions: &[&str]) -> Result<Value, String> {
        let mut synced = Vec::new();
        let mut warnings = Vec::new();
        let mut count = 0;
        for edition in regions {
            match self.refresh_region_models(edition).await {
                Ok(v) => {
                    count += v["count"].as_u64().unwrap_or(0);
                    synced.push(edition.to_string());
                }
                Err(e) => warnings.push(format!(
                    "Qoder {}：{e}",
                    if *edition == "cn" { "国内版" } else { "国际版" }
                )),
            }
        }
        if synced.is_empty() {
            return Err(warnings.join("；"));
        }
        Ok(json!({"count":count,"synced":synced,"warnings":warnings}))
    }
    async fn refresh_region_models(&self, edition: &str) -> Result<Value, String> {
        let disabled = self.config().disabled_accounts;
        let ed = edition.to_owned();
        let account = self
            .accounts()
            .into_iter()
            .find(|a| a.edition == ed && !disabled.contains(&a.id()) && !a.token.is_empty())
            .ok_or("请先添加或采集对应地区账号")?;
        let endpoint = if edition == "cn" {
            "https://gateway.qoder.com.cn"
        } else {
            "https://api2.qoder.sh"
        };
        let machine = machine_id(edition);
        let prepared = runtime_helper(json!({"op":"prepare_models","edition":edition,"uid":account.uid,"token":account.token,"machineId":machine,"endpoint":endpoint})).await?;
        let url = prepared["url"].as_str().ok_or("模型目录地址缺失")?;
        // Only the installed runtime may choose an official HTTPS endpoint.
        let parsed = reqwest::Url::parse(url).map_err(|_| "模型目录地址无效")?;
        if !valid_endpoint(parsed.origin().ascii_serialization().as_str()) {
            return Err("运行时返回非官方目录地址".into());
        }
        let mut req = self.client.get(url);
        for (k, v) in prepared["headers"].as_object().ok_or("目录鉴权头缺失")? {
            if let Some(v) = v.as_str() {
                req = req.header(k, v);
            }
        }
        let response = req.send().await.map_err(|_| "模型目录连接失败")?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(format!("模型目录查询失败，HTTP {status}；请刷新账号后重试"));
        }
        let bytes = bounded_body(response, 8 * 1024 * 1024).await?;
        let text = String::from_utf8(bytes).map_err(|_| "模型目录编码无效")?;
        let raw = runtime_helper(json!({"op":"decrypt","edition":edition,"text":text})).await?;
        let models = normalize_models(&raw)?;
        write_private(
            &data_file(&format!("models-{edition}.json")),
            &serde_json::to_vec_pretty(&models).unwrap(),
        )?;
        self.models
            .lock()
            .unwrap()
            .insert(edition.into(), models.clone());
        self.model_updated
            .lock()
            .unwrap()
            .insert(edition.into(), chrono::Utc::now().to_rfc3339());
        Ok(json!({"count":models.len()}))
    }

    pub fn upstream_models(&self) -> Vec<UpstreamModel> {
        let models = self.models.lock().unwrap();
        let mut out = vec![];
        for edition in ["cn", "intl"] {
            for m in models.get(edition).into_iter().flatten() {
                let Some(id) = m["id"].as_str() else {
                    continue;
                };
                out.push(UpstreamModel {
                    provider: Some(ProviderKind::Qoder),
                    region: edition.into(),
                    id: id.into(),
                    name: m["name"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or(id)
                        .into(),
                    supports_tools: m["supportsTools"].as_bool(),
                    supports_images: m["supportsImages"].as_bool(),
                    efforts: m["reasoning"]["supportedEfforts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                    default_effort: m["reasoning"]["defaultEffort"]
                        .as_str()
                        .map(str::to_owned),
                    context_window: m["contextWindow"].as_u64(),
                    max_output: None,
                    config: m.clone(),
                });
            }
        }
        out
    }

    pub fn pool_accounts(&self) -> Vec<PoolAccount> {
        let disabled = self.config().disabled_accounts;
        let credits = self.credits.lock().unwrap().clone();
        self.accounts()
            .iter()
            .enumerate()
            .map(|(order, a)| {
                let id = a.id();
                PoolAccount {
                    provider: ProviderKind::Qoder,
                    region: a.edition.clone(),
                    name: if a.note.is_empty() {
                        a.name.clone()
                    } else {
                        a.note.clone()
                    },
                    enabled: !disabled.contains(&id),
                    expires_at: chrono::DateTime::parse_from_rfc3339(&a.expiresAt)
                        .ok()
                        .map(|t| t.timestamp_millis()),
                    credit: credit_rank(credits.get(&id)),
                    order,
                    blocked: a.token.is_empty().then(|| "缺少登录令牌".to_owned()),
                    id,
                }
            })
            .collect()
    }
    /// Qoder catalogs are per region: an account serves any model of its region.
    pub fn chat_eligibility(&self, account_id: &str, model: &str) -> Result<Value, &'static str> {
        let edition = account_id.split_once(':').map(|(e, _)| e).unwrap_or("");
        self.models
            .lock()
            .unwrap()
            .get(edition)
            .and_then(|rows| rows.iter().find(|m| m["id"] == model).cloned())
            .ok_or("地区不支持此模型")
    }

    pub async fn send(
        &self,
        account_id: &str,
        model_info: Value,
        input: SendInput,
        details: &mut RequestDetails,
        started: Instant,
    ) -> Result<SendOk, SendFailure> {
        let account = self
            .accounts()
            .into_iter()
            .find(|a| a.id() == account_id)
            .ok_or_else(|| SendFailure::local(503, "no_available_account", "账号已移除"))?;
        let config = self.config();
        let edition = account.edition.clone();
        let mut body = input.body;
        let upstream_model = body["model"].as_str().unwrap_or("").to_owned();
        if input.effort_input.is_object() {
            anthropic::apply_effort(&input.effort_input, &mut body, &model_info);
        }
        if let Some(effort) = body["reasoning_effort"].as_str() {
            if model_info["reasoning"]["supportedEfforts"]
                .as_array()
                .is_some_and(|a| !a.is_empty() && !a.iter().any(|v| v == effort))
            {
                let e = "当前模型不支持所选思考档位，请在模型目录查看支持的档位";
                details.local_error("validation", e);
                return Err(SendFailure::local(400, "invalid_request", e));
            }
        }
        normalize_qoder_messages(&mut body);
        if let Err(e) = validate_tool_history(&body) {
            details.local_error("validation", &e);
            return Err(SendFailure::local(400, "invalid_request", e));
        }
        body["stream"] = json!(true);
        body["stream_options"] = json!({"include_usage":true});
        let request_id = input.request_id.clone();
        let session = input
            .session_id
            .clone()
            .or_else(|| agentdock_core::session::session_label(&input.headers))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        body["metadata"] = json!({"context":{"request_id":request_id,"request_set_id":request_id,"session_id":session,"client_type":"qodercli"},"business":{"id":request_id,"name":"chat","product":"Qoder","version":CLIENT_VERSION,"type":"cli","begin_at":chrono::Utc::now().timestamp_millis()}});
        if body.to_string().len() > MAX_BODY {
            details.local_error("encoding", "转换后请求超过 32 MiB");
            return Err(SendFailure::local(
                413,
                "request_too_large",
                "转换后请求超过 32 MiB",
            ));
        }
        details.upstream_summary = agentdock_core::diagnostics::summary(&body);
        details.upstream_summary["transport"] = json!(config.transport);
        let endpoint = if edition == "cn" {
            config.cn_endpoint.clone()
        } else {
            config.intl_endpoint.clone()
        };
        let request = if config.transport == "legacy" {
            let legacy = legacy_body(&body, &model_info, &request_id, &session, &edition);
            let prepared = runtime_helper(json!({"op":"prepare_chat","edition":edition,"uid":account.uid,"token":account.token,"machineId":machine_id(&edition),"endpoint":endpoint,"body":legacy,"model":upstream_model}))
                .await
                .map_err(|e| {
                    details.local_error("runtime", &e);
                    SendFailure::local(502, "runtime_unavailable", e)
                })?;
            let Some(url) = prepared["url"].as_str() else {
                details.local_error("runtime", "推理地址缺失");
                return Err(SendFailure::local(502, "runtime_unavailable", "推理地址缺失"));
            };
            details.upstream_url = reqwest::Url::parse(url)
                .ok()
                .map(|u| format!("{}{}", u.origin().ascii_serialization(), u.path()));
            if prepared["body"].as_str().unwrap_or("").len() > MAX_BODY {
                details.local_error("encoding", "签名后请求超过 32 MiB");
                return Err(SendFailure::local(
                    413,
                    "request_too_large",
                    "签名后请求超过 32 MiB",
                ));
            }
            let mut request = self.client.post(url);
            for (name, value) in prepared["headers"].as_object().into_iter().flatten() {
                if let Some(value) = value.as_str() {
                    request = request.header(name, value);
                }
            }
            // Match the installed runtime's device headers, without changing its auth signature.
            let ed_key = edition.clone();
            let identity = tokio::task::spawn_blocking(move || {
                qcore::edition(&ed_key)
                    .map(qcore::device_headers)
                    .unwrap_or_default()
            })
            .await
            .unwrap_or_default();
            for (name, value) in identity {
                if name != "Cosy-Version" && name != "Cosy-ClientType" {
                    request = request.header(name, value);
                }
            }
            request
                .header("Accept", "text/event-stream")
                .body(prepared["body"].as_str().unwrap_or("").to_owned())
        } else {
            let url = format!(
                "{}/model/v1/chat/completions",
                endpoint.trim_end_matches('/')
            );
            details.upstream_url = Some(url.clone());
            self.client
                .post(url)
                .bearer_auth(&account.token)
                .header("Accept", "text/event-stream")
                .header("X-Request-ID", &request_id)
                .header("X-Session-ID", &session)
                .header("User-Agent", format!("qoder/{CLIENT_VERSION}"))
                .json(&body)
        };
        let response = request.send().await.map_err(|_| {
            details.local_error("upstream", "Qoder 上游连接失败或超时");
            SendFailure::local(502, "upstream_unreachable", "Qoder 上游连接失败或超时")
        })?;
        let status = response.status().as_u16();
        details.upstream_status = Some(status);
        details.content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        if !response.status().is_success() {
            let retry = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60)
                .min(3600);
            let mut message =
                format!("Qoder 上游拒绝请求（HTTP {status}），请检查账号、模型权限或接入地址");
            let mut code = Value::Null;
            if let Ok(bytes) = bounded_body(response, 64 * 1024).await {
                if details.first_byte_ms.is_none() {
                    details.first_byte_ms = Some(started.elapsed().as_millis());
                }
                if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                    if let Some(hint) = provider_error_hint(&v) {
                        message = format!("Qoder 上游拒绝请求（HTTP {status}）：{hint}");
                    }
                    details.upstream_error(&v, &[&account.token]);
                    code = v.get("code").or_else(|| v["error"].get("code")).map(|v| {
                        json!(v
                            .to_string()
                            .chars()
                            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .take(48)
                            .collect::<String>())
                    }).unwrap_or(Value::Null);
                }
            }
            if details.error_stage.is_none() {
                details.local_error("upstream", &message);
            }
            let mut failure = SendFailure::local(status, "upstream_error", message);
            failure.error = json!({"code":code});
            failure.effect = if status == 429 {
                AccountEffect::Cooldown(retry)
            } else if [401, 403].contains(&status) {
                AccountEffect::Cooldown(60)
            } else {
                AccountEffect::None
            };
            return Err(failure);
        }
        if !details
            .content_type
            .as_deref()
            .unwrap_or("")
            .contains("text/event-stream")
        {
            details.local_error("upstream", "Qoder 上游没有返回 SSE 流");
            return Err(SendFailure::local(
                502,
                "invalid_upstream_response",
                "Qoder 上游没有返回 SSE 流",
            ));
        }
        let mut upstream = response.bytes_stream();
        let events = async_stream::stream! {
            let mut parser = Parser::default();
            while let Some(part) = upstream.next().await {
                let part = match part {
                    Ok(b) => b,
                    Err(_) => {
                        yield Err(StreamFailure::new("上游连接中断或超时"));
                        return;
                    }
                };
                let events = match parser.feed(&part) {
                    Ok(v) => v,
                    Err(e) => {
                        yield Err(parser.failure(e));
                        return;
                    }
                };
                for event in events {
                    match event {
                        Some(chunk) => yield Ok(UpstreamEvent::Chunk(chunk)),
                        None => {
                            yield Ok(UpstreamEvent::Done);
                            return;
                        }
                    }
                }
            }
        };
        Ok(SendOk {
            events: Box::pin(events),
        })
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法初始化网络客户端".into())
}

/// Rank Qoder quota: subscription + personal packs; add-on expiry when packs remain.
pub fn credit_rank(entry: Option<&Value>) -> CreditRank {
    let Some(entry) = entry.filter(|v| v["usage"].is_object()) else {
        return CreditRank {
            unit: "credits".into(),
            ..CreditRank::default()
        };
    };
    let usage = &entry["usage"];
    let parts = [&usage["userQuota"], &usage["addOnQuota"]];
    let known = parts
        .iter()
        .any(|q| q["remaining"].as_f64().is_some_and(f64::is_finite));
    let remaining = known.then(|| {
        parts
            .iter()
            .filter_map(|q| q["remaining"].as_f64())
            .filter(|n| n.is_finite() && *n > 0.0)
            .sum::<f64>()
    });
    let addon = usage["addOnQuota"]["remaining"].as_f64().unwrap_or(0.0);
    let expiry = usage["expiresAt"]
        .as_i64()
        .filter(|t| addon > 0.0 && *t > 946_684_800_000 && *t < 253_402_214_400_000);
    CreditRank {
        remaining,
        earliest_expiry: expiry,
        unit: "credits".into(),
        updated_at: entry["updatedAt"].as_i64(),
    }
}

pub fn machine_id(edition: &str) -> String {
    if let Some(ed) = qcore::edition(edition) {
        for path in [
            crate::platform::data_dir(ed).join("auth.machine-id"),
            qcore::home_dir().join(ed.home_dir).join(".auth/machine_id"),
        ] {
            if let Ok(value) = fs::read_to_string(path) {
                if !value.trim().is_empty() {
                    return value.trim().to_owned();
                }
            }
        }
    }
    let folder = if edition == "cn" {
        ".qoder-cn"
    } else {
        ".qoder"
    };
    fs::read_to_string(qcore::home_dir().join(folder).join("installation_id"))
        .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string())
        .trim()
        .to_owned()
}

const RUNTIME_SCRIPT: &[u8] = include_bytes!("../resources/qoder-runtime.mjs");

pub async fn runtime_helper(input: Value) -> Result<Value, String> {
    let path = data_file("runtime-helper.mjs");
    // Keep the helper in sync when upgrading the app.
    if fs::read(&path).ok().as_deref() != Some(RUNTIME_SCRIPT) {
        write_private(&path, RUNTIME_SCRIPT)?;
    }
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin/node"),
        PathBuf::from("/usr/local/bin/node"),
    ];
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(
            std::env::split_paths(&path).map(|p| p.join(if cfg!(windows) { "node.exe" } else { "node" })),
        );
    }
    let nvm = qcore::home_dir().join(".nvm/versions/node");
    if let Ok(entries) = fs::read_dir(nvm) {
        for e in entries.flatten() {
            candidates.push(e.path().join("bin/node"));
        }
    }
    #[cfg(windows)]
    for key in ["ProgramFiles", "LOCALAPPDATA", "NVM_SYMLINK"] {
        if let Some(root) = std::env::var_os(key) {
            candidates.push(PathBuf::from(&root).join("nodejs/node.exe"));
            candidates.push(PathBuf::from(root).join("node.exe"));
        }
    }
    let ed = qcore::edition(input["edition"].as_str().unwrap_or("intl")).ok_or("未知版本")?;
    // The installed Qoder doubles as the Node runtime (ELECTRON_RUN_AS_NODE).
    let executable = crate::platform::app_executable(ed)
        .or_else(|| candidates.into_iter().find(|p| p.is_file()))
        .ok_or("请安装 Qoder 或 Node.js")?;
    let source = crate::platform::resources_dir(ed)
        .ok_or("请安装对应地区的 Qoder 客户端")?
        .join("app.asar.unpacked/node_modules/@qoder-ai")
        .join(if ed.key == "cn" { "qoder-cn-agent-sdk" } else { "qoder-agent-sdk" })
        .join("dist/_worker/qoder-worker-runtime.obf.mjs");
    let mut command = tokio::process::Command::new(executable);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .arg(path)
        .env("AGENTDOCK_QODER_SOURCE", source)
        .env("ELECTRON_RUN_AS_NODE", "1")
        .env(
            "AGENTDOCK_QODER_CACHE",
            qcore::switch_dir().join("runtime-cache"),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "无法启动 Qoder 协议运行时，请安装 Qoder 或 Node.js")?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&input).unwrap())
        .await
        .map_err(|_| "无法发送本地协议请求")?;
    let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .map_err(|_| "本地协议运行时超时")?
        .map_err(|_| "本地协议运行时失败")?;
    let v: Value = serde_json::from_slice(&output.stdout).map_err(|_| "本地协议运行时响应无效")?;
    if v["ok"] != true {
        return Err("Qoder 本地协议运行时不可用，请确认对应版本 Qoder 已安装且兼容".into());
    }
    Ok(v["data"].clone())
}

pub fn normalize_models(raw: &Value) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    if let Some(a) = raw.as_array() {
        rows.extend(a.iter());
    } else if let Some(a) = raw["assistant"]
        .as_array()
        .or_else(|| raw["data"]["assistant"].as_array())
        .or_else(|| raw["qodercli"].as_array())
        .or_else(|| raw["data"]["qodercli"].as_array())
        .or_else(|| raw["models"].as_array())
        .or_else(|| raw["data"].as_array())
    {
        rows.extend(a.iter());
    } else if let Some(o) = raw.as_object() {
        for v in o.values() {
            if let Some(a) = v.as_array() {
                rows.extend(a.iter());
            }
        }
    }
    let mut models = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for m in rows {
        let Some(id) = m["key"]
            .as_str()
            .or_else(|| m["id"].as_str())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if !ids.insert(id.to_owned()) {
            continue;
        }
        models.push(json!({"id":id,"name":m["display_name"].as_str().or_else(||m["name"].as_str()).unwrap_or(id),"supportsTools":m["supports_tool_call"].as_bool().or(m["support_tool_call"].as_bool()),"supportsImages":m["is_vl"].as_bool(),"reasoning":model_reasoning(m),"contextWindow":m["max_input_tokens"].as_u64().or(m["context_length"].as_u64()),"isReasoning":m["is_reasoning"].as_bool().unwrap_or(false),"format":m["format"].as_str().unwrap_or("openai"),"verified":false}));
    }
    if models.is_empty() {
        return Err("模型目录未返回可识别的模型；请检查账号权限与客户端版本".into());
    }
    Ok(models)
}
fn model_reasoning(model: &Value) -> Value {
    let mut efforts = Vec::new();
    let mut default = None;
    if let Some(o) = model["thinking_config"]["enabled"]["efforts"].as_object() {
        for (effort, entry) in o {
            efforts.push(effort.clone());
            if entry["is_default"] == true {
                default = Some(effort.clone());
            }
        }
    }
    if model["thinking_config"].get("disabled").is_some() {
        efforts.push("none".into());
    }
    json!({"supportedEfforts":efforts,"defaultEffort":default})
}
async fn bounded_body(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(part) = stream.next().await {
        let part = part.map_err(|_| "上游响应读取失败")?;
        if bytes.len() + part.len() > limit {
            return Err("上游响应超过本地大小限制".into());
        }
        bytes.extend_from_slice(&part);
    }
    Ok(bytes)
}

pub fn normalize_qoder_messages(chat: &mut Value) {
    for row in chat["messages"].as_array_mut().into_iter().flatten() {
        if let Some(parts) = row["content"].as_array() {
            if parts
                .iter()
                .all(|p| p["type"] == "text" && p["text"].is_string())
            {
                row["content"] = json!(parts
                    .iter()
                    .map(|p| p["text"].as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join("\n"));
            }
        }
        // Match the installed SDK's history contract, not just OpenAI's generic
        // shape: tool-only assistant content is an empty string and every
        // historical call carries its index. IDs and arguments are preserved.
        if row["role"] == "assistant" && row["tool_calls"].as_array().is_some_and(|a| !a.is_empty()) {
            if row["content"].is_null() {
                row["content"] = json!("");
            }
            for (index, call) in row["tool_calls"].as_array_mut().unwrap().iter_mut().enumerate() {
                call["index"] = json!(index);
            }
        }
    }
}

/// Orphaned, duplicated or unanswered tool history is rejected locally: the
/// upstream would refuse it with a 400 after charging the request setup.
pub fn validate_tool_history(chat: &Value) -> Result<(), String> {
    let mut pending = std::collections::HashSet::new();
    for (index, row) in chat["messages"].as_array().into_iter().flatten().enumerate() {
        if row["role"] == "tool" {
            let id = row["tool_call_id"].as_str().unwrap_or("");
            if !pending.remove(id) {
                return Err(format!(
                    "messages[{index}] 的工具结果缺少匹配的前置调用；请检查工具历史是否被截断"
                ));
            }
            continue;
        }
        if !pending.is_empty() {
            return Err(format!(
                "messages[{index}] 前尚有 {} 个工具调用未返回结果；工具结果必须先完整回传",
                pending.len()
            ));
        }
        for call in row["tool_calls"].as_array().into_iter().flatten() {
            let id = call["id"].as_str().filter(|s| !s.is_empty()).ok_or("工具调用缺少 ID")?;
            if !pending.insert(id.to_owned()) {
                return Err(format!("messages[{index}] 存在重复工具调用 ID"));
            }
        }
    }
    if !pending.is_empty() {
        return Err(format!("历史末尾尚有 {} 个工具调用未返回结果", pending.len()));
    }
    Ok(())
}

/// Fixed hints for known upstream validation errors; raw bodies are never echoed.
fn provider_error_hint(body: &Value) -> Option<&'static str> {
    let serialized = body.to_string().to_lowercase();
    if serialized.contains("must be a response to a preceding message")
        || serialized.contains("tool_call_id") && serialized.contains("tool_calls")
    {
        Some("工具历史校验失败：工具结果缺少对应的前置 tool_calls")
    } else if serialized.contains("reasoning_content") {
        Some("模型要求回传 reasoning_content，请检查思考历史兼容性")
    } else if serialized.contains("max_tokens") || serialized.contains("max_output_tokens") {
        Some("输出 token 参数不符合模型限制")
    } else if serialized.contains("context_length") || serialized.contains("context window") {
        Some("请求上下文超过模型限制")
    } else if serialized.contains("schema") && serialized.contains("tool") {
        Some("工具参数 schema 不符合上游要求")
    } else {
        None
    }
}

fn rejection_hint(status: u16) -> &'static str {
    match status {
        400 => "上游请求格式或参数校验失败",
        401 => "上游认证被拒绝，请刷新账号登录凭证",
        403 => "上游拒绝此请求，可能与模型权限或临时风控有关；该账号对此模型将短暂冷却",
        429 => "上游限流，请稍后重试",
        _ => "请检查模型权限、配额和请求兼容性",
    }
}

fn safe_code(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '/')
        .take(48)
        .collect()
}

pub fn legacy_body(
    chat: &Value,
    model_info: &Value,
    request_id: &str,
    session: &str,
    edition: &str,
) -> Value {
    let model = chat["model"].as_str().unwrap_or("");
    let reasoning =
        model_info["isReasoning"].as_bool().unwrap_or(false) && chat["reasoning_effort"] != "none";
    let rows = chat["messages"].as_array().cloned().unwrap_or_default();
    let prompt = rows
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or("");
    let system: Vec<Value> = rows
        .iter()
        .filter(|m| m["role"] == "system")
        .filter_map(|m| m["content"].as_str())
        .map(|s| json!({"type":"text","text":s}))
        .collect();
    let mut parameters = json!({});
    for name in [
        "max_tokens",
        "temperature",
        "top_p",
        "stop",
        "reasoning_effort",
        "parallel_tool_calls",
        "tool_choice",
        "response_format",
    ] {
        if let Some(v) = chat.get(name) {
            parameters[name] = v.clone();
        }
    }
    if chat["reasoning_effort"] == "none" {
        parameters["enable_thinking"] = json!(false);
    }
    json!({"request_id":request_id,"request_set_id":request_id,"chat_record_id":request_id,"session_id":session,"stream":true,"chat_task":"FREE_INPUT","chat_context":{"text":prompt,"features":[],"extra":{"context":[],"modelConfig":{"key":model,"is_reasoning":reasoning},"originalContent":prompt},"chatPrompt":"","imageUrls":null},"is_reply":true,"is_retry":false,"source":1,"version":"3","agent_id":"agent_common","task_id":"common","session_type":if edition=="cn"{"qoderclicn"}else{"qodercli"},"aliyun_user_type":"","model_config":{"key":model,"display_name":model_info["name"].as_str().unwrap_or(model),"model":"","format":model_info["format"].as_str().unwrap_or("openai"),"is_vl":model_info["supportsImages"].as_bool().unwrap_or(false),"is_reasoning":reasoning,"api_key":"","url":"","source":"system","max_input_tokens":model_info["contextWindow"].as_u64().unwrap_or(200000)},"system":system,"messages":rows,"tools":chat.get("tools").cloned().unwrap_or(json!([])),"parameters":parameters,"business":{"id":request_id,"name":"chat","product":"cli","version":CLIENT_VERSION,"type":"agent","begin_at":chrono::Utc::now().timestamp_millis()}})
}

/// Qoder SSE: plain chunks, `statusCodeValue` envelopes and quota markers.
#[derive(Clone, Debug, PartialEq)]
pub struct QueueInfo {
    pub retry_after: u64,
    pub queue_count: Option<u64>,
    pub sub_code: Option<String>,
}
impl QueueInfo {
    pub fn label(&self) -> String {
        let count = self.queue_count.map(|c| format!("，队列 {c}")).unwrap_or_default();
        let code = self.sub_code.as_deref().map(|c| format!("（{c}）")).unwrap_or_default();
        format!("模型{code}排队中{count}，约 {} 秒后自动重试", self.retry_after)
    }
}

/// Qoder reports model queueing as a 403 envelope whose message is JSON with
/// code 10605 / isQueued; it is a wait signal, not an account rejection.
fn parse_queue_notice(body: &Value) -> Option<QueueInfo> {
    let outer: Value = serde_json::from_str(body.as_str()?).ok()?;
    let inner: Value = serde_json::from_str(outer.get("message")?.as_str()?).ok()?;
    if inner.get("code").and_then(Value::as_str) != Some("10605")
        && inner.get("isQueued").and_then(Value::as_bool) != Some(true)
    {
        return None;
    }
    Some(QueueInfo {
        retry_after: inner.get("retryAfterSeconds").and_then(Value::as_u64).unwrap_or(30).clamp(1, 120),
        queue_count: inner.get("queueCount").and_then(Value::as_u64),
        sub_code: inner
            .get("subCode")
            .or_else(|| outer.get("subCode"))
            .filter(|v| v.is_string())
            .map(safe_code),
    })
}

#[derive(Default)]
pub struct Parser {
    pending: Vec<u8>,
    frame: Vec<u8>,
    retained: usize,
    pub done: bool,
    pub finished: bool,
    /// Business status carried inside an HTTP 200 SSE envelope.
    pub business_status: Option<u16>,
    pub business_code: Option<String>,
    pub queue: Option<QueueInfo>,
}
impl Parser {
    /// Stream failure enriched with the business status/code seen so far.
    pub fn failure(&self, message: String) -> StreamFailure {
        StreamFailure {
            message,
            code: self.business_code.clone(),
            error: json!({"code": self.business_code}),
            status: self.business_status,
            retry_after: self.queue.as_ref().map(|q| q.retry_after),
        }
    }
}
impl Parser {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Option<Value>>, String> {
        self.retained += bytes.len();
        if self.retained > MAX_OUTPUT || self.pending.len() + bytes.len() > MAX_EVENT {
            return Err("上游流超过本地大小限制".into());
        }
        self.pending.extend_from_slice(bytes);
        let mut out = vec![];
        while let Some(n) = self.pending.iter().position(|b| *b == b'\n') {
            let mut line = self.pending.drain(..=n).collect::<Vec<_>>();
            while line.last().is_some_and(|b| *b == b'\r' || *b == b'\n') {
                line.pop();
            }
            if let Some(data) = line.strip_prefix(b"data:") {
                if !self.frame.is_empty() {
                    self.frame.push(b'\n');
                }
                self.frame
                    .extend_from_slice(data.strip_prefix(b" ").unwrap_or(data));
                if self.frame.len() > MAX_EVENT {
                    return Err("上游 SSE 事件过大".into());
                }
            }
            if !line.is_empty() || self.frame.is_empty() {
                continue;
            }
            let data = std::mem::take(&mut self.frame);
            if data == b"[DONE]" {
                if !self.finished {
                    return Err("上游没有返回有效 finish_reason".into());
                }
                self.done = true;
                out.push(None);
                break;
            }
            if data == b"[NOT_EXCEED_QUOTA]" {
                continue;
            }
            if data.starts_with(b"[EXCEED_QUOTA]") {
                return Err("Qoder 模型配额不足".into());
            }
            if data.starts_with(b"[NOTIFICATIONS]") {
                continue;
            }
            let mut v: Value = serde_json::from_slice(&data).map_err(|_| "上游 SSE JSON 无效")?;
            if let Some(status) = v["statusCodeValue"].as_u64() {
                if status != 200 {
                    self.business_status = u16::try_from(status).ok().filter(|s| (400..=599).contains(s));
                    self.business_code = Some(status.to_string());
                    if status == 403 {
                        if let Some(queue) = parse_queue_notice(&v["body"]) {
                            self.business_code = Some(
                                queue.sub_code.as_deref().map(|c| format!("10605/{c}")).unwrap_or_else(|| "10605".into()),
                            );
                            let label = queue.label();
                            self.queue = Some(queue);
                            return Err(label);
                        }
                    }
                    return Err(format!(
                        "Qoder 流式业务错误（{status}）；{}",
                        provider_error_hint(&v["body"]).unwrap_or(rejection_hint(self.business_status.unwrap_or(502)))
                    ));
                }
                let inner = v["body"].as_str().unwrap_or("").trim();
                if inner.is_empty() {
                    continue;
                }
                if inner == "[DONE]" {
                    if !self.finished {
                        return Err("上游没有返回有效 finish_reason".into());
                    }
                    self.done = true;
                    out.push(None);
                    break;
                }
                v = serde_json::from_str(inner).map_err(|_| "Qoder 流式信封内容无效")?;
            }
            if v.get("error").is_some_and(|v| !v.is_null())
                || v.get("code").is_some_and(|v| v.as_i64() != Some(0))
            {
                self.business_code = v.get("code").or_else(|| v["error"].get("code")).map(safe_code);
                self.business_status = self
                    .business_code
                    .as_ref()
                    .and_then(|s| s.parse::<u16>().ok())
                    .filter(|s| [401, 403, 429].contains(s));
                return Err(format!(
                    "Qoder 返回业务错误事件{}；{}",
                    self.business_code.as_ref().map(|c| format!("（{c}）")).unwrap_or_default(),
                    rejection_hint(self.business_status.unwrap_or(502))
                ));
            }
            for c in v["choices"].as_array().into_iter().flatten() {
                if c["finish_reason"].is_string() {
                    self.finished = true;
                }
            }
            out.push(Some(v));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_endpoints_only() {
        assert!(valid_endpoint("https://api2-v2.qoder.sh"));
        for s in [
            "http://api2.qoder.sh",
            "https://qoder.sh.evil.com",
            "https://user@api2.qoder.sh",
            "https://api2.qoder.sh/path",
        ] {
            assert!(!valid_endpoint(s));
        }
    }
    #[test]
    fn fragmented_sse_and_business_errors() {
        let mut p = Parser::default();
        assert!(p
            .feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},")
            .unwrap()
            .is_empty());
        assert_eq!(
            p.feed(b"\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\n\n")
                .unwrap()
                .len(),
            2
        );
        assert!(p.done);
        assert!(Parser::default()
            .feed(b"data: {\"code\":11128}\n\n")
            .is_err());
        assert!(Parser::default().feed(b"data: [DONE]\n\n").is_err());
    }
    #[test]
    fn account_catalog_deduplicates() {
        let rows =
            normalize_models(&json!({"qodercli":[{"key":"a","display_name":"A"},{"key":"a"}]}))
                .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "A");
    }
    #[test]
    fn text_history_is_scalar_while_user_images_are_preserved() {
        let mut chat = json!({"messages":[{"role":"assistant","content":[{"type":"text","text":"before tool"}],"tool_calls":[{"id":"c","type":"function","function":{"name":"echo","arguments":"{}"}}]},{"role":"tool","tool_call_id":"c","content":[{"type":"text","text":"first"},{"type":"text","text":"second"}]},{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"https://example.test/image.png"}}]}]});
        normalize_qoder_messages(&mut chat);
        assert_eq!(chat["messages"][0]["content"], "before tool");
        assert_eq!(chat["messages"][1]["content"], "first\nsecond");
        assert!(chat["messages"][2]["content"].is_array());
        assert_eq!(chat["messages"][0]["tool_calls"][0]["id"], "c");
    }
    #[test]
    fn legacy_transport_preserves_model_tools_and_token_limits() {
        let chat = json!({"model":"exact-model","messages":[{"role":"user","content":"a"}],"tools":[{"type":"function","function":{"name":"echo"}}],"max_tokens":321,"reasoning_effort":"none"});
        let body = legacy_body(
            &chat,
            &json!({"isReasoning":true,"supportsImages":true}),
            "r",
            "s",
            "cn",
        );
        assert_eq!(body["model_config"]["key"], "exact-model");
        assert_eq!(body["parameters"]["max_tokens"], 321);
        assert_eq!(body["parameters"]["enable_thinking"], false);
        assert_eq!(body["model_config"]["is_reasoning"], false);
        assert_eq!(body["tools"], chat["tools"]);
        assert_eq!(body["session_type"], "qoderclicn");
    }
    #[test]
    fn legacy_envelope_and_quota_events_are_not_misreported_as_success() {
        let chunk = json!({"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]});
        let envelope = json!({"statusCodeValue":200,"body":chunk.to_string()});
        let mut p = Parser::default();
        let events = p
            .feed(
                format!("data: [NOT_EXCEED_QUOTA]\n\ndata: {envelope}\n\ndata: [DONE]\n\n")
                    .as_bytes(),
            )
            .unwrap();
        assert_eq!(events.len(), 2);
        assert!(p.done);
        assert!(Parser::default()
            .feed(b"data: {\"statusCodeValue\":400,\"body\":\"provider rejected\"}\n\n")
            .is_err());
        assert!(Parser::default()
            .feed(b"data: [EXCEED_QUOTA] unavailable\n\n")
            .is_err());
    }
    #[test]
    fn stream_rejections_keep_business_status_code_and_queue_notice() {
        let mut p = Parser::default();
        assert!(p.feed(b"data: {\"statusCodeValue\":403,\"body\":\"denied\"}\n\n").is_err());
        let f = p.failure("x".into());
        assert_eq!((f.status, f.code.as_deref(), f.retry_after), (Some(403), Some("403"), None));
        let notice = json!({"message": json!({"code":"10605","retryAfterSeconds":7,"queueCount":3,"subCode":"busy"}).to_string()});
        let mut p = Parser::default();
        let frame = format!("data: {}\n\n", json!({"statusCodeValue":403,"body":notice.to_string()}));
        let e = p.feed(frame.as_bytes()).unwrap_err();
        assert!(e.contains("排队中，队列 3") && e.contains("7 秒"), "{e}");
        let f = p.failure(e);
        assert_eq!((f.status, f.code.as_deref(), f.retry_after), (Some(403), Some("10605/busy"), Some(7)));
        let mut p = Parser::default();
        assert!(p.feed(b"data: {\"code\":429,\"error\":{\"code\":429}}\n\n").is_err());
        assert_eq!(p.failure(String::new()).status, Some(429));
    }
    #[test]
    fn tool_only_assistant_matches_native_contract() {
        let input = json!({"model":"dfmodel","max_tokens":64,"messages":[{"role":"user","content":"use tools"},{"role":"assistant","content":[{"type":"tool_use","id":"call-1","name":"echo","input":{"x":1}},{"type":"tool_use","id":"call-2","name":"echo","input":{"x":2}}]},{"role":"user","content":[{"type":"text","text":"continue"},{"type":"tool_result","tool_use_id":"call-1","content":"one"},{"type":"text","text":"please"},{"type":"tool_result","tool_use_id":"call-2","content":"two"}]}]});
        let mut chat = anthropic::to_chat(&input).unwrap();
        normalize_qoder_messages(&mut chat);
        assert_eq!(chat["messages"][1]["content"], "");
        assert_eq!(chat["messages"][1]["tool_calls"][0]["index"], 0);
        assert_eq!(chat["messages"][1]["tool_calls"][1]["index"], 1);
        assert_eq!(chat["messages"][2]["tool_call_id"], "call-1");
        assert_eq!(chat["messages"][3]["tool_call_id"], "call-2");
        assert_eq!(chat["messages"][4]["content"], "continue\nplease");
        assert!(validate_tool_history(&chat).is_ok());
        let wire = legacy_body(&chat, &json!({"id":"dfmodel"}), "test", "test", "cn");
        assert_eq!(wire["messages"], chat["messages"]);
    }
    #[test]
    fn orphan_results_and_incomplete_parallel_calls_are_rejected_locally() {
        for messages in [
            json!([{"role":"tool","tool_call_id":"missing","content":"result"}]),
            json!([{"role":"assistant","content":null,"tool_calls":[{"id":"call-1"},{"id":"call-2"}]},{"role":"tool","tool_call_id":"call-1","content":"one"},{"role":"user","content":"continue"}]),
            json!([{"role":"assistant","content":"","tool_calls":[{"id":"a"},{"id":"a"}]}]),
        ] {
            assert!(validate_tool_history(&json!({"messages":messages})).is_err());
        }
        assert_eq!(
            provider_error_hint(&json!("Messages with role 'tool' must be a response to a preceding message with 'tool_calls'")),
            Some("工具历史校验失败：工具结果缺少对应的前置 tool_calls")
        );
    }
    #[test]
    fn quota_rank_sums_known_parts_and_uses_addon_expiry() {
        let rank = credit_rank(Some(&json!({"usage":{"userQuota":{"remaining":100,"total":300},"addOnQuota":{"remaining":50,"total":50},"expiresAt":1_900_000_000_000_i64},"updatedAt":5})));
        assert_eq!(rank.remaining, Some(150.0));
        assert_eq!(rank.earliest_expiry, Some(1_900_000_000_000));
        let never = credit_rank(Some(&json!({"usage":{"userQuota":{"remaining":1},"addOnQuota":{"remaining":0},"expiresAt":253402214400000_i64}})));
        assert_eq!(never.earliest_expiry, None);
        assert_eq!(credit_rank(None).remaining, None);
    }
    #[tokio::test]
    async fn model_server_send_normalizes_stream_and_maps_rejections() {
        use axum::{routing::post, Json, Router};
        let upstream = Router::new().route(
            "/model/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                if body["model"] == "denied" {
                    return (
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        [("retry-after", "7"), ("content-type", "application/json")],
                        json!({"code":"quota"}).to_string(),
                    );
                }
                let wire = format!(
                    "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"index":0,"delta":{"content":"OK"},"finish_reason":"stop"}]}),
                    json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":1}})
                );
                (
                    axum::http::StatusCode::OK,
                    [("retry-after", "0"), ("content-type", "text/event-stream")],
                    wire,
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let account = qcore::Account {
            edition: "cn".into(),
            uid: "u".into(),
            token: "never-real".into(),
            ..Default::default()
        };
        let provider = Provider::fixture(
            vec![account],
            Config {
                transport: "modelServer".into(),
                cn_endpoint: url.clone(),
                intl_endpoint: url,
                disabled_accounts: vec![],
            },
            HashMap::from([("cn".into(), vec![json!({"id":"m","name":"M"})])]),
        );
        let input = |model: &str| SendInput {
            body: json!({"model":model,"messages":[{"role":"user","content":"hi"}]}),
            headers: http::HeaderMap::new(),
            request_id: "r".into(),
            session_id: None,
            effort_input: Value::Null,
            client_profile: None,
        };
        let mut details = RequestDetails::default();
        let ok = provider
            .send("cn:u", json!({"id":"m"}), input("m"), &mut details, Instant::now())
            .await
            .ok()
            .unwrap();
        let events: Vec<_> = ok.events.collect().await;
        assert!(matches!(events.last(), Some(Ok(UpstreamEvent::Done))));
        assert_eq!(events.len(), 3);
        let failure = provider
            .send(
                "cn:u",
                json!({"id":"denied"}),
                input("denied"),
                &mut details,
                Instant::now(),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(failure.status, 429);
        assert_eq!(failure.effect, AccountEffect::Cooldown(7));
        server.abort();
    }
}
