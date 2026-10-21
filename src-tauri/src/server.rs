//! Loopback HTTP gateway: Chat Completions, Anthropic Messages (+count_tokens)
//! and stateless Responses over every provider. Paths may be scoped to a
//! provider and/or region, e.g. `/v1`, `/cn/v1`, `/qoder/v1`, `/trae/intl/v1`,
//! and the ZCode compatibility profile `/zcode/...`.
use crate::{
    app::Shared,
    catalog::{self, Scope},
    logs::RequestLog,
    scheduler::Exec,
};
use agentdock_core::{
    anthropic,
    diagnostics::{self, RequestDetails},
    provider::{AccountEffect, ProviderKind, SendFailure, SendInput},
    response_tools::ToolMap,
    responses,
    stream::{Aggregate, UpstreamEvent},
};
use axum::{
    body::{Body, Bytes},
    extract::{OriginalUri, Request, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Chat,
    Messages,
    CountTokens,
    Responses,
    Models,
    ModelDetail,
}

#[derive(Debug, PartialEq)]
pub struct Endpoint {
    pub kind: Kind,
    pub scope: Scope,
    pub zcode: bool,
    pub model: Option<String>,
}

fn decode(segment: &str) -> String {
    percent_encoding::percent_decode_str(segment)
        .decode_utf8_lossy()
        .into_owned()
}

/// Parse `[zcode/][provider/][region/][v1/[v1/]]<endpoint>`.
pub fn parse(path: &str) -> Option<Endpoint> {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut i = 0;
    let zcode = segs.first() == Some(&"zcode");
    if zcode {
        i += 1;
    }
    let mut scope = Scope::default();
    if let Some(p) = segs.get(i).and_then(|s| ProviderKind::parse(s)) {
        scope.provider = Some(p);
        i += 1;
    }
    if let Some(r) = segs.get(i).filter(|s| ["cn", "intl"].contains(*s)) {
        scope.region = Some(r.to_string());
        i += 1;
    }
    for _ in 0..2 {
        if segs.get(i) == Some(&"v1") {
            i += 1;
        }
    }
    let rest = &segs[i..];
    let (kind, model) = match rest {
        ["chat", "completions"] => (Kind::Chat, None),
        ["messages"] => (Kind::Messages, None),
        ["messages", "count_tokens"] => (Kind::CountTokens, None),
        ["responses"] => (Kind::Responses, None),
        ["models"] => (Kind::Models, None),
        ["models", id @ ..] if !id.is_empty() => (
            Kind::ModelDetail,
            Some(id.iter().map(|s| decode(s)).collect::<Vec<_>>().join("/")),
        ),
        _ => return None,
    };
    Some(Endpoint {
        kind,
        scope,
        zcode,
        model,
    })
}

fn protocol(ep: &Endpoint) -> &'static str {
    match (ep.kind, ep.zcode) {
        (Kind::Messages, false) => "anthropic-messages",
        (Kind::Messages, true) => "zcode-messages",
        (Kind::Chat, false) => "openai-chat",
        (Kind::Chat, true) => "zcode-chat",
        (Kind::Responses, false) => "openai-responses",
        (Kind::Responses, true) => "zcode-responses",
        (Kind::CountTokens, _) => "anthropic-count-tokens",
        _ => "models",
    }
}
fn anthropic_style(kind: Kind) -> bool {
    matches!(kind, Kind::Messages | Kind::CountTokens | Kind::ModelDetail)
}

pub fn openai_error(status: u16, message: &str, code: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
        Json(json!({"error":{"message":message,"type":"agentdock_error","code":code}})),
    )
        .into_response()
}
pub fn anthropic_error(status: u16, message: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
        Json(anthropic::error_value(status, message)),
    )
        .into_response()
}
fn error_for(kind: Kind, status: u16, message: &str, code: &str) -> Response {
    if anthropic_style(kind) {
        anthropic_error(status, message)
    } else {
        openai_error(status, message, code)
    }
}
/// Client-facing failure, keeping only bounded allowlisted upstream fields.
fn failure_response(kind: Kind, f: &SendFailure) -> Response {
    let status = StatusCode::from_u16(f.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut value = if anthropic_style(kind) {
        anthropic::error_value(f.status, &f.message)
    } else {
        json!({"error":{"message":f.message,"type":if f.code=="upstream_channel_denied"{"upstream_channel_denied"}else{"upstream_error"},"code":f.code}})
    };
    for key in ["code", "request_id"] {
        if let Some(v) = f.error.get(key).filter(|v| v.is_string() || v.is_number()) {
            value["error"][key] = match v.as_str() {
                Some(s) => json!(diagnostics::bounded(s, 256)),
                None => v.clone(),
            };
        }
    }
    if anthropic_style(kind) {
        if let Some(t) = &f.anthropic_type {
            value["error"]["type"] = json!(t);
        }
    }
    if let Some(r) = f.retryable {
        value["error"]["retryable"] = json!(r);
    }
    (status, Json(value)).into_response()
}

pub fn router(hub: Shared) -> Router {
    Router::new()
        .route("/health", get(|| async { Json(json!({"ok":true,"service":"agentdock"})) }))
        .fallback(dispatch)
        .with_state(hub)
}

fn authorized(headers: &HeaderMap, key: &str) -> bool {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|v| v == key);
    let api_key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == key);
    bearer || api_key
}

async fn dispatch(State(hub): State<Shared>, OriginalUri(uri): OriginalUri, request: Request) -> Response {
    let Some(ep) = parse(uri.path()) else {
        return openai_error(404, "接口不存在：支持 /v1、/cn/v1、/intl/v1、/<端>/v1 与 /zcode", "not_found");
    };
    let method = request.method().clone();
    let headers = request.headers().clone();
    // Authenticate before reading any body.
    if !authorized(&headers, &hub.api_key()) {
        return error_for(ep.kind, 401, "本地 API Key 不正确", "invalid_api_key");
    }
    if hub.settings().reject_browser_origin
        && headers
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|o| o.starts_with("http://") || o.starts_with("https://"))
    {
        return error_for(ep.kind, 403, "不接受网页跨域请求（可在接入设置中允许）", "origin_rejected");
    }
    match ep.kind {
        Kind::Models if method == Method::GET => return models(&hub, &ep),
        Kind::ModelDetail if method == Method::GET => return model_detail(&hub, &ep),
        Kind::Models | Kind::ModelDetail => return openai_error(405, "请求方法不支持", "method_not_allowed"),
        _ if method != Method::POST => return openai_error(405, "请求方法不支持", "method_not_allowed"),
        _ => {}
    }
    let path = uri.path().to_owned();
    let bytes = match axum::body::to_bytes(request.into_body(), agentdock_core::MAX_REQUEST_BODY_BYTES).await {
        Ok(b) => b,
        Err(_) => {
            let mut details = RequestDetails::incoming(protocol(&ep), &path, &Value::Null);
            details.request_summary["bodyBytes"] = json!(headers
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok()));
            details.request_summary["bodyLimitBytes"] = json!(agentdock_core::MAX_REQUEST_BODY_BYTES);
            details.local_error("request_body", agentdock_core::REQUEST_TOO_LARGE_MESSAGE);
            reject(&hub, &ep, "unknown", 413, "request_too_large", details);
            return error_for(ep.kind, 413, agentdock_core::REQUEST_TOO_LARGE_MESSAGE, "request_too_large");
        }
    };
    let input: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return error_for(ep.kind, 400, "请求体不是有效 JSON", "invalid_request"),
    };
    if ep.kind == Kind::CountTokens {
        return count_tokens(&hub, &ep, input);
    }
    execute(hub, ep, path, headers, input).await
}

/// Record a request that never reached an account.
fn reject(hub: &Shared, ep: &Endpoint, model: &str, status: u16, outcome: &str, details: RequestDetails) {
    let alias = details.request_summary["modelAlias"].as_str().unwrap_or(model).to_owned();
    hub.logs.push(RequestLog {
        id: uuid::Uuid::new_v4().simple().to_string(),
        time: chrono::Utc::now().to_rfc3339(),
        protocol: protocol(ep).into(),
        path: details.client_path.clone(),
        requested_model: diagnostics::bounded(model, 128),
        model: diagnostics::bounded(&alias, 128),
        real_model: details.request_summary["realModel"].as_str().unwrap_or("").into(),
        route_id: details.request_summary["routeId"].as_str().unwrap_or("").into(),
        status,
        outcome: outcome.into(),
        message: details.error["message"].as_str().unwrap_or("").into(),
        strategy: format!("{:?}", hub.settings().strategy),
        details: serde_json::to_value(&details).unwrap_or_default(),
        ..RequestLog::default()
    });
}

fn models(hub: &Shared, ep: &Endpoint) -> Response {
    let settings = hub.settings();
    let routes = hub.routes(&settings);
    let data: Vec<Value> = routes
        .iter()
        .filter(|r| !r.disabled && !r.scoped(&ep.scope).is_empty())
        .map(|r| {
            let v = r.view(&ep.scope);
            let first = r.scoped(&ep.scope)[0];
            json!({"id":r.alias,"object":"model","type":"model","created":0,"created_at":"1970-01-01T00:00:00Z",
                "owned_by":"agentdock","name":r.alias,"display_name":r.alias,"real_model":first.id,
                "providers":v["providers"],"regions":v["regions"],"upstreams":v["targets"],
                "supports_images":v["supportsImages"],"supports_tools":v["supportsTools"],"context_window":v["contextWindow"]})
        })
        .collect();
    Json(json!({"object":"list","first_id":data.first().map(|m|&m["id"]),"last_id":data.last().map(|m|&m["id"]),"has_more":false,"data":data})).into_response()
}

fn model_detail(hub: &Shared, ep: &Endpoint) -> Response {
    let settings = hub.settings();
    let routes = hub.routes(&settings);
    let requested = ep.model.as_deref().unwrap_or("");
    let Ok(route) = catalog::resolve(&routes, requested) else {
        return anthropic_error(404, "模型不存在");
    };
    let targets = route.scoped(&ep.scope);
    if targets.is_empty() {
        return anthropic_error(404, "所选端或地区不提供此模型");
    }
    let v = route.view(&ep.scope);
    Json(json!({"id":route.alias,"type":"model","display_name":route.alias,"real_model":targets[0].id,
        "providers":v["providers"],"regions":v["regions"],"upstreams":v["targets"],"created_at":"1970-01-01T00:00:00Z",
        "max_input_tokens":v["contextWindow"],"max_tokens":targets.iter().filter_map(|t| t.max_output).min()}))
    .into_response()
}

fn count_tokens(hub: &Shared, ep: &Endpoint, mut input: Value) -> Response {
    if !input.is_object() {
        return anthropic_error(400, "请求体必须是 JSON 对象");
    }
    input["max_tokens"] = json!(1);
    input["stream"] = json!(false);
    let chat = match anthropic::to_chat(&input) {
        Ok(v) => v,
        Err(e) => return anthropic_error(400, &e),
    };
    let settings = hub.settings();
    let routes = hub.routes(&settings);
    match catalog::resolve(&routes, chat["model"].as_str().unwrap_or("")) {
        Ok(r) if !r.scoped(&ep.scope).is_empty() => {}
        Ok(_) => return anthropic_error(400, "所选端或地区不提供此模型"),
        Err(e) => return anthropic_error(400, &e),
    }
    // No exact upstream tokenizer exists: return a labelled conservative
    // estimate instead of making a billable inference request.
    let estimate = (chat["messages"].to_string().chars().count()
        + chat.get("tools").map_or(0, |v| v.to_string().chars().count()))
    .div_ceil(3)
        + 16;
    let mut response = Json(json!({"input_tokens":estimate})).into_response();
    response
        .headers_mut()
        .insert("x-token-count-method", "estimate".parse().unwrap());
    response
}

/// Holds an account slot for one request; logs the outcome when dropped
/// (including client disconnects mid-stream).
pub struct Lease {
    hub: Shared,
    key: String,
    pub log: RequestLog,
    pub details: RequestDetails,
    start: Instant,
    finished: bool,
}
impl Lease {
    pub fn elapsed_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }
    pub fn finish(&mut self, status: u16, outcome: &str, message: &str) {
        self.log.status = status;
        self.log.outcome = outcome.into();
        if !message.is_empty() {
            self.log.message = diagnostics::bounded(message, 512);
        }
        self.finished = true;
    }
    fn first_byte(&mut self) {
        if self.details.first_byte_ms.is_none() {
            self.details.first_byte_ms = Some(self.elapsed_ms());
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if !self.finished {
            self.log.status = 499;
            self.log.outcome = "client_disconnected".into();
            self.log.message = "客户端断开或请求取消".into();
        }
        {
            let mut rt = self.hub.runtime.lock().unwrap();
            if let Some(n) = rt.in_flight.get_mut(&self.key) {
                *n = n.saturating_sub(1);
            }
        }
        self.hub.active.lock().unwrap().remove(&self.log.id);
        self.log.duration_ms = self.start.elapsed().as_millis();
        self.log.first_byte_ms = self.details.first_byte_ms;
        if self.details.usage.as_object().is_some_and(|u| !u.is_empty()) {
            self.log.usage = self.details.usage.clone();
        }
        self.log.details = serde_json::to_value(&self.details).unwrap_or_default();
        self.hub.logs.push(std::mem::take(&mut self.log));
    }
}

enum Converter {
    Chat,
    Messages(anthropic::Stream),
    Responses(responses::Stream),
}
impl Converter {
    fn succeeded(&self) -> bool {
        match self {
            Self::Chat => true,
            Self::Messages(s) => s.succeeded(),
            Self::Responses(s) => s.succeeded(),
        }
    }
    fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Self::Chat => bytes.to_vec(),
            Self::Messages(s) => s.feed(bytes),
            Self::Responses(s) => s.feed(bytes),
        }
    }
    fn fail(&mut self, e: &str) -> Vec<u8> {
        match self {
            Self::Chat => format!("data: {}\n\n", json!({"error":{"message":e,"type":"upstream_error"}})).into_bytes(),
            Self::Messages(s) => s.fail(e),
            Self::Responses(s) => s.fail(e),
        }
    }
    fn ping(&self) -> &'static [u8] {
        match self {
            Self::Messages(_) => b"event: ping\ndata: {\"type\":\"ping\"}\n\n",
            _ => b": keep-alive\n\n",
        }
    }
}

fn effort_of(kind: Kind, input: &Value) -> Value {
    const EFFORTS: [&str; 8] = ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"];
    match kind {
        Kind::Messages => {
            let effort = input["output_config"]["effort"]
                .as_str()
                .or_else(|| input["reasoning_effort"].as_str())
                .filter(|e| EFFORTS.contains(e));
            json!({"output_config":{"effort":effort},"thinking":{"type":if input["thinking"]["type"]=="disabled" {"disabled"} else {"enabled"}}})
        }
        Kind::Responses => {
            let effort = input["reasoning"]["effort"].as_str().filter(|e| EFFORTS.contains(e));
            json!({"output_config":{"effort":effort}})
        }
        _ => Value::Null,
    }
}

pub async fn execute(hub: Shared, ep: Endpoint, path: String, headers: HeaderMap, input: Value) -> Response {
    let kind = ep.kind;
    let mut details = RequestDetails::incoming(protocol(&ep), &path, &input);
    if ep.zcode {
        // Local provenance only; never disguise a client or strip its instructions.
        details.request_summary["clientProfile"] = json!("zcode");
    }
    if let Some(p) = ep.scope.provider {
        details.request_summary["scopeProvider"] = json!(p.key());
    }
    if let Some(r) = &ep.scope.region {
        details.request_summary["scopeRegion"] = json!(r);
    }
    let requested = input["model"].as_str().unwrap_or("").to_owned();
    let mut tools = ToolMap::default();
    let converted = match kind {
        Kind::Messages => anthropic::to_chat(&input),
        Kind::Responses => responses::to_chat_with_tools(&input).map(|(v, t)| {
            tools = t;
            v
        }),
        _ => responses::normalize_chat(input.clone()),
    };
    let mut body = match converted {
        Ok(v) => v,
        Err(e) => {
            details.local_error("conversion", &e);
            reject(&hub, &ep, &requested, 400, "invalid_request", details);
            return error_for(kind, 400, &e, "invalid_request");
        }
    };
    if kind == Kind::Responses {
        details.request_summary["inputItemCount"] = json!(input["input"].as_array().map(Vec::len));
        details.request_summary["hasInstructions"] = json!(input["instructions"].is_string());
    }
    if kind == Kind::Messages && input["extra_vars"].is_object() {
        body["extra_vars"] = input["extra_vars"].clone();
    }
    if !body["messages"].as_array().is_some_and(|a| !a.is_empty()) {
        details.local_error("validation", "messages 必须为非空数组");
        reject(&hub, &ep, &requested, 400, "invalid_request", details);
        return error_for(kind, 400, "messages 必须为非空数组", "invalid_request");
    }
    if body.get("n").is_some_and(|n| n != 1) {
        return error_for(kind, 400, "当前仅支持 n=1", "invalid_request");
    }
    let stream = input["stream"].as_bool().unwrap_or(false);
    let effort_input = effort_of(kind, &input);
    if effort_input.is_object() {
        details.request_summary["effortInput"] = effort_input.clone();
    }
    let settings = hub.settings();
    let routes = hub.routes(&settings);
    let route = match catalog::resolve(&routes, &requested) {
        Ok(r) => r.clone(),
        Err(e) => {
            details.local_error("model", &e);
            reject(&hub, &ep, &requested, 400, "model_not_found", details);
            return error_for(kind, 400, &e, "model_not_found");
        }
    };
    details.request_summary["requestedModel"] = json!(diagnostics::bounded(&requested, 128));
    details.request_summary["modelAlias"] = json!(route.alias);
    details.request_summary["routeId"] = json!(route.route_id);
    let targets: Vec<_> = route.scoped(&ep.scope);
    if targets.is_empty() {
        let message = if route.all_targets.is_empty() {
            "此模型映射的上游当前不在目录中，请同步模型目录"
        } else {
            "所选端或地区不提供此模型（或已在接入设置中停用）"
        };
        details.local_error("model", message);
        reject(&hub, &ep, &requested, 400, "model_not_available", details);
        return error_for(kind, 400, message, "model_not_available");
    }
    let pinned = match headers.get("x-account-id").and_then(|v| v.to_str().ok()) {
        Some(value) => match hub.resolve_account(value) {
            Ok(k) => Some(k),
            Err(e) => {
                details.local_error("account_selection", &e);
                reject(&hub, &ep, &requested, 503, "no_available_account", details);
                return error_for(kind, 503, &e, "no_available_account");
            }
        },
        None => settings.pinned.get(&ep.scope.key()).filter(|s| !s.is_empty()).cloned(),
    };
    let accounts = hub.pool_accounts();
    let now = chrono::Utc::now().timestamp_millis();
    let picked = {
        let check = |a: &agentdock_core::provider::PoolAccount, t: &agentdock_core::provider::UpstreamModel| {
            let mut probe = body.clone();
            probe["model"] = json!(t.id);
            hub.eligibility(a, t, &probe, &headers)
        };
        let mut rt = hub.runtime.lock().unwrap();
        crate::scheduler::select(&accounts, &targets, &settings, &mut rt, &route.route_id, pinned.as_deref(), now, &check)
    };
    let pick = match picked {
        Ok(p) => p,
        Err(e) => {
            details.local_error("account_selection", &e);
            reject(&hub, &ep, &requested, 503, "no_available_account", details);
            return if kind == Kind::Messages {
                let mut v = anthropic::error_value(503, &e);
                v["error"]["type"] = json!("overloaded_error");
                (StatusCode::SERVICE_UNAVAILABLE, Json(v)).into_response()
            } else {
                openai_error(503, &e, "no_available_account")
            };
        }
    };
    let id = uuid::Uuid::new_v4().simple().to_string();
    body["model"] = json!(pick.target.id);
    details.request_summary["realModel"] = json!(pick.target.id);
    details.request_summary["provider"] = json!(pick.account.provider.key());
    details.request_summary["region"] = json!(pick.account.region);
    if !pick.skipped.is_empty() {
        details.request_summary["skipped"] = pick
            .skipped
            .iter()
            .map(|((provider, region, reason), count)| json!({"provider": provider, "region": region, "reason": reason, "count": count}))
            .collect();
    }
    let log = RequestLog {
        id: id.clone(),
        time: chrono::Utc::now().to_rfc3339(),
        protocol: protocol(&ep).into(),
        path: path.clone(),
        requested_model: diagnostics::bounded(&requested, 128),
        model: route.alias.clone(),
        route_id: route.route_id.clone(),
        real_model: pick.target.id.clone(),
        model_name: pick.target.name.clone(),
        provider: pick.account.provider.key().into(),
        region: pick.account.region.clone(),
        account_id: pick.account.id.clone(),
        account_name: pick.account.name.clone(),
        status: 0,
        outcome: "running".into(),
        strategy: format!("{:?}", settings.strategy),
        stream,
        ..RequestLog::default()
    };
    hub.active.lock().unwrap().insert(id.clone(), log.clone());
    let mut lease = Lease {
        hub: hub.clone(),
        key: pick.account.key(),
        log,
        details,
        start: Instant::now(),
        finished: false,
    };
    let send = SendInput {
        body,
        headers: headers.clone(),
        request_id: id.clone(),
        session_id: agentdock_core::session::client_session(&headers, &input),
        effort_input,
        client_profile: ep.zcode.then(|| "zcode".into()),
    };
    let started = lease.start;
    let result = match pick.exec {
        Exec::Workbuddy(config) => {
            workbuddy::upstream::send(&hub.wb, &pick.account.id, config, send, &mut lease.details, started).await
        }
        Exec::Qoder(info) => hub.qoder.send(&pick.account.id, info, send, &mut lease.details, started).await,
        Exec::Trae(config) => hub.trae.send(&pick.account.id, config, send, &mut lease.details, started).await,
    };
    let ok = match result {
        Ok(ok) => ok,
        Err(f) => {
            {
                let mut rt = hub.runtime.lock().unwrap();
                match f.effect {
                    AccountEffect::Cooldown(s) => {
                        rt.cooldown.insert(lease.key.clone(), now + s as i64 * 1000);
                    }
                    AccountEffect::AuthFailed => {
                        rt.auth_failed.insert(lease.key.clone());
                    }
                    AccountEffect::None => {}
                }
            }
            lease.log.upstream_code = f.error["code"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| f.error["code"].as_i64().map(|c| c.to_string()));
            if lease.details.error_stage.is_none() {
                lease.details.local_error("upstream", &f.message);
            }
            let outcome = if f.code == "upstream_error" || f.code == "upstream_channel_denied" {
                "upstream_rejected"
            } else {
                f.code.as_str()
            }
            .to_owned();
            lease.finish(f.status, &outcome, &f.message);
            return failure_response(kind, &f);
        }
    };
    let alias = route.alias.clone();
    let mut events = ok.events;
    if !stream {
        let mut aggregate = Aggregate::new(&alias);
        let mut done = false;
        let mut failure: Option<agentdock_core::stream::StreamFailure> = None;
        while let Some(event) = events.next().await {
            lease.first_byte();
            match event {
                Ok(UpstreamEvent::Chunk(chunk)) => {
                    lease.details.response_bytes += chunk.to_string().len();
                    if let Err(e) = aggregate.push(&chunk) {
                        failure = Some(agentdock_core::stream::StreamFailure::new(e));
                        break;
                    }
                }
                Ok(UpstreamEvent::Done) => {
                    done = true;
                    break;
                }
                Err(f) => {
                    failure = Some(f);
                    break;
                }
            }
        }
        if failure.is_none() && (!done || !aggregate.finished) {
            failure = Some(agentdock_core::stream::StreamFailure::new("上游流未完整结束，不生成伪造的成功响应"));
        }
        lease.details.usage = diagnostics::usage(&aggregate.usage());
        if let Some(f) = failure {
            record_stream_failure(&mut lease, &f);
            lease.finish(502, "stream_error", &f.message);
            let mut sf = SendFailure::local(502, "upstream_error", f.message.clone());
            sf.error = json!({"code": f.code});
            return failure_response(kind, &sf);
        }
        let completion = aggregate.finish();
        lease.details.stream_complete = true;
        lease.details.finish_reason = completion["choices"][0]["finish_reason"]
            .as_str()
            .map(agentdock_core::stream::normalized_finish_reason);
        let result = match kind {
            Kind::Messages => anthropic::from_completion(&completion, &alias),
            Kind::Responses => responses::from_completion_with_tools(&completion, &alias, tools),
            _ => Ok(completion.clone()),
        };
        return match result {
            Ok(v) => {
                lease.finish(200, "completed", "");
                Json(v).into_response()
            }
            Err(e) => {
                lease.details.local_error("conversion", &e);
                lease.finish(502, "invalid_upstream_response", &e);
                error_for(kind, 502, &e, "invalid_upstream_response")
            }
        };
    }
    let mut converter = match kind {
        Kind::Messages => Converter::Messages(anthropic::Stream::new(&alias)),
        Kind::Responses => Converter::Responses(responses::Stream::with_tools(&alias, tools)),
        _ => Converter::Chat,
    };
    let created = chrono::Utc::now().timestamp();
    let chunk_id = format!("chatcmpl-{id}");
    let output = async_stream::stream! {
        let mut done = false;
        let mut failure: Option<agentdock_core::stream::StreamFailure> = None;
        loop {
            let next = match tokio::time::timeout(Duration::from_secs(15), events.next()).await {
                Ok(n) => n,
                Err(_) => {
                    yield Ok::<Bytes, std::convert::Infallible>(Bytes::from_static(converter.ping()));
                    continue;
                }
            };
            let Some(event) = next else { break };
            lease.first_byte();
            match event {
                Ok(UpstreamEvent::Chunk(mut chunk)) => {
                    chunk["model"] = json!(alias);
                    if chunk.get("object").is_none() {
                        chunk["object"] = json!("chat.completion.chunk");
                    }
                    if chunk.get("id").is_none() {
                        chunk["id"] = json!(chunk_id);
                    }
                    if chunk.get("created").is_none() {
                        chunk["created"] = json!(created);
                    }
                    if chunk["usage"].is_object() {
                        lease.details.usage = diagnostics::usage(&chunk["usage"]);
                    }
                    if let Some(reason) = chunk["choices"].as_array().into_iter().flatten().find_map(|c| c["finish_reason"].as_str()) {
                        lease.details.finish_reason = Some(agentdock_core::stream::normalized_finish_reason(reason));
                    }
                    let wire = format!("data: {chunk}\n\n");
                    lease.details.response_bytes += wire.len();
                    let out = converter.feed(wire.as_bytes());
                    if !out.is_empty() {
                        yield Ok(Bytes::from(out));
                    }
                }
                Ok(UpstreamEvent::Done) => {
                    done = true;
                    let out = converter.feed(b"data: [DONE]\n\n");
                    if !out.is_empty() {
                        yield Ok(Bytes::from(out));
                    }
                    break;
                }
                Err(f) => {
                    failure = Some(f);
                    break;
                }
            }
        }
        if failure.is_none() && !done {
            failure = Some(agentdock_core::stream::StreamFailure::new("上游流缺少完整结束标记"));
        }
        if failure.is_none() && !converter.succeeded() {
            failure = Some(agentdock_core::stream::StreamFailure::new("协议响应适配失败"));
        }
        match failure {
            Some(f) => {
                record_stream_failure(&mut lease, &f);
                yield Ok(Bytes::from(converter.fail(&f.message)));
                lease.finish(502, "stream_error", &f.message);
            }
            None => {
                lease.details.stream_complete = true;
                lease.finish(200, "completed", "");
            }
        }
    };
    Response::builder()
        .status(200)
        .header("content-type", "text/event-stream; charset=utf-8")
        .header("cache-control", "no-cache")
        .header("x-accel-buffering", "no")
        .header("x-request-id", &id)
        .body(Body::from_stream(output))
        .unwrap()
}

fn record_stream_failure(lease: &mut Lease, f: &agentdock_core::stream::StreamFailure) {
    lease.log.upstream_code = f.code.clone();
    if f.error.is_object() {
        lease.details.error_stage = Some("upstream".into());
        lease.details.error = f.error.clone();
        if lease.details.error["message"].is_null() {
            lease.details.error["message"] = json!(diagnostics::bounded(&f.message, 512));
        }
    } else {
        lease.details.local_error("stream", &f.message);
    }
}

/// Bind the loopback listener and serve until stopped.
pub async fn start(hub: Shared) -> Result<u16, String> {
    if hub.running() {
        return Ok(hub.server.lock().unwrap().as_ref().unwrap().port);
    }
    let settings = hub.settings();
    let wb_scope = settings.scope(ProviderKind::Workbuddy);
    let use_sdk = hub.wb.store.lock().unwrap().settings.use_device_sdk;
    if wb_scope.enabled && use_sdk && !hub.wb.store.lock().unwrap().accounts.is_empty() {
        // Await device authorization before publishing the listener as ready.
        if let Err(e) = hub.wb.official_transport.prepare_device().await {
            *hub.error.lock().unwrap() = format!("WorkBuddy 设备 SDK：{e}");
        }
    }
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, settings.port))
        .await
        .map_err(|_| {
            let e = format!("端口 {} 已占用或无法监听", settings.port);
            *hub.error.lock().unwrap() = e.clone();
            e
        })?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut server = hub.server.lock().unwrap();
        if server.is_some() {
            return Ok(port);
        }
        *server = Some(crate::app::ServerHandle { port, stop: tx });
    }
    hub.error.lock().unwrap().clear();
    let app = router(hub.clone());
    let state = hub.clone();
    tokio::spawn(async move {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
        state.server.lock().unwrap().take();
        if result.is_err() {
            *state.error.lock().unwrap() = "本地 HTTP 服务意外退出".into();
        }
    });
    Ok(port)
}

pub fn stop(hub: &Shared) {
    if let Some(handle) = hub.server.lock().unwrap().take() {
        let _ = handle.stop.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_cover_scopes_profiles_and_aliases() {
        let ep = parse("/v1/chat/completions").unwrap();
        assert_eq!((ep.kind, ep.scope.clone(), ep.zcode), (Kind::Chat, Scope::default(), false));
        assert_eq!(parse("/v1/v1/messages?beta=true".split('?').next().unwrap()).unwrap().kind, Kind::Messages);
        assert_eq!(parse("/messages/count_tokens").unwrap().kind, Kind::CountTokens);
        let ep = parse("/trae/cn/v1/chat/completions").unwrap();
        assert_eq!(ep.scope.provider, Some(ProviderKind::Trae));
        assert_eq!(ep.scope.region.as_deref(), Some("cn"));
        let ep = parse("/intl/v1/models").unwrap();
        assert_eq!(ep.scope.region.as_deref(), Some("intl"));
        assert!(ep.scope.provider.is_none());
        let ep = parse("/zcode/v1/responses").unwrap();
        assert!(ep.zcode);
        assert_eq!(protocol(&ep), "zcode-responses");
        let ep = parse("/qoder/v1/models/Hy3%20%5Bhy3%5D").unwrap();
        assert_eq!(ep.model.as_deref(), Some("Hy3 [hy3]"));
        assert!(parse("/v1/unknown").is_none());
        assert!(parse("/eu/v1/models").is_none());
    }
}
