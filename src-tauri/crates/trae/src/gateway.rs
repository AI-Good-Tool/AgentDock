//! Trae as a unified-gateway upstream via the official Remote Agent chain
//! (`/api/remote/v1`: models → chat_sessions → events), ported from Trae
//! Switch 0.6. One request is one Remote turn: the reply arrives when the turn
//! completes (not token by token). Client tools use a text bridge: the model
//! is asked to emit `<client_tool_call>{…}</client_tool_call>` and never to run
//! remote tools; Remote tool execution, model mismatch and incomplete turns
//! fail the request. Images are not supported.
use crate::client::host_for;
use agentdock_core::{
    diagnostics::RequestDetails,
    provider::{AccountEffect, SendFailure, SendInput, SendOk},
    stream::{SseFrames, UpstreamEvent},
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const AGENT: &str = "solo_agent_remote";
const MODELS_QUERY: &str = "/models?functions=solo_agent_remote%2Csolo_work_remote&show_custom_model=true";
/// One Remote turn (planning + answer) may take a while; abandoned turns are stopped.
const TURN_TIMEOUT: Duration = Duration::from_secs(300);
const OUTPUT_LIMIT: usize = 1024 * 1024;

fn remote_base(account: &Value, host_override: Option<&str>) -> Result<String, String> {
    let host = match host_override {
        Some(h) => h.to_owned(),
        None => host_for(account, "agent")?,
    };
    Ok(format!("{host}/api/remote/v1"))
}

fn remote_headers(account: &Value) -> Vec<(&'static str, String)> {
    vec![
        ("Authorization", format!("Cloud-IDE-JWT {}", account["user"]["token"].as_str().unwrap_or(""))),
        ("X-Trae-Client-Type", "web".into()),
        ("x-user-region", if account["variant"] == "cn" { "CN" } else { "SG" }.into()),
    ]
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("http client")
}

fn valid_model_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "._:/-".contains(c))
}

/// Raw `solo_agent_remote` model entries of the account's official Remote catalog.
async fn remote_models(account: &Value, host_override: Option<&str>) -> Result<Vec<Value>, String> {
    let mut req = http()
        .get(format!("{}{MODELS_QUERY}", remote_base(account, host_override)?))
        .timeout(Duration::from_secs(20));
    for (k, v) in remote_headers(account) {
        req = req.header(k, v);
    }
    let r = req.send().await.map_err(|_| "官方 Remote 模型目录连接失败")?;
    let status = r.status();
    let j: Value = r.json().await.map_err(|_| "官方 Remote 模型目录响应不是 JSON")?;
    if !status.is_success() || j["code"].as_i64().is_some_and(|c| c != 0) {
        return Err(format!("官方 Remote 模型目录查询失败（HTTP {}）", status.as_u16()));
    }
    let list = j["data"]["list"].as_array().ok_or("官方 Remote 模型目录格式变化")?;
    Ok(list
        .iter()
        .filter(|g| g["function"] == AGENT)
        .flat_map(|g| g["models"].as_array().cloned().unwrap_or_default())
        .filter(|m| m["name"].as_str().is_some_and(valid_model_name))
        .collect())
}

/// The account's Remote Agent models, stored on the account as `models`.
pub async fn model_catalog(account: &Value) -> Result<Vec<Value>, String> {
    let mut seen = std::collections::HashSet::new();
    let models: Vec<Value> = remote_models(account, None)
        .await?
        .into_iter()
        .filter(|m| seen.insert(m["name"].as_str().unwrap_or("").to_owned()))
        .map(|m| {
            let name = m["name"].as_str().unwrap_or("");
            json!({
                "id": name,
                "name": m["display_name"].as_str().filter(|s| !s.is_empty()).unwrap_or(name),
                "model": name,
                "source": m["config_source"].as_i64().unwrap_or(1),
                "catalogSource": "remote-api",
                "remote": true,
                "isDefault": m["is_default"] == true,
                "functions": [AGENT],
            })
        })
        .collect();
    if models.is_empty() {
        return Err("当前账号没有可用的 Remote Agent 模型".into());
    }
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

fn text_of(content: &Value) -> Result<String, String> {
    match content {
        Value::String(s) => Ok(s.clone()),
        Value::Null => Ok(String::new()),
        Value::Array(parts) => {
            let mut out = vec![];
            for p in parts {
                match p["type"].as_str() {
                    Some("text") | Some("input_text") | Some("output_text") => {
                        out.push(p["text"].as_str().unwrap_or("").to_owned())
                    }
                    Some("image_url") | Some("input_image") | Some("image") => {
                        return Err("Trae 暂不支持图片输入".into())
                    }
                    _ => return Err("消息内容无效".into()),
                }
            }
            Ok(out.join("\n"))
        }
        _ => Err("消息内容无效".into()),
    }
}

/// Requests the Remote chain can serve: text (and client tools), no images.
pub fn validate(chat: &Value) -> Result<(), String> {
    remote_context(chat).map(|_| ())
}

/// Client tools in the shape the bridge prompt shows the model.
fn client_tools(chat: &Value) -> Vec<Value> {
    chat["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let f = if t["function"].is_object() { &t["function"] } else { t };
            let name = f["name"].as_str()?;
            Some(json!({
                "name": name,
                "description": f["description"],
                "input_schema": if f["parameters"].is_object() { f["parameters"].clone() } else { f["input_schema"].clone() },
            }))
        })
        .collect()
}

/// OpenAI tool_choice → bridge policy (`auto` / `none` / `any` / `tool`).
fn tool_choice(chat: &Value) -> Value {
    match &chat["tool_choice"] {
        Value::String(s) if s == "none" => json!({"type":"none"}),
        Value::String(s) if s == "required" || s == "any" => json!({"type":"any"}),
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("function") => {
            json!({"type":"tool","name": o.get("function").and_then(|f| f.get("name")).cloned().unwrap_or(Value::Null)})
        }
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("tool") => {
            json!({"type":"tool","name": o.get("name").cloned().unwrap_or(Value::Null)})
        }
        _ => json!({"type":"auto"}),
    }
}

fn tool_prompt(tools: &[Value], choice: &Value) -> String {
    if tools.is_empty() || choice["type"] == "none" {
        return String::new();
    }
    format!(
        "\nCLIENT TOOL BRIDGE: You are answering an external API caller. NEVER execute any built-in or remote tools on behalf of this caller. To request a client tool, output exactly <client_tool_call>{{\"name\":\"tool name\",\"input\":{{}}}}</client_tool_call> then finish. Do not use markdown fences. Only use tools from the JSON catalog below. The caller executes them locally and returns their results in a later request. For a normal final answer output ordinary text, without this tag. Tool policy: {choice}\nTools: {}",
        Value::Array(tools.to_vec())
    )
}

/// Flatten the conversation into the single Remote query (role-tagged blocks).
fn remote_context(chat: &Value) -> Result<String, String> {
    let rows = chat["messages"].as_array().filter(|r| !r.is_empty()).ok_or("messages 不能为空")?;
    let mut blocks = vec![];
    for m in rows {
        let role = m["role"].as_str().unwrap_or("");
        let text = match role {
            "system" | "developer" | "user" => text_of(&m["content"])?,
            "assistant" => {
                let mut lines = vec![text_of(&m["content"])?];
                for c in m["tool_calls"].as_array().into_iter().flatten() {
                    let input = c["function"]["arguments"]
                        .as_str()
                        .and_then(|a| serde_json::from_str::<Value>(a).ok())
                        .unwrap_or(json!({}));
                    lines.push(format!(
                        "[Client tool call] {}",
                        json!({"id": c["id"], "name": c["function"]["name"], "input": input})
                    ));
                }
                lines.retain(|l| !l.is_empty());
                lines.join("\n")
            }
            "tool" => format!(
                "[Client tool result] {}",
                json!({"id": m["tool_call_id"], "error": false, "content": text_of(&m["content"])?})
            ),
            _ => return Err("messages 中包含无效角色".into()),
        };
        let tag = match role {
            "developer" => "system",
            "tool" => "user",
            r => r,
        };
        blocks.push(format!("[{tag}]\n{text}"));
    }
    Ok(blocks.join("\n\n") + &tool_prompt(&client_tools(chat), &tool_choice(chat)))
}

/// Split the final text into plain text and validated client tool calls.
fn parse_tool_reply(text: &str, tools: &[Value], choice: &Value) -> Result<(String, Vec<Value>), String> {
    const OPEN: &str = "<client_tool_call>";
    const CLOSE: &str = "</client_tool_call>";
    let mut clean = String::new();
    let mut calls = vec![];
    let mut rest = text;
    while let Some(at) = rest.find(OPEN) {
        clean.push_str(&rest[..at]);
        let after = &rest[at + OPEN.len()..];
        let end = after.find(CLOSE).ok_or("invalid_tool_output")?;
        let call: Value = serde_json::from_str(after[..end].trim()).map_err(|_| "invalid_tool_output")?;
        let name = call["name"].as_str().unwrap_or("");
        if !tools.iter().any(|t| t["name"] == name)
            || !call["input"].is_object()
            || choice["type"] == "none"
            || (choice["type"] == "tool" && choice["name"] != name)
        {
            return Err("invalid_tool_output".into());
        }
        calls.push(json!({
            "index": calls.len(),
            "id": format!("call_{}", uuid::Uuid::new_v4().simple()),
            "type": "function",
            "function": {"name": name, "arguments": call["input"].to_string()},
        }));
        rest = &after[end + CLOSE.len()..];
    }
    clean.push_str(rest);
    let clean = clean.trim().to_owned();
    if clean.contains(CLOSE) || (["any", "tool"].contains(&choice["type"].as_str().unwrap_or("")) && calls.is_empty()) {
        return Err("invalid_tool_output".into());
    }
    Ok((clean, calls))
}

/// Stops the upstream turn unless it completed (also when the request is dropped).
struct StopGuard {
    url: String,
    headers: Vec<(&'static str, String)>,
    body: Value,
    finished: bool,
}
impl Drop for StopGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let (url, headers, body) = (self.url.clone(), self.headers.clone(), self.body.clone());
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let mut req = http().post(url).timeout(Duration::from_secs(5)).json(&body);
                for (k, v) in headers {
                    req = req.header(k, v);
                }
                let _ = req.send().await;
            });
        }
    }
}

fn fail(details: &mut RequestDetails, status: u16, code: &str, message: &str) -> SendFailure {
    details.local_error("upstream", message);
    SendFailure::local(status, code, message.to_owned())
}

pub async fn send(
    account: &Value,
    config_id: Option<String>,
    input: SendInput,
    details: &mut RequestDetails,
    started: Instant,
    host_override: Option<String>,
) -> Result<SendOk, SendFailure> {
    let chat = &input.body;
    let context = remote_context(chat).map_err(|e| {
        details.local_error("conversion", &e);
        SendFailure::local(400, "invalid_request", e)
    })?;
    let tools = client_tools(chat);
    let choice = tool_choice(chat);
    let base = remote_base(account, host_override.as_deref())
        .map_err(|e| SendFailure::local(503, "client_unavailable", e))?;
    let headers = remote_headers(account);

    // 1) Resolve the exact catalog entry (the full object is sent as custom_model).
    let catalog = remote_models(account, host_override.as_deref()).await.map_err(|e| {
        let mut f = fail(details, 502, "upstream_http_error", &e);
        if e.contains("401") || e.contains("403") {
            f = f.with_effect(AccountEffect::Cooldown(60));
        }
        f
    })?;
    let wanted: Vec<String> = [config_id.clone(), chat["model"].as_str().map(str::to_owned)]
        .into_iter()
        .flatten()
        .filter(|m| m != "auto")
        .collect();
    let model = if wanted.is_empty() {
        catalog.iter().find(|m| m["is_default"] == true).or(catalog.first())
    } else {
        catalog.iter().find(|m| wanted.iter().any(|w| m["name"] == w.as_str()))
    }
    .cloned()
    .ok_or_else(|| {
        fail(
            details,
            502,
            "remote_model_unavailable",
            "当前账号的官方 Remote 模型目录不提供此模型，请同步模型目录后选择当前可用名称",
        )
    })?;
    let model_name = model["name"].as_str().unwrap_or("").to_owned();
    details.upstream_url = Some(format!("{base}/chat_sessions"));
    details.upstream_summary = json!({
        "transport": "trae-remote-agent",
        "model": model_name,
        "messageCount": chat["messages"].as_array().map_or(0, Vec::len),
        "clientTools": tools.len(),
        "contextChars": context.chars().count(),
    });

    // 2) Create the session with the whole conversation as its first message.
    let cn = account["variant"] == "cn";
    let common = json!({
        "language": "zh-cn", "app_language": "zh-CN", "quality": "stable",
        "scope": if cn { "marscode-cn" } else { "marscode" }, "tenant": "marscode",
        "region": if cn { "cn" } else { "sg" }, "solo_chat_mode": "code",
        "biz_session_id": uuid::Uuid::new_v4().to_string(),
    });
    let create = json!({
        "mode": "code", "environment_id": "default", "env": "remote",
        "auto_create_project": false, "origin": "web",
        "initial_message": {
            "chat_session_id": "", "content": [],
            "query": json!([{"type":"text","data":{"content": context}}]).to_string(),
            "model_name": model_name, "custom_model": model,
            "model_config_source": model["config_source"].as_i64().unwrap_or(1),
            "model_is_preset": model["is_preset"] != false,
            "agent_type": AGENT, "agent_id": AGENT,
            "model_selection_strategy": "manual",
            "common_params": common.to_string(),
        },
    });
    let mut req = http().post(format!("{base}/chat_sessions")).timeout(Duration::from_secs(30)).json(&create);
    for (k, v) in &headers {
        req = req.header(*k, v);
    }
    let r = req.send().await.map_err(|_| fail(details, 502, "upstream_unreachable", "Trae 上游连接失败或超时"))?;
    let status = r.status().as_u16();
    details.upstream_status = Some(status);
    let session: Value = r.json().await.unwrap_or(Value::Null);
    if !(200..300).contains(&status) || session["code"].as_i64().is_some_and(|c| c != 0) {
        let msg = format!("Trae 创建会话失败（HTTP {status}）");
        let mut f = fail(details, 502, "upstream_http_error", &msg);
        if let Some(c) = session["code"].as_i64() {
            f.error = json!({"code": c.to_string()});
        }
        if matches!(status, 401 | 403 | 429) {
            f = f.with_effect(AccountEffect::Cooldown(60));
        }
        return Err(f);
    }
    let data = if session["data"].is_object() { &session["data"] } else { &session };
    let (Some(sid), Some(mid)) = (data["chat_session_id"].as_str(), data["message_id"].as_str()) else {
        return Err(fail(details, 502, "upstream_protocol_error", "Trae 会话响应缺少会话或消息 ID"));
    };
    details.upstream_request_id = Some(sid.to_owned());
    let mut guard = StopGuard {
        url: format!("{base}/chat_sessions/{sid}/stop"),
        headers: headers.clone(),
        body: json!({"chat_session_id": sid, "user_message_id": mid}),
        finished: false,
    };

    // 3) Read the turn's events until `done`.
    let mut req = http()
        .get(format!("{base}/chat_sessions/{sid}/events"))
        .query(&[("reply_to_message_id", mid)])
        .timeout(TURN_TIMEOUT);
    for (k, v) in &headers {
        req = req.header(*k, v);
    }
    let r = req.send().await.map_err(|_| fail(details, 502, "upstream_unreachable", "Trae 事件流连接失败或超时"))?;
    if !r.status().is_success() {
        let msg = format!("Trae 事件流请求失败（HTTP {}）", r.status().as_u16());
        return Err(fail(details, 502, "upstream_http_error", &msg));
    }
    let mut frames = SseFrames::default();
    let mut stream = r.bytes_stream();
    let (mut summary, mut plans, mut usage) = (String::new(), Vec::<(String, String)>::new(), None);
    let (mut confirmed, mut finished) = (false, false);
    'read: while let Some(part) = stream.next().await {
        let part = part.map_err(|_| fail(details, 502, "truncated_stream", "Trae 事件流意外中断"))?;
        if details.first_byte_ms.is_none() {
            details.first_byte_ms = Some(started.elapsed().as_millis());
        }
        details.response_bytes += part.len();
        for frame in frames.feed(&part).map_err(|e| fail(details, 502, "upstream_protocol_error", &e))? {
            let Ok(data) = serde_json::from_slice::<Value>(&frame.data) else { continue };
            let event = frame.event.as_deref().unwrap_or("message");
            if event == "error" || data["code"].as_i64().is_some_and(|c| c != 0) {
                let code = data["code"].as_i64().map(|c| c.to_string()).unwrap_or_else(|| "upstream_error".into());
                let mut f = fail(details, 502, "upstream_business_error", &format!("Trae 上游拒绝请求（{code}）"));
                f.error = json!({"code": code});
                return Err(f);
            }
            match event {
                "model_config" => {
                    if data["config_name"].as_str().is_some_and(|n| n != model_name) {
                        return Err(fail(details, 502, "upstream_model_mismatch", "上游实际模型与请求模型不一致，已停止该请求"));
                    }
                    confirmed = true;
                }
                "plan_item" => {
                    let tool = &data["tool_call_info"];
                    if tool["name"] == "finish" && tool["params"]["summary"].is_string() {
                        summary = tool["params"]["summary"].as_str().unwrap_or("").to_owned();
                    } else if tool["name"].as_str().is_some_and(|n| !n.is_empty()) {
                        return Err(fail(details, 502, "remote_tool_execution", "上游尝试执行内置工具，已停止该请求"));
                    }
                    if let (Some(id), Some(thought)) = (data["id"].as_str(), data["thought"].as_str().filter(|t| !t.is_empty())) {
                        match plans.iter_mut().find(|(k, _)| k == id) {
                            Some(entry) => entry.1 = thought.to_owned(),
                            None => plans.push((id.to_owned(), thought.to_owned())),
                        }
                    }
                }
                "token_usage" => {
                    if let (Some(i), Some(o)) = (data["prompt_tokens"].as_u64(), data["completion_tokens"].as_u64()) {
                        usage = Some(json!({"prompt_tokens": i, "completion_tokens": o, "total_tokens": i + o}));
                    }
                }
                "done" => {
                    if data["status"] != "completed" {
                        return Err(fail(details, 502, "truncated_stream", "Trae 本轮未正常完成"));
                    }
                    finished = true;
                    break 'read;
                }
                _ => {}
            }
        }
    }
    if !finished || !confirmed {
        return Err(fail(details, 502, "truncated_stream", "Trae 事件流在完成前结束"));
    }
    guard.finished = true;
    details.stream_complete = true;
    let text = if summary.is_empty() {
        plans.into_iter().map(|(_, t)| t).collect::<Vec<_>>().join("\n")
    } else {
        summary
    };
    if text.is_empty() {
        return Err(fail(details, 502, "empty_output", "Trae 未返回文本"));
    }
    if text.len() > OUTPUT_LIMIT {
        return Err(fail(details, 502, "output_limit", "上游输出超过 1MB"));
    }
    let (clean, calls) = parse_tool_reply(&text, &tools, &choice)
        .map_err(|_| fail(details, 502, "invalid_tool_output", "上游返回的客户端工具调用格式无效"))?;
    let finish = if calls.is_empty() { "stop" } else { "tool_calls" };
    details.finish_reason = Some(finish.into());

    let mut chunks = vec![];
    if !clean.is_empty() {
        chunks.push(json!({"choices":[{"index":0,"delta":{"role":"assistant","content":clean},"finish_reason":null}]}));
    }
    if !calls.is_empty() {
        chunks.push(json!({"choices":[{"index":0,"delta":{"tool_calls":calls},"finish_reason":null}]}));
    }
    let mut last = json!({"choices":[{"index":0,"delta":{},"finish_reason":finish}]});
    if let Some(u) = usage {
        last["usage"] = u;
    }
    chunks.push(last);
    let events = futures_util::stream::iter(
        chunks
            .into_iter()
            .map(|c| Ok(UpstreamEvent::Chunk(c)))
            .chain(std::iter::once(Ok(UpstreamEvent::Done))),
    );
    Ok(SendOk { events: Box::pin(events) })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_flattens_roles_tools_and_rejects_images() {
        let chat = json!({
            "messages": [
                {"role":"developer","content":"be brief"},
                {"role":"user","content":[{"type":"text","text":"list files"}]},
                {"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"ls","arguments":"{\"path\":\".\"}"}}]},
                {"role":"tool","tool_call_id":"call_1","content":"a.txt"},
            ],
            "tools":[{"type":"function","function":{"name":"ls","description":"list","parameters":{"type":"object"}}}],
            "tool_choice":"required",
        });
        let ctx = remote_context(&chat).unwrap();
        assert!(ctx.starts_with("[system]\nbe brief\n\n[user]\nlist files\n\n[assistant]\n[Client tool call] "));
        assert!(ctx.contains("\"input\":{\"path\":\".\"}"));
        assert!(ctx.contains("[user]\n[Client tool result] {\"id\":\"call_1\",\"error\":false,\"content\":\"a.txt\"}"));
        assert!(ctx.contains("CLIENT TOOL BRIDGE") && ctx.contains("{\"type\":\"any\"}"));
        assert!(validate(&json!({"messages":[{"role":"user","content":[{"type":"image_url"}]}]})).is_err());
        assert!(validate(&json!({"messages":[]})).is_err());
        assert!(!remote_context(&json!({"messages":[{"role":"user","content":"x"}]})).unwrap().contains("BRIDGE"));
    }
    #[test]
    fn tool_replies_are_validated_against_the_catalog_and_policy() {
        let tools = vec![json!({"name":"ls"})];
        let auto = json!({"type":"auto"});
        let (text, calls) = parse_tool_reply("checking <client_tool_call>{\"name\":\"ls\",\"input\":{\"p\":1}}</client_tool_call>", &tools, &auto).unwrap();
        assert_eq!(text, "checking");
        assert_eq!(calls[0]["function"]["name"], "ls");
        assert_eq!(calls[0]["function"]["arguments"], "{\"p\":1}");
        assert_eq!(parse_tool_reply("plain", &tools, &auto).unwrap(), ("plain".into(), vec![]));
        for bad in [
            "<client_tool_call>{\"name\":\"rm\",\"input\":{}}</client_tool_call>",
            "<client_tool_call>{\"name\":\"ls\",\"input\":[]}</client_tool_call>",
            "<client_tool_call>{oops</client_tool_call>",
            "<client_tool_call>{\"name\":\"ls\",\"input\":{}}",
        ] {
            assert!(parse_tool_reply(bad, &tools, &auto).is_err(), "{bad}");
        }
        assert!(parse_tool_reply("no call", &tools, &json!({"type":"any"})).is_err());
        assert!(parse_tool_reply("<client_tool_call>{\"name\":\"ls\",\"input\":{}}</client_tool_call>", &tools, &json!({"type":"none"})).is_err());
    }
    #[test]
    fn tool_choice_maps_openai_and_anthropic_shapes() {
        assert_eq!(tool_choice(&json!({})), json!({"type":"auto"}));
        assert_eq!(tool_choice(&json!({"tool_choice":"none"})), json!({"type":"none"}));
        assert_eq!(tool_choice(&json!({"tool_choice":{"type":"function","function":{"name":"ls"}}})), json!({"type":"tool","name":"ls"}));
        assert!(valid_model_name("deepseek-v4.1-flash") && !valid_model_name("bad name") && !valid_model_name(""));
    }
}
