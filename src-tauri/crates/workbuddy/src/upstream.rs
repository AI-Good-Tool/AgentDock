//! WorkBuddy chat upstream for the unified gateway. Account choice, concurrency
//! and protocol conversion happen in the gateway; this module only prepares
//! the official request for one chosen account and normalizes its SSE.
use crate::{
    diagnostics::{self, RequestDetails},
    pool::{self, Shared},
};
use agentdock_core::{
    anthropic,
    provider::{AccountEffect, SendFailure, SendInput, SendOk},
    stream::{SseFrames, StreamFailure, UpstreamEvent},
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::Instant;

pub const REQUEST_PROFILE: &str = "WorkBuddy Desktop 5.8.3 / CLI 2.161.0";

pub fn channel_denied(code: &Value) -> bool {
    code.as_i64() == Some(11128) || code.as_str() == Some("11128")
}

/// Map an allowlisted upstream rejection into a client-facing failure.
pub fn rejection(status: u16, details: &RequestDetails) -> SendFailure {
    let data = &details.error;
    let denied = channel_denied(&data["code"]);
    let message = if denied {
        "WorkBuddy 官方渠道校验拒绝（11128）：此 API 调用来自未获批准的渠道。重复发送或切换账号不能保证解决；请通过官方支持确认第三方接入权限，或使用获授权的模型 API。"
    } else {
        data["displayMessage"]
            .as_str()
            .or_else(|| data["message"].as_str())
            .or_else(|| data["msg"].as_str())
            .unwrap_or("WorkBuddy 拒绝了请求")
    };
    let mut error = json!({"code": data["code"], "request_id": details.upstream_request_id});
    if denied {
        error["retryable"] = json!(false);
    }
    SendFailure {
        status,
        code: if denied {
            "upstream_channel_denied"
        } else {
            "upstream_error"
        }
        .into(),
        message: message.into(),
        effect: AccountEffect::None,
        error,
        anthropic_type: denied.then(|| "permission_error".into()),
        retryable: denied.then_some(false),
    }
}

/// Read at most 64 KiB of an error body; never retain arbitrary text.
pub async fn diagnostic_json(
    response: reqwest::Response,
    details: &mut RequestDetails,
    started: Instant,
) -> Value {
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return json!({"message":"读取上游错误响应失败"});
        };
        if details.first_byte_ms.is_none() {
            details.first_byte_ms = Some(started.elapsed().as_millis());
        }
        details.response_bytes += chunk.len();
        if buffer.len() + chunk.len() > 65536 {
            return json!({"message":"上游错误响应超过 64 KiB，已停止读取正文"});
        }
        buffer.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&buffer)
        .unwrap_or_else(|_| json!({"message":"上游未返回 JSON 错误结构，响应正文已省略"}))
}

/// Explicit SSE error envelopes only; assistant text is never inspected.
pub fn error_event(value: &Value) -> bool {
    value.get("error").is_some_and(|e| !e.is_null())
        || (value.get("choices").is_none()
            && value
                .get("code")
                .is_some_and(|c| !c.is_null() && c != 0 && c != 200))
}

pub async fn send(
    state: &Shared,
    account_id: &str,
    config: Value,
    input: SendInput,
    details: &mut RequestDetails,
    started: Instant,
) -> Result<SendOk, SendFailure> {
    let account = state
        .store
        .lock()
        .unwrap()
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .cloned()
        .ok_or_else(|| SendFailure::local(503, "no_available_account", "账号已移除"))?;
    let model = input.body["model"].as_str().unwrap_or("").to_owned();
    let allow_auxiliary = state.store.lock().unwrap().settings.allow_auxiliary_agents;
    let agent_purpose = crate::official::purpose(&input.headers, allow_auxiliary).map_err(|m| {
        details.local_error("agent_policy", m);
        SendFailure::local(403, "agent_disabled", m)
    })?;
    details.request_summary["agentPurpose"] = json!(agent_purpose);
    details.upstream_url = Some(format!("{}/v2/chat/completions", pool::endpoint(&account)));
    let cred = pool::credentials(&account).map_err(|e| {
        details.local_error("credentials", &e);
        SendFailure::local(401, "credential_unavailable", e)
    })?;
    let mut body = input.body;
    if input.effort_input.is_object() {
        // Effort support comes from the selected model, never a substitute model.
        body.as_object_mut().unwrap().remove("reasoning_effort");
        anthropic::apply_effort(&input.effort_input, &mut body, &config);
    }
    if input.client_profile.as_deref() == Some("zcode") {
        // Local provenance only; never disguise or strip a client's instructions.
        details.request_summary["clientProfile"] = json!("zcode");
    }
    crate::official::prepare_body(&mut body, Some(&config));
    let mut headers = input.headers.clone();
    if let Some(id) = input.session_id.as_deref().and_then(|s| s.parse().ok()) {
        headers.insert("x-conversation-id", id);
    }
    let context = state.conversations.lock().unwrap().prepare(&headers, &mut body);
    details.upstream_summary = diagnostics::summary(&body);
    details.upstream_summary["agentPurpose"] = json!(agent_purpose);
    details.upstream_summary["requestProfile"] = json!(REQUEST_PROFILE);
    details.upstream_summary["gzip"] = json!(true);
    details.upstream_summary["sessionInferred"] = json!(context.inferred);
    details.upstream_summary["transport"] = json!("installed-electron-node");
    let request = crate::official::chat_request_with_context(
        crate::proxy::official_request(state, &account, &cred, "/v2/chat/completions"),
        &context,
        &agent_purpose,
        &body,
    )
    .and_then(|r| r.build().map_err(|_| "请求构造失败".to_owned()));
    let request = match request {
        Ok(request) => request,
        Err(message) if message == crate::REQUEST_TOO_LARGE_MESSAGE => {
            details.local_error("encoding", &message);
            return Err(SendFailure::local(413, "request_too_large", message));
        }
        Err(_) => {
            details.local_error("encoding", "请求压缩失败");
            return Err(SendFailure::local(500, "encoding_failed", "请求压缩失败"));
        }
    };
    let use_device_sdk = state.store.lock().unwrap().settings.use_device_sdk;
    let response = state
        .official_transport
        .send(request, use_device_sdk)
        .await
        .map_err(|message| {
            details.local_error("official_transport", &message);
            SendFailure::local(503, "official_transport_unavailable", message)
        })?;
    let status = response.status().as_u16();
    if let Some(code) = response
        .headers()
        .get("x-wb-local-error")
        .and_then(|v| v.to_str().ok())
    {
        if code == "request_too_large" {
            details.local_error("official_transport", crate::REQUEST_TOO_LARGE_MESSAGE);
            return Err(SendFailure::local(
                status,
                "request_too_large",
                crate::REQUEST_TOO_LARGE_MESSAGE,
            ));
        }
        let message = if code == "device_token_unavailable" {
            "官方设备 SDK 暂未取得有效令牌，请检查 WorkBuddy 安装与设备服务状态后重试"
        } else {
            "本机官方传输不可用，请检查 WorkBuddy 版本与区域是否匹配"
        };
        details.local_error("official_transport", message);
        return Err(SendFailure::local(
            status,
            "official_transport_unavailable",
            message,
        ));
    }
    details.upstream_summary["deviceTokenSource"] = json!("installed-native-sdk");
    details.upstream_summary["gzipHeaderHex"] = json!("1f8b0800000000000013");
    details.upstream_status = Some(status);
    let safe_header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| diagnostics::bounded(s, 256))
    };
    details.upstream_request_id = safe_header("x-request-id");
    details.trace_id = safe_header("traceid").or_else(|| safe_header("x-trace-id"));
    details.content_type = safe_header("content-type");
    details.retry_after = safe_header("retry-after");
    let retry_after_seconds = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(60)
        .clamp(1, 3600);
    if !response.status().is_success() {
        let data = diagnostic_json(response, details, started).await;
        details.upstream_error(&data, &[&cred.token]);
        let model_limited = crate::switch_bridge::observe_limit(state, &account.id, &model, &data);
        let mut failure = rejection(status, details);
        failure.effect = if status == 401 || status == 403 {
            AccountEffect::AuthFailed
        } else if status == 429 && !model_limited {
            AccountEffect::Cooldown(retry_after_seconds)
        } else {
            AccountEffect::None
        };
        return Err(failure);
    }
    let content_type = details.content_type.clone().unwrap_or_default();
    if !content_type.contains("text/event-stream") {
        let data = diagnostic_json(response, details, started).await;
        details.upstream_error(&data, &[&cred.token]);
        crate::switch_bridge::observe_limit(state, &account.id, &model, &data);
        return Err(SendFailure::local(
            502,
            "invalid_upstream_response",
            "上游没有返回 SSE 流",
        ));
    }
    let state = state.clone();
    let account_id = account.id.clone();
    let token = cred.token;
    let mut upstream = response.bytes_stream();
    let events = async_stream::stream! {
        let mut frames = SseFrames::default();
        while let Some(part) = upstream.next().await {
            let part = match part {
                Ok(bytes) => bytes,
                Err(_) => {
                    yield Err(StreamFailure::new("上游流中断"));
                    return;
                }
            };
            let parsed = match frames.feed(&part) {
                Ok(frames) => frames,
                Err(e) => {
                    yield Err(StreamFailure::new(e));
                    return;
                }
            };
            for frame in parsed {
                if frame.data == b"[DONE]" {
                    yield Ok(UpstreamEvent::Done);
                    return;
                }
                let Ok(value) = serde_json::from_slice::<Value>(&frame.data) else {
                    yield Err(StreamFailure::new("上游 SSE 包含无效 JSON"));
                    return;
                };
                if error_event(&value) {
                    crate::switch_bridge::observe_limit(&state, &account_id, &model, &value);
                    let mut scratch = RequestDetails::default();
                    scratch.upstream_error(&value, &[&token]);
                    let message = scratch.error["displayMessage"]
                        .as_str()
                        .or_else(|| scratch.error["message"].as_str())
                        .or_else(|| scratch.error["msg"].as_str())
                        .unwrap_or("WorkBuddy 返回错误事件")
                        .to_owned();
                    yield Err(StreamFailure {
                        message,
                        code: scratch.error["code"].as_i64().map(|c| c.to_string())
                            .or_else(|| scratch.error["code"].as_str().map(str::to_owned)),
                        error: scratch.error,
                    });
                    return;
                }
                yield Ok(UpstreamEvent::Chunk(value));
            }
        }
    };
    Ok(SendOk {
        events: Box::pin(events),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channel_rejection_is_explicit_and_not_retryable() {
        for code in [json!(11128), json!("11128")] {
            let mut details = RequestDetails::default();
            details.upstream_error(
                &json!({"code": code, "msg": "Illegal API invocation", "requestId": "rid", "authorization":"NEVER_COPY"}),
                &[],
            );
            let failure = rejection(400, &details);
            assert_eq!(failure.code, "upstream_channel_denied");
            assert!(failure.message.contains("11128"));
            assert_eq!(failure.retryable, Some(false));
            assert_eq!(failure.anthropic_type.as_deref(), Some("permission_error"));
            assert_eq!(failure.error["request_id"], "rid");
            assert!(!failure.error.to_string().contains("NEVER_COPY"));
        }
        let mut details = RequestDetails::default();
        details.upstream_error(
            &json!({"code": 12345, "msg": "blocked token-secret"}),
            &["token-secret"],
        );
        let failure = rejection(429, &details);
        assert_eq!(failure.message, "blocked [REDACTED]");
        assert_eq!(failure.code, "upstream_error");
        assert!(failure.retryable.is_none());
    }
    #[test]
    fn only_explicit_envelopes_are_errors() {
        assert!(error_event(&json!({"error":{"message":"quota"}})));
        assert!(error_event(&json!({"code":11128})));
        assert!(!error_event(&json!({"code":0})));
        assert!(!error_event(
            &json!({"choices":[{"delta":{"content":"error"}}],"code":5})
        ));
    }
}
