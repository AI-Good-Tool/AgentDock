//! Bounded request metadata and allowlisted error diagnostics; never store bodies.
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestDetails {
    pub protocol: String,
    pub method: String,
    pub client_path: String,
    pub started_at: String,
    pub upstream_url: Option<String>,
    pub upstream_status: Option<u16>,
    pub upstream_request_id: Option<String>,
    pub trace_id: Option<String>,
    pub content_type: Option<String>,
    pub retry_after: Option<String>,
    pub request_summary: Value,
    pub upstream_summary: Value,
    pub error_stage: Option<String>,
    pub error: Value,
    pub first_byte_ms: Option<u128>,
    pub response_bytes: usize,
    pub stream_complete: bool,
    pub finish_reason: Option<String>,
    pub usage: Value,
}

impl RequestDetails {
    pub fn incoming(protocol: &str, path: &str, body: &Value) -> Self {
        Self {
            protocol: protocol.into(),
            method: if protocol == "internal" {
                "COMMAND"
            } else {
                "POST"
            }
            .into(),
            client_path: path.into(),
            started_at: chrono::Utc::now().to_rfc3339(),
            request_summary: summary(body),
            ..Self::default()
        }
    }
    pub fn local_error(&mut self, stage: &str, message: &str) {
        self.error_stage = Some(stage.into());
        self.error = json!({"message":bounded(message,2048)});
    }
    pub fn upstream_error(&mut self, data: &Value, secrets: &[&str]) {
        self.error_stage = Some("upstream".into());
        // Unknown response fields may echo prompts, tokens or completions.
        // Keep only documented error fields, with hard limits and redaction.
        let mut result = json!({});
        for key in ["code", "msg", "message", "requestId"] {
            if let Some(v) = data.get(key).filter(|v| v.is_string() || v.is_number()) {
                result[key] = safe_value(v, secrets);
            }
        }
        if let Some(message) = data["displayMsg"]["zh"].as_str() {
            result["displayMessage"] = json!(redact(message, secrets));
        }
        if let Some(error) = data.get("error") {
            if let Some(message) = error.as_str() {
                result["message"] = json!(redact(message, secrets));
            }
            for key in ["message", "type", "code", "param"] {
                if let Some(v) = error.get(key).filter(|v| v.is_string() || v.is_number()) {
                    result[key] = safe_value(v, secrets);
                }
            }
        }
        if result.as_object().is_some_and(|o| o.is_empty()) {
            result["message"] = json!("上游未提供可展示的错误详情");
        }
        if self.upstream_request_id.is_none() {
            self.upstream_request_id = result["requestId"].as_str().map(str::to_owned);
        }
        self.error = result;
    }
}
fn safe_value(value: &Value, secrets: &[&str]) -> Value {
    if let Some(s) = value.as_str() {
        json!(redact(s, secrets))
    } else {
        value.clone()
    }
}
fn redact(message: &str, secrets: &[&str]) -> String {
    let mut result = message.to_owned();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        result = result.replace(secret, "[REDACTED]");
    }
    bounded(&result, 2048)
}
pub fn bounded(value: &str, max: usize) -> String {
    let mut text = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        text.push('…');
    }
    text
}
pub fn summary(body: &Value) -> Value {
    let rows = body["messages"].as_array();
    let mut roles = BTreeMap::<String, usize>::new();
    let mut blocks = BTreeMap::<String, usize>::new();
    for row in rows.into_iter().flatten() {
        let role = row["role"]
            .as_str()
            .filter(|r| ["system", "developer", "user", "assistant", "tool"].contains(r))
            .unwrap_or("other");
        *roles.entry(role.into()).or_default() += 1;
        if row["content"].is_string() {
            *blocks.entry("text".into()).or_default() += 1;
        }
        for b in row["content"].as_array().into_iter().flatten() {
            let kind = b["type"]
                .as_str()
                .filter(|t| {
                    [
                        "text",
                        "image",
                        "image_url",
                        "tool_use",
                        "tool_result",
                        "thinking",
                        "document",
                    ]
                    .contains(t)
                })
                .unwrap_or("other");
            *blocks.entry(kind.into()).or_default() += 1;
        }
    }
    let mut out = json!({"messageCount":rows.map_or(0,Vec::len),"roles":roles,"contentBlocks":blocks,"toolCount":body["tools"].as_array().map_or(0,Vec::len),"bodyBytes":body.to_string().len(),"stream":body["stream"].as_bool().unwrap_or(false),"hasSystem":body.get("system").is_some()||roles.contains_key("system")});
    for key in ["max_tokens", "temperature", "top_p", "top_k"] {
        if let Some(value) = body.get(key).filter(|v| v.is_number()) {
            out[key] = value.clone();
        }
    }
    if let Some(effort) = body["reasoning_effort"]
        .as_str()
        .or_else(|| body["output_config"]["effort"].as_str())
    {
        if ["none", "minimal", "low", "medium", "high", "xhigh", "max"].contains(&effort) {
            out["reasoningEffort"] = json!(effort);
        }
    }
    let choice = body["tool_choice"]
        .as_str()
        .or_else(|| body["tool_choice"]["type"].as_str());
    if let Some(choice) =
        choice.filter(|s| ["auto", "any", "required", "none", "tool", "function"].contains(s))
    {
        out["toolChoice"] = json!(choice);
    }
    out
}

pub fn usage(value: &Value) -> Value {
    let mut out = json!({});
    for key in [
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
        "credit",
        "credits",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
        "prompt_cache_write_tokens",
    ] {
        if let Some(v) = value.get(key).filter(|v| v.is_number()) {
            out[key] = v.clone();
        }
    }
    for (key, field) in [
        ("prompt_tokens_details", "cached_tokens"),
        ("completion_tokens_details", "reasoning_tokens"),
    ] {
        if value[key][field].is_number() {
            out[key] = json!({});
            out[key][field] = value[key][field].clone();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summaries_never_copy_prompt_tool_arguments_or_schema() {
        let body = json!({"model":"kimi-k3-1","messages":[{"role":"user","content":"PRIVATE_PROMPT"},{"role":"assistant","tool_calls":[{"function":{"arguments":"PRIVATE_ARGUMENTS"}}]}],"tools":[{"name":"PRIVATE_TOOL","description":"PRIVATE_SCHEMA"}],"max_tokens":512,"output_config":{"effort":"medium"}});
        let value = summary(&body);
        let serialized = value.to_string();
        assert!(!serialized.contains("PRIVATE_"));
        assert_eq!(value["messageCount"], 2);
        assert_eq!(value["toolCount"], 1);
        assert_eq!(value["reasoningEffort"], "medium");
    }
    #[test]
    fn errors_are_allowlisted_bounded_and_credentials_are_redacted() {
        let mut details = RequestDetails::default();
        details.upstream_error(&json!({"code":11111,"msg":"blocked token-secret local-key","displayMsg":{"zh":"请求被拦截"},"requestId":"request-1","access_token":"NEVER_COPY","data":{"prompt":"NEVER_COPY"}}),&["token-secret","local-key"]);
        let serialized = details.error.to_string();
        assert!(!serialized.contains("token-secret"));
        assert!(!serialized.contains("local-key"));
        assert!(!serialized.contains("NEVER_COPY"));
        assert_eq!(details.error["code"], 11111);
        assert_eq!(details.upstream_request_id.as_deref(), Some("request-1"));
        details.upstream_error(&json!({"message":"x".repeat(5000)}), &[]);
        assert!(details.error["message"].as_str().unwrap().len() < 2100);
    }
    #[test]
    fn usage_only_keeps_numeric_counters_and_known_details() {
        let out = usage(
            &json!({"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":12,"prompt":"PRIVATE"},"completion_tokens_details":{"reasoning_tokens":8},"text":"PRIVATE"}),
        );
        assert_eq!(out["prompt_tokens_details"]["cached_tokens"], 12);
        assert_eq!(out["completion_tokens_details"]["reasoning_tokens"], 8);
        assert!(!out.to_string().contains("PRIVATE"));
    }
}
