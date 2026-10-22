//! Provider-neutral chat streaming: every upstream is normalized into OpenAI
//! chat.completion.chunk values before protocol conversion and aliasing.
use futures_util::Stream;
use serde_json::{json, Value};
use std::pin::Pin;

pub const MAX_OUTPUT: usize = 16 * 1024 * 1024;
pub const MAX_EVENT: usize = 2 * 1024 * 1024;

/// One normalized upstream event. `Done` is only emitted after a complete,
/// well-terminated upstream stream; a stream that ends without it is truncated.
#[derive(Debug, Clone)]
pub enum UpstreamEvent {
    Chunk(Value),
    Done,
}

/// A failure observed after the upstream accepted the request. Only
/// allowlisted, redacted fields belong in `error` (see diagnostics).
#[derive(Debug, Clone, Default)]
pub struct StreamFailure {
    pub message: String,
    pub code: Option<String>,
    pub error: Value,
    /// Business HTTP-like status reported inside the stream (e.g. 401/403/429).
    pub status: Option<u16>,
    /// Set when the upstream asked to queue: wait this long, then retry.
    pub retry_after: Option<u64>,
}
impl StreamFailure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }
}

pub type EventStream =
    Pin<Box<dyn Stream<Item = Result<UpstreamEvent, StreamFailure>> + Send + 'static>>;

/// Incremental SSE frame splitter with hard size limits. Yields the joined
/// `data:` payload of each complete frame and its optional `event:` name.
#[derive(Default)]
pub struct SseFrames {
    pending: Vec<u8>,
    frame: Vec<u8>,
    event: Option<String>,
    retained: usize,
}
pub struct Frame {
    pub event: Option<String>,
    pub data: Vec<u8>,
}
impl SseFrames {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, String> {
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
                continue;
            }
            if let Some(name) = line.strip_prefix(b"event:") {
                self.event = Some(String::from_utf8_lossy(name).trim().to_owned());
                continue;
            }
            if !line.is_empty() || self.frame.is_empty() {
                if line.is_empty() {
                    self.event = None;
                }
                continue;
            }
            out.push(Frame {
                event: self.event.take(),
                data: std::mem::take(&mut self.frame),
            });
        }
        Ok(out)
    }
    /// A final frame without the terminating blank line.
    pub fn finish(&mut self) -> Option<Frame> {
        if self.frame.is_empty() {
            return None;
        }
        Some(Frame {
            event: self.event.take(),
            data: std::mem::take(&mut self.frame),
        })
    }
}

pub fn normalized_finish_reason(reason: &str) -> String {
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
    }
}

/// Aggregate normalized chunks into one chat.completion (single choice).
pub struct Aggregate {
    pub value: Value,
    size: usize,
    pub finished: bool,
}
impl Aggregate {
    pub fn new(model: &str) -> Self {
        Self {
            value: json!({"id":format!("chatcmpl_{}",uuid::Uuid::new_v4().simple()),"object":"chat.completion","created":chrono::Utc::now().timestamp(),"model":model,"choices":[{"index":0,"message":{"role":"assistant","content":""},"finish_reason":null}],"usage":null}),
            size: 0,
            finished: false,
        }
    }
    pub fn usage(&self) -> Value {
        self.value["usage"].clone()
    }
    pub fn push(&mut self, v: &Value) -> Result<(), String> {
        self.size += v.to_string().len();
        if self.size > MAX_OUTPUT {
            return Err("聚合响应过大".into());
        }
        if v["usage"].is_object() {
            self.value["usage"] = v["usage"].clone();
        }
        for choice in v["choices"].as_array().into_iter().flatten() {
            if choice["index"].as_u64().unwrap_or(0) != 0 {
                return Err("当前只支持一个生成结果".into());
            }
            if choice["finish_reason"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
            {
                self.value["choices"][0]["finish_reason"] = choice["finish_reason"].clone();
                self.finished = true;
            }
            let m = &mut self.value["choices"][0]["message"];
            for field in ["content", "reasoning_content", "refusal"] {
                if let Some(text) = choice["delta"][field].as_str() {
                    let current = m[field].as_str().unwrap_or("");
                    m[field] = json!(format!("{current}{text}"));
                }
            }
            for tool in choice["delta"]["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let i = tool["index"].as_u64().unwrap_or(0) as usize;
                if i > 256 {
                    return Err("上游工具索引无效".into());
                }
                if !m["tool_calls"].is_array() {
                    m["tool_calls"] = json!([]);
                }
                let calls = m["tool_calls"].as_array_mut().unwrap();
                while calls.len() <= i {
                    calls.push(
                        json!({"type":"function","id":"","function":{"name":"","arguments":""}}),
                    );
                }
                if tool["id"].is_string() {
                    calls[i]["id"] = tool["id"].clone();
                }
                for field in ["name", "arguments"] {
                    if let Some(delta) = tool["function"][field].as_str() {
                        let current = calls[i]["function"][field].as_str().unwrap_or("");
                        calls[i]["function"][field] = json!(format!("{current}{delta}"));
                    }
                }
            }
        }
        Ok(())
    }
    /// Final value: tool-call-only replies carry null content, as upstream SDKs expect.
    pub fn finish(mut self) -> Value {
        let m = &mut self.value["choices"][0]["message"];
        if m["tool_calls"].as_array().is_some_and(|a| !a.is_empty()) && m["content"] == "" {
            m["content"] = Value::Null;
        }
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_split_bytes_crlf_events_and_limits() {
        let mut p = SseFrames::default();
        assert!(p.feed(b"event: output\ndata: {\"a\":").unwrap().is_empty());
        let frames = p.feed(b"1}\r\n\r\ndata: [DONE]\n\n").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].event.as_deref(), Some("output"));
        assert_eq!(frames[0].data, b"{\"a\":1}");
        assert_eq!(frames[1].event, None);
        assert_eq!(frames[1].data, b"[DONE]");
        assert!(SseFrames::default()
            .feed(&vec![b'x'; MAX_EVENT + 1])
            .is_err());
    }
    #[test]
    fn aggregate_parallel_tool_fragments_and_null_content() {
        let mut a = Aggregate::new("m");
        a.push(&json!({"choices":[{"delta":{"reasoning_content":"r","tool_calls":[{"index":0,"id":"a","function":{"name":"read","arguments":"{\"x\":"}},{"index":1,"id":"b","function":{"name":"write","arguments":"{}"}}]}}]})).unwrap();
        a.push(&json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":12}})).unwrap();
        assert!(a.finished);
        assert_eq!(a.usage()["prompt_tokens"], 12);
        let v = a.finish();
        let m = &v["choices"][0]["message"];
        assert_eq!(m["tool_calls"][0]["function"]["arguments"], "{\"x\":1}");
        assert_eq!(m["reasoning_content"], "r");
        assert!(m["content"].is_null());
    }
}
