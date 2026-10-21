use crate::pool::Shared;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub account_refresh_enabled: bool,
    pub account_refresh_minutes: u32,
    pub allow_auxiliary_agents: bool,
    pub use_device_sdk: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            account_refresh_enabled: false,
            account_refresh_minutes: 30,
            allow_auxiliary_agents: true,
            use_device_sdk: false,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !(5..=1440).contains(&self.account_refresh_minutes) {
            return Err("刷新间隔需为 5 至 1440 分钟".into());
        }
        Ok(())
    }
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshStatus {
    pub next_at: Option<i64>,
    pub last_at: Option<i64>,
    pub running: bool,
    pub success: usize,
    pub failed: usize,
}

/// A single gate covers manual and scheduled batches. Never refreshes login tokens.
pub async fn refresh_accounts(state: Shared, ids: Vec<String>) -> Result<Value, String> {
    refresh_batch(state, ids, false).await
}
async fn refresh_batch(state: Shared, ids: Vec<String>, scheduled: bool) -> Result<Value, String> {
    let _guard = state
        .refresh_gate
        .try_lock()
        .map_err(|_| "已有账号刷新任务正在运行")?;
    state.runtime.lock().unwrap().account_refresh.running = true;
    struct Running(Shared);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.runtime.lock().unwrap().account_refresh.running = false;
        }
    }
    let _running = Running(state.clone());
    // Local status scanning has its own monitor/button; don't repeat expensive
    // client/log scans before a billing-only refresh.
    let mut results = vec![];
    for id in ids {
        if scheduled && !state.store.lock().unwrap().settings.account_refresh_enabled {
            break;
        }
        if !state
            .store
            .lock()
            .unwrap()
            .accounts
            .iter()
            .any(|a| a.id == id && (!scheduled || a.enabled))
        {
            continue;
        }
        let result = match crate::switch_bridge::refresh_credit(state.clone(), id.clone()).await {
            Ok(v) => v,
            Err(e) => {
                let v = json!({"ok":false,"error":e});
                let (state, id, cached) = (state.clone(), id.clone(), v.clone());
                let _ = tokio::task::spawn_blocking(move || state.cache_credit(&id, cached)).await;
                v
            }
        };
        results.push(json!({"id":id,"ok":result["ok"],"error":result["error"]}));
    }
    let mut rt = state.runtime.lock().unwrap();
    rt.account_refresh.running = false;
    rt.account_refresh.last_at = Some(Utc::now().timestamp_millis());
    rt.account_refresh.success = results.iter().filter(|v| v["ok"] == true).count();
    rt.account_refresh.failed = results.len() - rt.account_refresh.success;
    Ok(json!({"results":results}))
}

pub fn next_due(settings: &Settings, next: Option<i64>, now: i64) -> Option<i64> {
    settings
        .account_refresh_enabled
        .then(|| next.unwrap_or(now + i64::from(settings.account_refresh_minutes) * 60_000))
}

pub async fn run_scheduler(state: Shared) {
    let mut timer = tokio::time::interval(std::time::Duration::from_secs(15));
    loop {
        timer.tick().await;
        let now = Utc::now().timestamp_millis();
        let ids = {
            let store = state.store.lock().unwrap();
            let mut rt = state.runtime.lock().unwrap();
            let settings = &store.settings;
            rt.account_refresh.next_at = next_due(settings, rt.account_refresh.next_at, now);
            if !rt.account_refresh.next_at.is_some_and(|at| at <= now) {
                continue;
            }
            rt.account_refresh.next_at =
                Some(now + i64::from(settings.account_refresh_minutes) * 60_000);
            store
                .accounts
                .iter()
                .filter(|a| a.enabled)
                .map(|a| a.id.clone())
                .collect::<Vec<_>>()
        };
        // Recheck the switch immediately before a batch starts. No immediate query on enable/startup.
        if state.store.lock().unwrap().settings.account_refresh_enabled {
            let _ = refresh_batch(state.clone(), ids, true).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_is_opt_in_and_interval_is_bounded() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(!s.account_refresh_enabled);
        assert!(s.allow_auxiliary_agents);
        assert!(!s.use_device_sdk);
        assert_eq!(next_due(&s, Some(1), 100), None);
        let mut s = s;
        s.account_refresh_enabled = true;
        assert_eq!(next_due(&s, None, 100), Some(1_800_100));
        assert_eq!(next_due(&s, Some(50), 100), Some(50));
        s.account_refresh_minutes = 1;
        assert!(s.validate().is_err());
    }
    #[test]
    fn sdk_is_explicit_opt_in_persisted_and_can_be_disabled_without_confirmation() {
        let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let state = crate::pool::AppState::new(path.clone()).unwrap();
        let mut settings = Settings::default();
        assert!(!state.snapshot()["settings"]["useDeviceSdk"]
            .as_bool()
            .unwrap());
        settings.use_device_sdk = true;
        assert!(state.set_settings(settings.clone(), false).is_err());
        assert!(!path.exists());
        state.set_settings(settings.clone(), true).unwrap();
        let loaded = crate::pool::AppState::new(path).unwrap();
        assert!(loaded.store.lock().unwrap().settings.use_device_sdk);
        settings.use_device_sdk = false;
        loaded.set_settings(settings, false).unwrap();
        assert_eq!(
            loaded.snapshot()["officialProfile"]["deviceToken"],
            "disabled"
        );
    }
    #[tokio::test]
    async fn overlapping_refreshes_fail_before_scanning_credentials() {
        let state = crate::pool::AppState::new(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()))
        .unwrap();
        let _guard = state.refresh_gate.lock().await;
        assert!(refresh_accounts(state.clone(), vec!["nonexistent".into()])
            .await
            .unwrap_err()
            .contains("正在运行"));
        assert!(!state.runtime.lock().unwrap().account_refresh.running);
    }
    #[test]
    fn enabling_needs_confirmation_and_persists_without_network() {
        let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let state = crate::pool::AppState::new(path.clone()).unwrap();
        let mut s = Settings::default();
        s.account_refresh_enabled = true;
        assert!(state.set_settings(s.clone(), false).is_err());
        assert!(!path.exists());
        state.set_settings(s.clone(), true).unwrap();
        let loaded = crate::pool::AppState::new(path).unwrap();
        assert_eq!(loaded.store.lock().unwrap().settings, s);
        assert_eq!(loaded.runtime.lock().unwrap().account_refresh.next_at, None);
        s.account_refresh_enabled = false;
        loaded.set_settings(s, false).unwrap();
        assert!(!loaded.snapshot()["settings"]["accountRefreshEnabled"]
            .as_bool()
            .unwrap());
    }
}
