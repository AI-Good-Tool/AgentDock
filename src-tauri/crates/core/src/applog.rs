//! Process-wide runtime log shared by every provider crate.
//!
//! Disabled logging costs one relaxed atomic load: entries are built inside a
//! closure that only runs when a sink is installed and logging is on. The app
//! crate owns storage (bounded ring + background file writer).
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Monotonic per-process id (stable key for the UI).
    pub seq: u64,
    /// Unix milliseconds.
    pub time: i64,
    pub level: Level,
    /// Component, e.g. "gateway", "workbuddy.sdk", "workbuddy.agent", "checkin".
    pub source: String,
    pub message: String,
    /// Small structured context; never prompts, outputs or credentials.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub fields: Value,
}

type Sink = Box<dyn Fn(Entry) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();
static ENABLED: AtomicBool = AtomicBool::new(false);
static SEQ: AtomicU64 = AtomicU64::new(1);

impl Entry {
    /// Allocate a process-unique UI key, including for restored history.
    pub fn new(time: i64, level: Level, source: &str, message: String, fields: Value) -> Self {
        Self { seq: SEQ.fetch_add(1, Ordering::Relaxed), time, level, source: source.into(), message, fields }
    }
}

/// Install the process sink once; later calls are ignored.
pub fn install(sink: impl Fn(Entry) + Send + Sync + 'static) {
    let _ = SINK.set(Box::new(sink));
}
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}
#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Record one entry; `build` runs only when logging is on.
#[inline]
pub fn emit(level: Level, source: &str, build: impl FnOnce() -> (String, Value)) {
    if !enabled() {
        return;
    }
    let Some(sink) = SINK.get() else { return };
    let (message, fields) = build();
    sink(Entry::new(chrono::Utc::now().timestamp_millis(), level, source, message, fields));
}
pub fn info(source: &str, build: impl FnOnce() -> (String, Value)) {
    emit(Level::Info, source, build)
}
pub fn warn(source: &str, build: impl FnOnce() -> (String, Value)) {
    emit(Level::Warn, source, build)
}
pub fn error(source: &str, build: impl FnOnce() -> (String, Value)) {
    emit(Level::Error, source, build)
}
pub fn debug(source: &str, build: impl FnOnce() -> (String, Value)) {
    emit(Level::Debug, source, build)
}
