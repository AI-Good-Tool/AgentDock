//! AgentDock shared core: client protocol conversion (Anthropic Messages,
//! OpenAI Responses, Chat Completions), bounded diagnostics and the
//! provider-neutral streaming contract used by every upstream integration.
pub mod anthropic;
pub mod applog;
pub mod custom_tools;
pub mod diagnostics;
pub mod provider;
pub mod response_tools;
pub mod responses;
pub mod session;
pub mod stream;

/// Bound raw client JSON and normalized upstream JSON consistently.
pub const MAX_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;
pub const REQUEST_TOO_LARGE_MESSAGE: &str = "请求体超过本地 32 MiB 上限，请缩短会话历史或减少附件后重试；反复发送相同请求不会解决大小超限。";
