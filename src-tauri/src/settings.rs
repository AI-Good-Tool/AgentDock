//! Unified gateway and appearance settings (`~/.agentdock/settings.json`).
use agentdock_core::provider::ProviderKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_PORT: u16 = 24800;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Strategy {
    /// Per-model rotation in saved account order.
    #[default]
    RoundRobin,
    /// Most remaining credits first (unknown balances last).
    CreditFirst,
    /// Earliest-expiring remaining credits first (no expiry last).
    ExpiryFirst,
    /// Always the first eligible account in saved order.
    Ordered,
    Random,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderScope {
    /// Whether this provider's accounts join the gateway at all.
    pub enabled: bool,
    /// Participating regions ("cn", "intl").
    pub regions: Vec<String>,
    /// Concurrent chat requests per account.
    pub concurrency: usize,
}
impl Default for ProviderScope {
    fn default() -> Self {
        Self {
            enabled: true,
            regions: vec!["cn".into(), "intl".into()],
            concurrency: 2,
        }
    }
}

/// Saved alias/targets for one model route. Auto routes need no entry.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RouteOverride {
    /// Public model name returned to clients; empty keeps the default.
    pub alias: String,
    /// Explicit upstream keys ("provider:region:id"); None keeps automatic grouping.
    pub targets: Option<Vec<String>>,
    /// Hide the route from /models and reject requests for it.
    pub disabled: bool,
    /// User-created route (not derived from a catalog name).
    pub custom: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub port: u16,
    pub strategy: Strategy,
    pub providers: BTreeMap<ProviderKind, ProviderScope>,
    pub routes: BTreeMap<String, RouteOverride>,
    /// Fixed account (account key "provider/id") per path scope ("all", "cn", "intl", "trae", "trae/cn"…).
    pub pinned: BTreeMap<String, String>,
    /// Reject browser (http/https Origin) requests; native and file:// clients pass.
    pub reject_browser_origin: bool,
    /// Start the gateway automatically when AgentDock opens.
    pub auto_start: bool,
    pub theme: String,
    pub lang: String,
    pub timezone: String,
    pub token_format: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            strategy: Strategy::RoundRobin,
            providers: ProviderKind::ALL
                .into_iter()
                .map(|p| (p, ProviderScope::default()))
                .collect(),
            routes: BTreeMap::new(),
            pinned: BTreeMap::new(),
            reject_browser_origin: true,
            auto_start: false,
            theme: "light".into(),
            lang: "system".into(),
            timezone: "system".into(),
            token_format: "km".into(),
        }
    }
}
impl Settings {
    pub fn scope(&self, provider: ProviderKind) -> ProviderScope {
        self.providers.get(&provider).cloned().unwrap_or_default()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.port < 1024 {
            return Err("端口应为 1024–65535".into());
        }
        for (p, s) in &self.providers {
            if s.regions.iter().any(|r| !["cn", "intl"].contains(&r.as_str())) {
                return Err(format!("{} 的地区设置无效", p.label()));
            }
            if !(1..=16).contains(&s.concurrency) {
                return Err("单账号并发需为 1–16".into());
            }
        }
        if !["light", "dark", "system"].contains(&self.theme.as_str()) {
            return Err("主题无效".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_cover_every_provider_and_partial_files_load() {
        let s: Settings = serde_json::from_str(r#"{"port":25000,"strategy":"creditFirst"}"#).unwrap();
        assert_eq!(s.port, 25000);
        assert_eq!(s.strategy, Strategy::CreditFirst);
        assert_eq!(s.providers.len(), 3);
        assert!(s.scope(ProviderKind::Trae).enabled);
        assert!(s.reject_browser_origin);
        s.validate().unwrap();
        let mut bad = s.clone();
        bad.providers.get_mut(&ProviderKind::Qoder).unwrap().regions = vec!["eu".into()];
        assert!(bad.validate().is_err());
    }
}
