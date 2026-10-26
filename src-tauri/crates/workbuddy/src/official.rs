//! Desktop sidecar profile verified against the installed 5.9.2 client.
//! Chat SDK headers belong here, not on unrelated billing/config requests.
use crate::pool::{Account, Credential, Shared};
use axum::http::HeaderMap;
use flate2::{Compression, GzBuilder};
use serde_json::{json, Value};
use std::io::Write;
use uuid::Uuid;

pub const DESKTOP_VERSION: &str = "5.9.2";
pub const REQUEST_PROFILE: &str = "WorkBuddy Desktop 5.9.2 / CLI 2.165.0";
pub const USER_AGENT: &str = "WorkBuddy/5.9.2 WorkBuddy/5.9.2 CLI/2.165.0";
pub fn profile_summary(use_device_sdk: bool) -> Value {
    json!({"desktopVersion":"5.9.2","cliVersion":"2.165.0","nodeVersion":"v22.21.1","userAgent":USER_AGENT,"ideType":"WorkBuddy","ideVersion":"5.9.2","gzip":true,"transport":"installed-electron-node","deviceToken": if use_device_sdk { "installed-native-sdk" } else { "disabled" },"evidence":"official-desktop-offline.json"})
}
pub fn prepare_body(body: &mut Value, config: Option<&Value>) {
    body["stream"] = json!(true);
    if let Some(config) = config {
        for (field, source) in [
            ("temperature", "temperature"),
            ("max_tokens", "maxOutputTokens"),
        ] {
            if body.get(field).is_none() {
                if let Some(value) = config.get(source).filter(|v| !v.is_null()) {
                    body[field] = value.clone();
                }
            }
        }
        if body.get("reasoning_effort").is_none() && body.get("reasoning").is_none() {
            if let Some(effort) = config["reasoning"]["defaultEffort"]
                .as_str()
                .or_else(|| config["reasoning"]["effort"].as_str())
            {
                body["reasoning_effort"] = json!(effort);
            }
        }
        if body.get("reasoning_summary").is_none() {
            if let Some(summary) = config["reasoning"]["summary"].as_str() {
                body["reasoning_summary"] = json!(summary);
            }
        }
    }
    if !body["stream_options"].is_object() {
        body["stream_options"] = json!({});
    }
    body["stream_options"]["include_usage"] = json!(true);
    if body.get("verbosity").is_none() {
        body["verbosity"] = json!("high");
    }
}
pub fn request(
    state: &Shared,
    a: &Account,
    c: &Credential,
    route: &str,
) -> reqwest::RequestBuilder {
    let mut request = state
        .client
        .post(format!("{}{route}", crate::pool::endpoint(a)))
        .bearer_auth(&c.token)
        .header("X-User-Id", &c.uid)
        .header("X-Domain", &c.domain)
        .header("X-Product", "SaaS")
        .header("User-Agent", USER_AGENT)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("X-Request-ID", Uuid::now_v7().simple().to_string());
    if let Some(id) = &c.enterprise {
        request = request
            .header("X-Enterprise-Id", id)
            .header("X-Tenant-Id", id);
    }
    request
}

const AUXILIARY: &[&str] = &[
    "summary",
    "webfetch",
    "conversation_topic",
    "memory_selection",
    "insights",
    "prompt_hook",
    "agent_instructions",
    "context_compact",
    "context_summary_pre_message",
    "context_summary_max_token",
    "prompt_suggestion",
    "memory_extraction",
    "auto_mode_classifier",
    "security_reviewer",
    "content_security_reviewer",
    "side_question",
    "subagent",
];
pub fn purpose(headers: &HeaderMap, allow_auxiliary: bool) -> Result<String, &'static str> {
    let value = headers
        .get("x-agent-purpose")
        .map(|v| v.to_str().map_err(|_| "Agent 用途头无效"))
        .transpose()?
        .unwrap_or("conversation");
    if value == "conversation" || value == "conversation:compact" || value == "automation" {
        return Ok(value.into());
    }
    if !AUXILIARY.contains(&value) {
        return Err("未知 Agent 用途；请使用官方用途名称");
    }
    if !allow_auxiliary {
        return Err("客户端辅助 Agent 请求已在接入设置中禁用");
    }
    Ok(value.into())
}

pub(crate) fn agent_type(purpose: &str) -> &'static str {
    match purpose {
        "context_compact" | "context_summary_pre_message" | "context_summary_max_token" => {
            "compaction"
        }
        "subagent" => "subagent",
        value if AUXILIARY.contains(&value) => "auxiliary",
        _ => "main",
    }
}

/// Standalone diagnostic entry point. Production uses a retained session context.
pub fn chat_request(
    builder: reqwest::RequestBuilder,
    incoming: &HeaderMap,
    purpose: &str,
    body: &Value,
) -> Result<reqwest::RequestBuilder, String> {
    let context = crate::conversation::Sessions::default().prepare(incoming, &mut body.clone());
    chat_request_with_context(builder, &context, purpose, body)
}

pub fn chat_request_with_context(
    builder: reqwest::RequestBuilder,
    context: &crate::conversation::Context,
    purpose: &str,
    body: &Value,
) -> Result<reqwest::RequestBuilder, String> {
    // The bridge normalizes JSON and re-encodes using the installed Node zlib.
    // This staging buffer also uses the observed macOS gzip OS marker (19).
    let mut gzip = GzBuilder::new()
        .operating_system(19)
        .write(Vec::new(), Compression::default());
    let plain = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    if plain.len() > crate::MAX_REQUEST_BODY_BYTES {
        return Err(crate::REQUEST_TOO_LARGE_MESSAGE.into());
    }
    gzip.write_all(&plain).map_err(|e| e.to_string())?;
    let bytes = gzip.finish().map_err(|e| e.to_string())?;
    let mut request_header = HeaderMap::new();
    request_header.insert(
        "x-request-id",
        context.request.parse().map_err(|_| "请求 ID 格式无效")?,
    );
    Ok(builder
        .headers(request_header)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header(
            "X-Stainless-Arch",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x64"
            },
        )
        .header("X-Stainless-Lang", "js")
        .header(
            "X-Stainless-OS",
            match std::env::consts::OS {
                "windows" => "Windows",
                "macos" => "MacOS",
                _ => "Linux",
            },
        )
        .header("X-Stainless-Package-Version", "6.25.0")
        .header("X-Stainless-Retry-Count", "0")
        .header("X-Stainless-Runtime", "node")
        .header("X-Stainless-Runtime-Version", "v22.21.1")
        .header("X-Conversation-ID", &context.conversation)
        .header("ACP-Connection-ID", &context.connection)
        .header("X-Conversation-Message-ID", &context.request)
        .header("X-Conversation-Request-ID", &context.root)
        .header("X-Root-Request-ID", &context.root)
        .header("X-Agent-Intent", "craft")
        .header("X-Agent-Runtime", "other")
        .header(
            "baggage",
            format!(
                "codebuddy.session_id={},codebuddy.conversation_request_id={}",
                context.conversation, context.root
            ),
        )
        .header("X-Agent-Purpose", purpose)
        .header("X-Agent-Type", agent_type(purpose))
        .header("X-IDE-Type", "WorkBuddy")
        .header("X-IDE-Name", "WorkBuddy")
        .header("X-IDE-Version", "5.9.2")
        .header("X-Private-Data", "true")
        .header("X-Codebuddy-Request", "1")
        .header(
            "traceparent",
            format!("00-{}-{}-0{}", context.trace, context.span, context.sampled),
        )
        .header(
            "b3",
            format!(
                "{}-{}-{}-{}",
                context.trace, context.span, context.sampled, context.parent
            ),
        )
        .header("X-B3-TraceId", &context.trace)
        .header("X-B3-SpanId", &context.span)
        .header("X-B3-ParentSpanId", &context.parent)
        .header("X-B3-Sampled", context.sampled)
        .header("X-Trace-ID", &context.trace)
        .header("Accept-Encoding", "gzip, compress, deflate, br")
        .header("Content-Encoding", "gzip")
        .body(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_payload_is_bounded_before_device_sdk_or_network() {
        let client = reqwest::Client::new();
        let input = json!({"messages":[{"role":"user","content":"x".repeat(5*1024*1024)}]});
        let request = chat_request(
            client.post("https://www.workbuddy.cn/v2/chat/completions"),
            &HeaderMap::new(),
            "main",
            &input,
        )
        .unwrap()
        .build()
        .unwrap();
        let bytes = request.body().unwrap().as_bytes().unwrap();
        let mut decoded = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(bytes), &mut decoded).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&decoded).unwrap(), input);
        let too_large = json!({"messages":[{"role":"user","content":"x".repeat(crate::MAX_REQUEST_BODY_BYTES)}]});
        let error = chat_request(
            client.post("https://www.workbuddy.cn/v2/chat/completions"),
            &HeaderMap::new(),
            "main",
            &too_large,
        )
        .err()
        .unwrap();
        assert_eq!(error, crate::REQUEST_TOO_LARGE_MESSAGE);
    }
    #[test]
    fn defaults_match_model_catalog_without_replacing_client_context_or_limits() {
        let config = json!({"temperature":1,"maxOutputTokens":128000,"reasoning":{"defaultEffort":"high","summary":"auto"}});
        let mut body = json!({"model":"deepseek-v4.1-flash","max_tokens":512,"temperature":0.7,"messages":[{"role":"system","content":"client instructions"}],"tools":[{"function":{"name":"client_tool"}}],"extra_vars":{"growthEvent":"current-client-event"}});
        prepare_body(&mut body, Some(&config));
        assert_eq!(body["model"], "deepseek-v4.1-flash");
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(body["messages"][0]["content"], "client instructions");
        assert_eq!(body["tools"][0]["function"]["name"], "client_tool");
        assert_eq!(body["extra_vars"]["growthEvent"], "current-client-event");
        assert_eq!(body["reasoning_summary"], "auto");
        assert_eq!(body["stream_options"]["include_usage"], true);
        let mut default = json!({"model":"deepseek-v4.1-flash"});
        prepare_body(&mut default, Some(&config));
        assert_eq!(default["max_tokens"], 128000);
        assert!(default.get("extra_vars").is_none());
    }
    #[test]
    fn auxiliary_switch_is_effective_without_changing_main_conversations() {
        let mut headers = HeaderMap::new();
        assert_eq!(purpose(&headers, false).unwrap(), "conversation");
        headers.insert("x-agent-purpose", "conversation_topic".parse().unwrap());
        assert!(purpose(&headers, false).is_err());
        assert_eq!(purpose(&headers, true).unwrap(), "conversation_topic");
        assert_eq!(agent_type("conversation_topic"), "auxiliary");
        assert_eq!(agent_type("context_summary_pre_message"), "compaction");
        assert_eq!(agent_type("conversation"), "main");
        headers.insert("x-agent-purpose", "automation".parse().unwrap());
        assert_eq!(purpose(&headers, false).unwrap(), "automation");
        headers.insert("x-agent-purpose", "unknown".parse().unwrap());
        assert!(purpose(&headers, true).is_err());
    }
    #[tokio::test]
    async fn wire_headers_ids_and_gzip_match_official_profile() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
        let app = axum::Router::new().route(
            "/",
            axum::routing::post(
                move |headers: HeaderMap, bytes: axum::body::Bytes| async move {
                    tx.lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .send((headers, bytes))
                        .unwrap();
                    "OK"
                },
            ),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let input =
            json!({"model":"hy3","stream":true,"messages":[{"role":"user","content":"offline"}]});
        let state =
            crate::pool::AppState::new(std::env::temp_dir().join(Uuid::new_v4().to_string()))
                .unwrap();
        let account = Account {
            id: "fixture".into(),
            name: "fixture".into(),
            source: "fixture".into(),
            source_id: "fixture".into(),
            variant: "cn".into(),
            enabled: true,
            checkin_enabled: false,
            note: String::new(),
            display_field: String::new(),
            expires_at: None,
        };
        let credential = Credential {
            token: "offline-fixture".into(),
            uid: "offline-uid".into(),
            domain: "www.codebuddy.cn".into(),
            enterprise: None,
        };
        let mut incoming = HeaderMap::new();
        incoming.insert(
            "authorization",
            "Bearer untrusted-downstream".parse().unwrap(),
        );
        incoming.insert("user-agent", "untrusted-client".parse().unwrap());
        incoming.insert("x-ide-type", "untrusted-ide".parse().unwrap());
        incoming.insert("x-user-id", "untrusted-user".parse().unwrap());
        incoming.insert("cookie", "untrusted-cookie".parse().unwrap());
        incoming.insert("traceparent", "untrusted-trace".parse().unwrap());
        let mut request = chat_request(
            request(&state, &account, &credential, "/v2/chat/completions"),
            &incoming,
            "conversation",
            &input,
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://www.workbuddy.cn/v2/chat/completions"
        );
        *request.url_mut() = format!("http://{address}/").parse().unwrap();
        state.client.execute(request).await.unwrap();
        let (headers, bytes) = rx.await.unwrap();
        assert_eq!(headers["authorization"], "Bearer offline-fixture");
        assert_eq!(headers["x-user-id"], "offline-uid");
        assert_eq!(headers["x-domain"], "www.codebuddy.cn");
        assert_eq!(headers["host"].to_str().unwrap(), address.to_string());
        assert_eq!(
            headers["content-length"]
                .to_str()
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            bytes.len()
        );
        assert!(!headers.contains_key("cookie"));
        assert!(!headers.contains_key("x-product-version"));
        for name in headers.keys() {
            assert_eq!(
                headers.get_all(name).iter().count(),
                1,
                "duplicate header: {name}"
            );
        }
        for name in [
            "x-conversation-id",
            "x-request-id",
            "x-conversation-request-id",
        ] {
            assert_eq!(
                Uuid::parse_str(headers[name].to_str().unwrap())
                    .unwrap()
                    .get_version_num(),
                if name == "x-conversation-id" { 4 } else { 7 },
                "wrong UUID version: {name}"
            );
        }
        let mut decoded = Vec::new();
        std::io::Read::read_to_end(
            &mut flate2::read::GzDecoder::new(bytes.as_ref()),
            &mut decoded,
        )
        .unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&decoded).unwrap(), input);
        assert_eq!(headers.get_all("x-request-id").iter().count(), 1);
        assert_eq!(
            headers["x-request-id"],
            headers["x-conversation-message-id"]
        );
        assert_eq!(
            headers["x-root-request-id"],
            headers["x-conversation-request-id"]
        );
        assert_ne!(headers["x-root-request-id"], headers["x-request-id"]);
        assert_eq!(headers["content-encoding"], "gzip");
        assert_eq!(headers["x-stainless-runtime-version"], "v22.21.1");
        assert_eq!(
            headers["x-stainless-os"],
            if cfg!(windows) {
                "Windows"
            } else if cfg!(target_os = "macos") {
                "MacOS"
            } else {
                "Linux"
            }
        );
        assert_eq!(
            headers["x-stainless-arch"],
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x64"
            }
        );
        assert_eq!(headers["x-ide-version"], "5.9.2");
        assert_eq!(headers["x-b3-traceid"], headers["x-trace-id"]);
        assert_eq!(
            headers["traceparent"].to_str().unwrap(),
            format!(
                "00-{}-{}-01",
                headers["x-trace-id"].to_str().unwrap(),
                headers["x-b3-spanid"].to_str().unwrap()
            )
        );
        assert!(!headers.contains_key("x-model-id"));
        let fixture: Value =
            serde_json::from_str(include_str!("../evidence/official-desktop-offline.json"))
                .unwrap();
        let dynamic = [
            "host",
            "x-stainless-os",
            "x-stainless-arch",
            "connection",
            "content-length",
            "authorization",
            "x-user-id",
            "x-domain",
            "x-conversation-id",
            "x-request-id",
            "x-conversation-message-id",
            "x-conversation-request-id",
            "x-root-request-id",
            "baggage",
            "x-private-data",
            "traceparent",
            "b3",
            "x-b3-traceid",
            "x-b3-spanid",
            "x-trace-id",
        ];
        for (name, value) in fixture["records"][0]["headers"].as_object().unwrap() {
            if !dynamic.contains(&name.as_str()) {
                assert_eq!(
                    headers.get(name).and_then(|v| v.to_str().ok()),
                    value.as_str(),
                    "official header mismatch: {name}"
                );
            }
        }
        assert_eq!(
            headers["b3"].to_str().unwrap(),
            format!(
                "{}-{}-1-{}",
                headers["x-trace-id"].to_str().unwrap(),
                headers["x-b3-spanid"].to_str().unwrap(),
                headers["x-b3-parentspanid"].to_str().unwrap()
            )
        );
        for name in headers.keys() {
            assert!(
                ["acp-connection-id", "x-b3-parentspanid"].contains(&name.as_str())
                    || fixture["records"][0]["headers"]
                        .get(name.as_str())
                        .is_some(),
                "unexpected official header: {name}"
            );
        }
        server.abort();
    }
}
