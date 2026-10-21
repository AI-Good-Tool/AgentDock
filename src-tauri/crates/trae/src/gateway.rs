//! Trae native chat (`llm_utils_chat`) as a unified-gateway upstream. Only
//! plain-text conversations are supported by the native endpoint; tools and
//! images are rejected before any account is charged.
use crate::client::{host_for, profile};
use agentdock_core::{
    diagnostics::RequestDetails,
    provider::{AccountEffect, SendFailure, SendInput, SendOk},
    stream::{SseFrames, StreamFailure, UpstreamEvent},
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub fn native_headers(account: &Value) -> Result<Vec<(String, String)>, String> {
    let p = profile(account["variant"].as_str().unwrap_or(""))?;
    let token = account["user"]["token"].as_str().unwrap_or("").to_owned();
    Ok(vec![
        ("Content-Type".into(), "application/json".into()),
        ("Authorization".into(), format!("Cloud-IDE-JWT {token}")),
        ("x-ide-token".into(), token.clone()),
        ("x-cloudide-token".into(), token),
        (
            "x-app-id".into(),
            p.product["bootConfig"]["agent"]["appId"]
                .as_str()
                .unwrap_or("")
                .into(),
        ),
        (
            "x-uid".into(),
            match &account["user"]["userId"] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            },
        ),
        (
            "x-device-id".into(),
            account["deviceId"].as_str().unwrap_or("0").into(),
        ),
        (
            "x-machine-id".into(),
            account["machineId"].as_str().unwrap_or("").into(),
        ),
        (
            "x-ide-version".into(),
            p.product["appVersion"].as_str().unwrap_or("").into(),
        ),
        (
            "x-ide-version-code".into(),
            match &p.product["traeVersionCode"] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            },
        ),
        ("x-ide-version-type".into(), "stable".into()),
        ("x-request-id".into(), uuid::Uuid::new_v4().to_string()),
        ("Accept".into(), "text/event-stream".into()),
    ])
}

/// Server-side model configurations for this account (merged into `models`).
pub async fn model_catalog(account: &Value) -> Result<Vec<Value>, String> {
    let mut req = crate::api::http()
        .post(format!(
            "{}/api/ide/v1/get_detail_param",
            host_for(account, "agent")?
        ))
        .timeout(Duration::from_secs(20))
        .json(&json!({"function":"chat_v3","need_prompt":false,"poly_prompt":true}));
    for (k, v) in native_headers(account)? {
        req = req.header(k, v);
    }
    let r = req.send().await.map_err(|_| "模型目录连接失败")?;
    let status = r.status().as_u16();
    let j: Value = r.json().await.map_err(|_| "模型目录响应不是 JSON")?;
    if !(200..300).contains(&status) || (!j["code"].is_null() && j["code"] != 0) {
        return Err(format!(
            "模型目录请求失败：{status}/{}",
            if j["code"].is_null() {
                String::new()
            } else {
                j["code"].to_string()
            }
        ));
    }
    let fresh: Vec<Value> = j["config_info_list"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let id = c["config_name"].as_str()?;
            Some(json!({
                "id": id,
                "name": c["display_config"]["display_name"].as_str().unwrap_or(id),
                "model": c["model_detail_list"][0]["model_name"].as_str().unwrap_or(id),
                "source": c["config_source"].as_i64().unwrap_or(1),
                "catalogSource": "native-api",
            }))
        })
        .collect();
    let mut models: Vec<Value> = account["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| !fresh.iter().any(|x| x["id"] == m["id"]))
        .cloned()
        .collect();
    models.extend(fresh);
    Ok(models)
}

/// Real model name an account entry serves.
pub fn real_model(m: &Value) -> Option<&str> {
    m["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| m["id"].as_str())
        .filter(|s| !s.is_empty())
}

/// Convert canonical chat to the native payload (text-only conversations).
pub fn native_body(chat: &Value, config_id: Option<&str>) -> Result<Value, String> {
    if chat["tools"].as_array().is_some_and(|t| !t.is_empty()) {
        return Err("Trae 原生反代暂只支持文本对话，不支持工具调用".into());
    }
    let rows = chat["messages"]
        .as_array()
        .filter(|r| !r.is_empty())
        .ok_or("messages 不能为空")?;
    let mut messages = vec![];
    for m in rows {
        let role = m["role"].as_str().unwrap_or("");
        if !["system", "developer", "user", "assistant"].contains(&role) {
            return Err("Trae 原生反代暂只支持 system/user/assistant 文本角色，不支持工具结果".into());
        }
        if m["tool_calls"].as_array().is_some_and(|t| !t.is_empty()) {
            return Err("Trae 原生反代暂只支持文本对话，不支持工具调用".into());
        }
        let text = match &m["content"] {
            Value::String(s) => s.clone(),
            Value::Array(parts) => {
                let mut out = vec![];
                for p in parts {
                    if p["type"] != "text" {
                        return Err("Trae 原生反代暂只支持纯文本内容，不支持图片".into());
                    }
                    out.push(p["text"].as_str().unwrap_or("").to_owned());
                }
                out.join("\n")
            }
            Value::Null if role == "assistant" => String::new(),
            _ => return Err("messages 中包含无效内容".into()),
        };
        messages.push(json!({
            "role": if role == "developer" { "system" } else { role },
            "content": [{"type":"text","text":text}],
        }));
    }
    let mut body = json!({"function":"chat_v3","stream":true,"messages":messages});
    if let Some(id) = config_id.filter(|m| *m != "auto") {
        body["model"] = json!(id);
    }
    if let Some(n) = chat["max_tokens"]
        .as_u64()
        .or_else(|| chat["max_completion_tokens"].as_u64())
    {
        if !(1..=128_000).contains(&n) {
            return Err("max_tokens 须为 1–128000 的整数".into());
        }
        body["max_tokens"] = json!(n);
    }
    Ok(body)
}

fn text_delta(data: &Value) -> Option<&str> {
    data["choices"][0]["delta"]["content"]
        .as_str()
        .or_else(|| data["delta"]["content"].as_str())
        .or_else(|| data["delta"].as_str())
        .or_else(|| data["response"].as_str())
        .or_else(|| data["content"].as_str())
        .or_else(|| data["text"].as_str())
}

pub async fn send(
    account: &Value,
    config_id: Option<String>,
    input: SendInput,
    details: &mut RequestDetails,
    started: Instant,
    host_override: Option<String>,
) -> Result<SendOk, SendFailure> {
    let payload = native_body(&input.body, config_id.as_deref()).map_err(|e| {
        details.local_error("conversion", &e);
        SendFailure::local(400, "invalid_request", e)
    })?;
    let host = match host_override.clone() {
        Some(h) => h,
        None => host_for(account, "agent").map_err(|e| SendFailure::local(503, "client_unavailable", e))?,
    };
    let url = format!("{host}/api/agent/v3/llm_utils_chat");
    details.upstream_url = Some(url.clone());
    details.upstream_summary = json!({"function":"chat_v3","messageCount":payload["messages"].as_array().map_or(0,Vec::len),"model":payload["model"],"transport":"trae-native"});
    let mut req = crate::api::http()
        .post(url)
        .timeout(Duration::from_secs(90))
        .json(&payload);
    let headers = if host_override.is_some() {
        vec![("Accept".to_owned(), "text/event-stream".to_owned())]
    } else {
        native_headers(account).map_err(|e| SendFailure::local(503, "client_unavailable", e))?
    };
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let response = req.send().await.map_err(|_| {
        details.local_error("upstream", "Trae 上游连接失败或超时");
        SendFailure::local(502, "upstream_unreachable", "Trae 上游连接失败或超时")
    })?;
    let status = response.status().as_u16();
    details.upstream_status = Some(status);
    details.content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if !response.status().is_success() {
        let raw = response.bytes().await.unwrap_or_default();
        let body: Value = serde_json::from_slice(&raw[..raw.len().min(16 * 1024)]).unwrap_or(Value::Null);
        let upstream_message: String = body["message"].as_str().unwrap_or("").chars().take(200).collect();
        let message = if upstream_message.is_empty() {
            format!("上游 HTTP {status}")
        } else {
            format!("上游 HTTP {status}：{upstream_message}")
        };
        details.local_error("upstream", &message);
        let effect = match status {
            401 | 403 => AccountEffect::Cooldown(60),
            429 => AccountEffect::Cooldown(60),
            _ => AccountEffect::None,
        };
        let mut failure = SendFailure::local(502, "upstream_http_error", message).with_effect(effect);
        if let Some(code) = body["code"].as_i64() {
            failure.error = json!({"code": code.to_string()});
        }
        return Err(failure);
    }
    if !details
        .content_type
        .as_deref()
        .unwrap_or("")
        .contains("text/event-stream")
    {
        details.local_error("upstream", "上游未返回预期的 SSE 流");
        return Err(SendFailure::local(
            502,
            "upstream_protocol_error",
            "上游未返回预期的 SSE 流",
        ));
    }
    let _ = started;
    let mut upstream = response.bytes_stream();
    let events = async_stream::stream! {
        let mut frames = SseFrames::default();
        let mut produced = 0usize;
        let mut usage: Option<Value> = None;
        let mut tail = String::new();
        let mut done = false;
        'outer: while let Some(part) = upstream.next().await {
            let part = match part {
                Ok(b) => b,
                Err(_) => {
                    yield Err(StreamFailure::new("上游流意外中断"));
                    return;
                }
            };
            let parsed = match frames.feed(&part) {
                Ok(v) => v,
                Err(e) => {
                    yield Err(StreamFailure::new(e));
                    return;
                }
            };
            for frame in parsed {
                if frame.data == b"[DONE]" {
                    done = true;
                    break 'outer;
                }
                let Ok(data) = serde_json::from_slice::<Value>(&frame.data) else {
                    yield Err(StreamFailure::new("上游协议格式异常"));
                    return;
                };
                if !data.is_object() {
                    yield Err(StreamFailure::new("上游协议格式异常"));
                    return;
                }
                let event = frame.event.as_deref().unwrap_or("message");
                if event == "error" || data.get("code").is_some_and(|c| !c.is_null() && c != 0) {
                    let code = data["code"].as_i64().map(|c| c.to_string()).unwrap_or_else(|| "upstream_error".into());
                    yield Err(StreamFailure {
                        message: format!("上游拒绝请求 ({code})"),
                        code: Some(code.clone()),
                        error: json!({"code": code}),
                    });
                    return;
                }
                if let Some(u) = data.get("usage").filter(|u| u.is_object()) {
                    let input = u["prompt_tokens"].as_u64().or_else(|| u["input_tokens"].as_u64());
                    let output = u["completion_tokens"].as_u64().or_else(|| u["output_tokens"].as_u64());
                    if let (Some(i), Some(o)) = (input, output) {
                        usage = Some(json!({"prompt_tokens":i,"completion_tokens":o,"total_tokens":i+o}));
                    } else if input.is_some() || output.is_some() {
                        usage = Some(json!({"prompt_tokens":input,"completion_tokens":output}));
                    }
                }
                let finished = event == "done" || data["choices"][0]["finish_reason"].is_string();
                if let Some(delta) = text_delta(&data).filter(|s| !s.is_empty()) {
                    produced += delta.len();
                    tail.push_str(delta);
                    if tail.len() > 4096 {
                        let cut = tail.len() - 2048;
                        let cut = (cut..tail.len()).find(|i| tail.is_char_boundary(*i)).unwrap_or(tail.len());
                        tail.drain(..cut);
                    }
                    if produced > 1024 * 1024 {
                        yield Err(StreamFailure::new("上游输出超过 1MB"));
                        return;
                    }
                    if produced > 2048 && tail.len() >= 2048 && tail.trim().is_empty() {
                        yield Err(StreamFailure::new("上游连续返回空白"));
                        return;
                    }
                    yield Ok(UpstreamEvent::Chunk(json!({"choices":[{"index":0,"delta":{"content":delta},"finish_reason":null}]})));
                }
                if finished {
                    done = true;
                    break 'outer;
                }
            }
        }
        if produced == 0 {
            yield Err(StreamFailure::new("上游未返回文本"));
            return;
        }
        if !done {
            yield Err(StreamFailure::new("上游流意外中断"));
            return;
        }
        // Drain any usage event that trails the completion marker.
        while let Some(Ok(part)) = upstream.next().await {
            if let Ok(parsed) = frames.feed(&part) {
                for frame in parsed {
                    if let Ok(data) = serde_json::from_slice::<Value>(&frame.data) {
                        if let Some(u) = data.get("usage").filter(|u| u.is_object()) {
                            let i = u["prompt_tokens"].as_u64().or_else(|| u["input_tokens"].as_u64());
                            let o = u["completion_tokens"].as_u64().or_else(|| u["output_tokens"].as_u64());
                            if let (Some(i), Some(o)) = (i, o) {
                                usage = Some(json!({"prompt_tokens":i,"completion_tokens":o,"total_tokens":i+o}));
                            }
                        }
                    }
                }
            }
        }
        let mut last = json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
        if let Some(u) = usage {
            last["usage"] = u;
        }
        yield Ok(UpstreamEvent::Chunk(last));
        yield Ok(UpstreamEvent::Done);
    };
    Ok(SendOk {
        events: Box::pin(events),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_payload_is_text_only_and_auto_is_not_a_model_name() {
        let r = native_body(
            &json!({"model":"auto","messages":[{"role":"developer","content":"a"},{"role":"user","content":[{"type":"text","text":"b"}]}]}),
            Some("auto"),
        )
        .unwrap();
        assert!(r.get("model").is_none());
        assert_eq!(r["messages"][0]["role"], "system");
        assert_eq!(r["messages"][1]["content"], json!([{"type":"text","text":"b"}]));
        assert_eq!(
            native_body(&json!({"messages":[{"role":"user","content":"x"}]}), Some("cfg-1")).unwrap()["model"],
            "cfg-1"
        );
        for bad in [
            json!({"messages":[]}),
            json!({"messages":[{"role":"user","content":"x"}],"tools":[{}]}),
            json!({"messages":[{"role":"tool","content":"x"}]}),
            json!({"messages":[{"role":"user","content":[{"type":"image_url"}]}]}),
            json!({"messages":[{"role":"user","content":"x"}],"max_tokens":0}),
        ] {
            assert!(native_body(&bad, None).is_err(), "{bad}");
        }
    }
    #[test]
    fn deltas_from_every_native_shape() {
        for v in [
            json!({"response":"你好"}),
            json!({"delta":{"content":"你好"}}),
            json!({"delta":"你好"}),
            json!({"choices":[{"delta":{"content":"你好"}}]}),
            json!({"content":"你好"}),
            json!({"text":"你好"}),
        ] {
            assert_eq!(text_delta(&v), Some("你好"));
        }
    }
}
