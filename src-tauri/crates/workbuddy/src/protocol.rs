use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Inspect only explicit SSE error envelopes. Assistant text is never treated
/// as quota evidence, and an oversized line is skipped until its next newline.
#[derive(Default)]
pub struct SseErrors {
    line: Vec<u8>,
    skipping: bool,
    pub usage: Value,
    pub finish_reason: Option<String>,
}
impl SseErrors {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Value> {
        let mut errors = vec![];
        for &byte in bytes {
            if byte == b'\n' {
                if !self.skipping {
                    if let Some(data) = self.line.strip_prefix(b"data:") {
                        if let Ok(value) = serde_json::from_slice::<Value>(data) {
                            if value["usage"].is_object() {
                                self.usage = crate::diagnostics::usage(&value["usage"]);
                            }
                            if let Some(reason) = value["choices"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .find_map(|c| c["finish_reason"].as_str())
                            {
                                self.finish_reason = Some(
                                    if [
                                        "stop",
                                        "length",
                                        "tool_calls",
                                        "function_call",
                                        "content_filter",
                                    ]
                                    .contains(&reason)
                                    {
                                        reason.into()
                                    } else {
                                        "other".into()
                                    },
                                );
                            }
                            if value.get("error").is_some_and(|e| !e.is_null())
                                || (value.get("choices").is_none()
                                    && value
                                        .get("code")
                                        .is_some_and(|c| !c.is_null() && c != 0 && c != 200))
                            {
                                errors.push(value);
                            }
                        }
                    }
                }
                self.line.clear();
                self.skipping = false;
            } else if !self.skipping {
                if self.line.len() < 65536 {
                    self.line.push(byte);
                } else {
                    self.line.clear();
                    self.skipping = true;
                }
            }
        }
        errors
    }
}

/// Aggregate SSE into OpenAI chat.completion, including reasoning and tool fragments.
/// Decoding happens after complete byte collection so split UTF-8 codepoints survive.
pub fn aggregate(bytes: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "上游返回了无效 UTF-8")?
        .replace("\r\n", "\n");
    let mut result = json!({ "object":"chat.completion", "choices":[] });
    let mut choices: BTreeMap<u64, Value> = BTreeMap::new();
    let mut tools: BTreeMap<(u64, u64), Value> = BTreeMap::new();
    let mut done = false;
    for frame in text.split("\n\n") {
        let data = frame
            .lines()
            .filter_map(|s| s.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            done = true;
            break;
        }
        let chunk: Value = serde_json::from_str(&data).map_err(|_| "上游 SSE 包含无效 JSON")?;
        if chunk.get("error").is_some() {
            return Err("上游 SSE 返回错误事件".into());
        }
        for k in ["id", "created", "model", "system_fingerprint"] {
            if let Some(v) = chunk.get(k) {
                result[k] = v.clone();
            }
        }
        if chunk.get("usage").is_some_and(|x| !x.is_null()) {
            result["usage"] = chunk["usage"].clone();
        }
        for c in chunk["choices"].as_array().into_iter().flatten() {
            let index = c["index"].as_u64().unwrap_or(0);
            let out=choices.entry(index).or_insert_with(|| json!({"index":index,"message":{"role":"assistant","content":""},"finish_reason":null}));
            for k in ["content", "reasoning_content", "refusal"] {
                if let Some(s) = c["delta"][k].as_str().filter(|s| !s.is_empty()) {
                    let old = out["message"][k].as_str().unwrap_or("");
                    out["message"][k] = json!(format!("{old}{s}"));
                }
            }
            if c["finish_reason"].as_str().is_some_and(|s| !s.is_empty()) {
                out["finish_reason"] = c["finish_reason"].clone();
            }
            for t in c["delta"]["tool_calls"].as_array().into_iter().flatten() {
                let ti = t["index"].as_u64().unwrap_or(0);
                let tool = tools.entry((index, ti)).or_insert_with(
                    || json!({"type":"function","function":{"name":"","arguments":""}}),
                );
                for k in ["id", "type"] {
                    if let Some(v) = t.get(k) {
                        tool[k] = v.clone();
                    }
                }
                for k in ["name", "arguments"] {
                    if let Some(s) = t["function"][k].as_str() {
                        let old = tool["function"][k].as_str().unwrap_or("");
                        tool["function"][k] = json!(format!("{old}{s}"));
                    }
                }
            }
        }
    }
    if !done || choices.is_empty() {
        return Err("上游流未完整结束，不生成伪造的成功响应".into());
    }
    for (index, c) in choices.iter_mut() {
        let calls = tools
            .iter()
            .filter(|((i, _), _)| i == index)
            .map(|(_, v)| v.clone())
            .collect::<Vec<_>>();
        if !calls.is_empty() {
            c["message"]["tool_calls"] = json!(calls);
            if c["message"]["content"] == "" {
                c["message"]["content"] = Value::Null;
            }
        }
    }
    result["choices"] = json!(choices.into_values().collect::<Vec<_>>());
    Ok(result)
}

pub fn validate(body: &Value, models: &[Value]) -> Result<(), String> {
    let id = body["model"].as_str().ok_or("model 必须是字符串")?;
    let model = models
        .iter()
        .find(|m| m["id"] == id)
        .ok_or("模型不在已发现目录中，请先同步目录")?;
    if model["disabled"] == true {
        return Err("模型已被上游标记为不可用".into());
    }
    if model["tags"].as_array().is_some_and(|tags| {
        tags.iter().any(|t| {
            t.as_str()
                .is_some_and(|s| s.contains("image") || s.contains("video"))
        })
    }) {
        return Err("图片/视频模型不能使用聊天端点".into());
    }
    if model
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
    {
        return Err("自定义端点需要独立凭据，当前反代不转发 WorkBuddy 令牌到自定义域名".into());
    }
    if !body["messages"].as_array().is_some_and(|a| !a.is_empty()) {
        return Err("messages 必须是非空数组".into());
    }
    if body.get("n").is_some_and(|n| n != 1) {
        return Err("当前仅支持 n=1".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sse_quota_observer_handles_split_bytes_and_ignores_assistant_quotes() {
        let mut observer = SseErrors::default();
        let quoted = json!({"choices":[{"delta":{"content":"超出频率限制，将在 2099-01-01 12:00:00 UTC+8 重置"}}]});
        assert!(observer
            .feed(format!("data: {quoted}\r\n\r\n").as_bytes())
            .is_empty());
        let event = format!(
            "data: {}\r\n\r\n",
            json!({"error":{"message":"超出频率限制，将在 2099-01-01 12:00:00 UTC+8 重置"}})
        );
        let mut errors = vec![];
        for byte in event.bytes() {
            errors.extend(observer.feed(&[byte]));
        }
        assert_eq!(errors.len(), 1);
        assert!(errors[0]["error"].is_object());
        assert!(observer.feed(&vec![b'x'; 100_000]).is_empty());
        assert!(observer.feed(b"\n").is_empty());
        assert_eq!(observer.feed(event.as_bytes()).len(), 1);
    }
    #[test]
    fn reasoning_usage_tools_and_crlf() {
        let events = [
            json!({"id":"a","model":"glm","created":1,"choices":[{"index":0,"delta":{"reasoning_content":"想","tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"read","arguments":"{\""}}]}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"x\":1}"}}]},"finish_reason":"tool_calls"}],"usage":{"total_tokens":9}}),
        ];
        let text = events
            .iter()
            .map(|e| format!("data: {e}\r\n\r\n"))
            .collect::<String>()
            + "data: [DONE]\r\n\r\n";
        let out = aggregate(text.as_bytes()).unwrap();
        assert_eq!(out["choices"][0]["message"]["reasoning_content"], "想");
        assert_eq!(
            out["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
            "{\"x\":1}"
        );
        assert!(out["choices"][0]["message"]["content"].is_null());
        assert_eq!(out["usage"]["total_tokens"], 9);
    }
    #[test]
    fn incomplete_stream_is_failure() {
        assert!(aggregate(b"data: {\"choices\":[]}\n\n").is_err());
    }
    #[test]
    fn custom_domain_and_media_rejected() {
        let m = json!([{"id":"evil","url":"https://example.com"},{"id":"image","tags":["text-to-image"]}]);
        for id in ["evil", "image"] {
            assert!(validate(
                &json!({"model":id,"messages":[{"role":"user","content":"hi"}]}),
                m.as_array().unwrap()
            )
            .is_err());
        }
    }
}
