//! Types shared by the unified gateway and each provider integration.
use crate::stream::EventStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Workbuddy,
    Qoder,
    Trae,
}
impl ProviderKind {
    pub const ALL: [ProviderKind; 3] = [Self::Workbuddy, Self::Qoder, Self::Trae];
    pub fn key(self) -> &'static str {
        match self {
            Self::Workbuddy => "workbuddy",
            Self::Qoder => "qoder",
            Self::Trae => "trae",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Workbuddy => "WorkBuddy",
            Self::Qoder => "Qoder",
            Self::Trae => "Trae",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == value)
    }
}

/// Unified regions. WorkBuddy calls its international edition "ai"; it is
/// mapped to "intl" at the boundary and never mixed with the domestic pool.
pub const REGIONS: [&str; 2] = ["cn", "intl"];
pub fn region_label(region: &str) -> &'static str {
    if region == "cn" {
        "国内版"
    } else {
        "国际版"
    }
}

/// One real model in one provider region catalog.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamModel {
    pub provider: Option<ProviderKind>,
    pub region: String,
    /// Real upstream identity (never an alias).
    pub id: String,
    /// Official display name, used for automatic cross-provider grouping.
    pub name: String,
    pub supports_tools: Option<bool>,
    pub supports_images: Option<bool>,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
    pub context_window: Option<u64>,
    pub max_output: Option<u64>,
    /// Provider configuration kept for execution (efforts, defaults, formats).
    #[serde(default)]
    pub config: Value,
}
impl UpstreamModel {
    pub fn provider(&self) -> ProviderKind {
        self.provider.unwrap_or(ProviderKind::Workbuddy)
    }
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.provider().key(), self.region, self.id)
    }
}

/// Normalized name used to merge identical models across providers/regions.
/// Case, whitespace and separator differences ("GLM-5.3 Flash" vs
/// "glm-5.3-flash") collapse; any other difference keeps models apart.
pub fn group_key(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_whitespace() || c == '_' || c == '-' {
            if !out.is_empty() {
                dash = true;
            }
        } else {
            if dash {
                out.push('-');
                dash = false;
            }
            out.push(c);
        }
    }
    out
}

/// Cached billing data used for ranking. Selecting an account never queries upstream.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreditRank {
    /// Remaining credits in comparable credit units; None when unknown or
    /// denominated in another unit (e.g. Trae international USD balances).
    pub remaining: Option<f64>,
    /// Earliest expiry (ms) among packs that still have remaining credits.
    pub earliest_expiry: Option<i64>,
    pub unit: String,
    pub updated_at: Option<i64>,
}

/// One account as seen by the unified scheduler.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolAccount {
    pub provider: ProviderKind,
    /// Provider-local identifier.
    pub id: String,
    pub region: String,
    pub name: String,
    /// Account-level proxy switch.
    pub enabled: bool,
    /// Login/token expiry in ms, when known.
    pub expires_at: Option<i64>,
    pub credit: CreditRank,
    /// Stable position within the provider's saved account order.
    pub order: usize,
    /// A static reason this account can never serve right now (no token…).
    pub blocked: Option<String>,
}
impl PoolAccount {
    pub fn key(&self) -> String {
        account_key(self.provider, &self.id)
    }
}
pub fn account_key(provider: ProviderKind, id: &str) -> String {
    format!("{}/{}", provider.key(), id)
}

/// Only explicit authentication signals; never infer expiry from free-form
/// error text, quota/model permission failures, or a generic HTTP 403.
pub fn login_rejected(status: u16, error: &Value) -> bool {
    status == 401 || [
        &error["code"], &error["error"]["code"],
        &error["type"], &error["error"]["type"],
    ].into_iter().any(|code| {
        code.as_u64() == Some(401) || code.as_str().is_some_and(|s| matches!(
            s.to_ascii_lowercase().as_str(),
            "401" | "unauthorized" | "authentication_error" | "token_expired"
                | "access_token_expired" | "invalid_token" | "invalid_access_token"
        ))
    })
}

/// An opaque equality stamp for a credential snapshot; never a credential or
/// an authentication check. Kept out of logs and client diagnostics.
pub fn credential_revision(token: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut hash);
    hash.finish()
}

/// Request handed to a provider after protocol conversion and alias resolution.
pub struct SendInput {
    /// Canonical chat.completions body; `model` is the real upstream id.
    pub body: Value,
    pub headers: http::HeaderMap,
    pub request_id: String,
    pub session_id: Option<String>,
    /// Effort hints from Messages/Responses (`output_config.effort`, `thinking`).
    pub effort_input: Value,
    /// Downstream client profile, e.g. "zcode"; local provenance only.
    pub client_profile: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AccountEffect {
    None,
    /// Exclude the account for this many seconds.
    Cooldown(u64),
    /// Exclude until the user re-enables the account.
    AuthFailed,
}

/// A failure before any model output reached the client.
#[derive(Clone, Debug)]
pub struct SendFailure {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub effect: AccountEffect,
    /// Allowlisted upstream error fields (code, request id…).
    pub error: Value,
    /// Native error type for Anthropic clients (e.g. permission_error).
    pub anthropic_type: Option<String>,
    pub retryable: Option<bool>,
    pub credential_revision: Option<u64>,
}
impl SendFailure {
    pub fn local(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            effect: AccountEffect::None,
            error: Value::Null,
            anthropic_type: None,
            retryable: None,
            credential_revision: None,
        }
    }
    pub fn with_effect(mut self, effect: AccountEffect) -> Self {
        self.effect = effect;
        self
    }
}

pub struct SendOk {
    pub events: EventStream,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_rejection_requires_explicit_authentication_evidence() {
        assert!(login_rejected(401, &Value::Null));
        assert!(login_rejected(200, &serde_json::json!({"code":401})));
        assert!(login_rejected(403, &serde_json::json!({"error":{"code":"TOKEN_EXPIRED"}})));
        for status in [200, 400, 403, 429, 500] {
            assert!(!login_rejected(status, &serde_json::json!({"code":"model_not_allowed", "message":"token expired"})));
            assert!(!login_rejected(status, &serde_json::json!({"code":"insufficient_quota"})));
        }
    }
    #[test]
    fn group_keys_merge_case_and_separators_only() {
        assert_eq!(group_key("GLM-5.3-Flash"), group_key("glm 5.3 flash"));
        assert_eq!(group_key(" Kimi_K3 "), "kimi-k3");
        assert_ne!(group_key("GLM-5.3"), group_key("GLM-5.3-Flash"));
        assert_ne!(group_key("DeepSeek-V4.1"), group_key("DeepSeek-V4"));
    }
}
