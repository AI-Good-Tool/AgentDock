//! WorkBuddy provider for AgentDock: account pool metadata, the installed
//! official transport, catalog sync, daily check-in, credit/token statistics
//! and client switching (via the vendored WorkBuddy Switch core).
pub use agentdock_core::{anthropic, diagnostics, MAX_REQUEST_BODY_BYTES, REQUEST_TOO_LARGE_MESSAGE};

pub mod account_priority;
pub mod account_transfer;
pub mod auth;
pub mod checkin;
pub mod conversation;
pub mod model_registry;
pub mod official;
pub mod official_transport;
pub mod pool;
pub mod protocol;
pub mod proxy;
pub mod settings;
pub mod statistics;
pub mod switch_bridge;
pub mod upstream;
