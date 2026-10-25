//! Unified gateway and appearance settings (`~/.agentdock/settings.json`).
use agentdock_core::provider::ProviderKind;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_PORT: u16 = 24800;
pub const DEFAULT_CONCURRENCY: usize = 4;
/// Bumped when a default changes in a way saved files should pick up.
pub const SETTINGS_VERSION: u32 = 3;

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
            concurrency: DEFAULT_CONCURRENCY,
        }
    }
}

/// Unified daily check-in / credit claim schedule for all three platforms.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CheckinSettings {
    /// WorkBuddy CN daily check-in (accounts with check-in enabled).
    pub workbuddy: bool,
    /// Qoder daily credit campaigns.
    pub qoder: bool,
    /// Trae CN daily check-in (accounts with auto check-in on).
    pub trae: bool,
    /// Run one pass 10 seconds after AgentDock starts.
    pub on_startup: bool,
    /// Enable the shared application/system schedule.
    pub scheduled: bool,
    /// Legacy interval, retained when importing older settings.
    pub interval_minutes: u32,
    #[serde(default)]
    pub schedule: Option<crate::checkin_schedule::Schedule>,
}
impl Default for CheckinSettings {
    fn default() -> Self {
        Self {
            workbuddy: true,
            qoder: true,
            trae: true,
            on_startup: true,
            scheduled: true,
            interval_minutes: 60,
            schedule: Some(crate::checkin_schedule::Schedule::default()),
        }
    }
}
impl CheckinSettings {
    pub fn schedule(&self) -> crate::checkin_schedule::Schedule {
        self.schedule.clone().unwrap_or_else(|| {
            if self.interval_minutes == 60 {
                crate::checkin_schedule::Schedule::default()
            } else {
                crate::checkin_schedule::Schedule {
                    kind: crate::checkin_schedule::Kind::Minutes,
                    every: self.interval_minutes,
                    ..Default::default()
                }
            }
        })
    }
    pub fn enabled(&self, provider: ProviderKind) -> bool {
        match provider {
            ProviderKind::Workbuddy => self.workbuddy,
            ProviderKind::Qoder => self.qoder,
            ProviderKind::Trae => self.trae,
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
    /// Route id this route's upstreams join; its accounts are then scheduled
    /// with the destination's, and its names resolve to the destination.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_into: Option<String>,
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
    /// Accounts automatically suspended after login expiry/authentication failure.
    /// Kept until verified login recovery or manual acknowledgement. Recovery
    /// clears this evidence while leaving the native proxy switch disabled.
    pub auth_disabled_accounts: BTreeSet<String>,
    /// Reject browser (http/https Origin) requests; native and file:// clients pass.
    pub reject_browser_origin: bool,
    /// Start the gateway automatically when AgentDock opens.
    pub auto_start: bool,
    /// Images sent upstream per request, counted from the newest; older ones
    /// become a text note. `None` sends every image.
    pub recent_images: Option<u32>,
    pub theme: String,
    pub lang: String,
    pub timezone: String,
    pub token_format: String,
    pub checkin: CheckinSettings,
    /// Record the runtime log (gateway, providers, WorkBuddy SDK/agents, check-ins)
    /// and show the Logs page.
    pub logging: bool,
    /// Saved-file schema version (see `SETTINGS_VERSION`); absent in old files.
    #[serde(default)]
    pub version: u32,
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
            auth_disabled_accounts: BTreeSet::new(),
            reject_browser_origin: true,
            auto_start: false,
            recent_images: None,
            theme: "light".into(),
            lang: "system".into(),
            timezone: "system".into(),
            token_format: "km".into(),
            checkin: CheckinSettings::default(),
            logging: true,
            version: SETTINGS_VERSION,
        }
    }
}
impl Settings {
    pub fn scope(&self, provider: ProviderKind) -> ProviderScope {
        self.providers.get(&provider).cloned().unwrap_or_default()
    }
    /// Upgrade a file saved by an older version (missing `version` reads as 0).
    pub fn upgrade(&mut self) -> bool {
        if self.version >= SETTINGS_VERSION {
            return false;
        }
        if self.version < 2 {
            // The old default per-account concurrency was 2.
            for scope in self.providers.values_mut() {
                if scope.concurrency == 2 {
                    scope.concurrency = DEFAULT_CONCURRENCY;
                }
            }
        }
        if self.version < 3 && self.checkin.schedule.is_none() {
            self.checkin.schedule = Some(self.checkin.schedule());
        }
        self.version = SETTINGS_VERSION;
        true
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
        if self.recent_images.is_some_and(|n| !(1..=100).contains(&n)) {
            return Err("保留的图片数量需为 1–100".into());
        }
        self.checkin.schedule().validate()?;
        crate::checkin_schedule::validate_zone(&self.timezone)?;
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
    fn checkin_upgrade_defaults_to_seven_and_preserves_custom_intervals() {
        use crate::checkin_schedule::{Kind, Schedule};
        for (minutes, kind, every) in [(60, Kind::Daily, 1), (45, Kind::Minutes, 45)] {
            let mut settings: Settings = serde_json::from_value(serde_json::json!({
                "version": 2, "checkin": {"intervalMinutes": minutes, "scheduled": false}
            })).unwrap();
            assert!(settings.checkin.schedule.is_none());
            assert!(settings.upgrade());
            let schedule = settings.checkin.schedule();
            assert_eq!(schedule.kind, kind);
            assert_eq!(schedule.every, every);
            assert_eq!(schedule.time, "07:00");
            assert!(!settings.checkin.scheduled);
            settings.validate().unwrap();
        }
        let mut settings: Settings = serde_json::from_value(serde_json::json!({
            "version": 2, "checkin": {"schedule": {"kind":"weekly", "time":"09:30", "weekdays":[7]}}
        })).unwrap();
        settings.upgrade();
        assert_eq!(settings.checkin.schedule().kind, Kind::Weekly);
        assert_eq!(settings.checkin.schedule().weekdays, vec![7]);
        assert_eq!(Settings::default().checkin.schedule(), Schedule::default());
    }
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
    #[test]
    fn old_files_upgrade_default_concurrency_only() {
        let mut s: Settings = serde_json::from_str(
            r#"{"providers":{"qoder":{"concurrency":2},"trae":{"concurrency":3}}}"#,
        )
        .unwrap();
        assert_eq!(s.version, 0);
        assert!(s.upgrade());
        assert_eq!(s.scope(ProviderKind::Qoder).concurrency, 4);
        assert_eq!(s.scope(ProviderKind::Trae).concurrency, 3);
        assert!(!s.upgrade());
        assert_eq!(Settings::default().scope(ProviderKind::Workbuddy).concurrency, 4);
        assert_eq!(Settings::default().checkin.interval_minutes, 60);
        let mut bad = Settings::default();
        bad.checkin.schedule = None;
        bad.checkin.interval_minutes = 0;
        assert!(bad.validate().is_err());
    }
}
