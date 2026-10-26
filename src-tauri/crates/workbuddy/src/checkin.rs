//! Switch's daily window/recovery policy, adapted to our isolated account store.
use chrono::{Datelike, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub start: String,
    pub end: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            start: "09:00".into(),
            end: "24:00".into(),
        }
    }
}
fn minute(s: &str) -> Option<u32> {
    if s.len() != 5 || s.as_bytes()[2] != b':' {
        return None;
    }
    let h = s.get(..2)?.parse::<u32>().ok()?;
    let m = s.get(3..)?.parse::<u32>().ok()?;
    (h < 24 && m < 60 || h == 24 && m == 0).then_some(h * 60 + m)
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.start.is_empty() && self.end.is_empty() {
            return Ok(());
        }
        match (minute(&self.start), minute(&self.end)) {
            (Some(a), Some(b)) if a < b => Ok(()),
            _ => Err(
                "请选择有效的签到时间段（HH:MM，结束晚于开始，不支持跨午夜）；或同时留空".into(),
            ),
        }
    }
    pub fn due(&self, id: &str, now: chrono::DateTime<chrono::FixedOffset>) -> bool {
        if self.validate().is_err() {
            return false;
        }
        let (Some(start), Some(end)) = (minute(&self.start), minute(&self.end)) else {
            return true;
        };
        let current = now.hour() * 60 + now.minute();
        // Same day/account yields the same slot after restart, like Switch.
        let seed = format!("{}:{}:{id}", now.year(), now.ordinal());
        let hash = seed.bytes().fold(2166136261u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(16777619)
        });
        let target = start + (hash % u64::from(end - start)) as u32;
        current >= target && current < end
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub at: i64,
    pub account_id: String,
    pub submit: bool,
    pub status: Option<Value>,
    pub error: Option<String>,
    /// manual / startup / scheduled / launchd; absent in older records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountState {
    pub status: Option<Value>,
    pub updated_at: Option<i64>,
    pub error: Option<String>,
    pub error_at: Option<i64>,
}
impl AccountState {
    pub fn observe(&mut self, record: &Record) {
        if let Some(status) = &record.status {
            self.status = Some(status.clone());
            self.updated_at = Some(record.at);
        }
        self.error = record.error.clone();
        self.error_at = record.error.as_ref().map(|_| record.at);
    }
}
pub fn auto_allowed(
    store: &crate::pool::Store,
    id: &str,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> bool {
    store.auto_checkin
        && store
            .accounts
            .iter()
            .any(|a| a.id == id && a.variant == "cn" && a.checkin_enabled)
        && store.checkin_config.due(id, now)
}
#[derive(Default)]
pub struct Attempt {
    pub day: String,
    pub at: i64,
    pub failures: u32,
}

pub struct RunGuard {
    state: crate::pool::Shared,
    id: String,
}
impl RunGuard {
    pub fn acquire(state: crate::pool::Shared, id: &str) -> Result<Self, String> {
        if !state
            .runtime
            .lock()
            .unwrap()
            .checkin_running
            .insert(id.into())
        {
            return Err("该账号正在查询或签到，请稍后重试".into());
        }
        Ok(Self {
            state,
            id: id.into(),
        })
    }
}
impl Drop for RunGuard {
    fn drop(&mut self) {
        self.state
            .runtime
            .lock()
            .unwrap()
            .checkin_running
            .remove(&self.id);
    }
}
pub fn retry_due(attempt: Option<&Attempt>, day: &str, now: i64) -> bool {
    let Some(a) = attempt.filter(|a| a.day == day) else {
        return true;
    };
    // Switch recovery: bounded fast retries, then a slower verification cycle.
    let delay = if a.failures > 0 && a.failures <= 12 {
        5
    } else {
        30
    };
    now - a.at >= delay * 60_000
}
pub fn normalize(data: &Value) -> Result<Value, String> {
    if !data.is_object() {
        return Err("签到状态数据异常，未执行签到".into());
    }
    // Persist/display only known billing fields, never arbitrary upstream data.
    let mut out = serde_json::Map::new();
    let checked = data
        .get("today_checked_in")
        .or_else(|| data.get("todayCheckedIn"))
        .and_then(Value::as_bool)
        .ok_or("无法判定签到状态，未执行签到")?;
    out.insert("today_checked_in".into(), checked.into());
    if let Some(active) = data.get("active").filter(|v| !v.is_null()) {
        if !active.is_boolean() {
            return Err("签到活动状态格式异常，未执行签到".into());
        }
        out.insert("active".into(), active.clone());
    }
    for (snake, camel) in [
        ("streak_days", "streakDays"),
        ("today_credit", "todayCredit"),
        ("daily_credit", "dailyCredit"),
    ] {
        if let Some(v) = data
            .get(snake)
            .or_else(|| data.get(camel))
            .filter(|v| !v.is_null())
        {
            let n = v
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or("签到数值格式异常")?;
            if snake == "streak_days" && n.fract() != 0.0 {
                return Err("签到天数格式异常".into());
            }
            out.insert(snake.into(), v.clone());
        }
    }
    Ok(Value::Object(out))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn cached_state_survives_log_eviction_and_error_preserves_last_success() {
        let mut state = AccountState::default();
        let mut log = vec![Record {
            at: 1,
            account_id: "a".into(),
            submit: true,
            status: Some(json!({"today_checked_in":true})),
            error: None,
            trigger: None,
        }];
        state.observe(&log[0]);
        log.clear();
        let restored: AccountState =
            serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        assert_eq!(restored.status.unwrap()["today_checked_in"], true);
        state.observe(&Record {
            at: 2,
            account_id: "a".into(),
            submit: false,
            status: None,
            error: Some("offline".into()),
            trigger: None,
        });
        assert_eq!(state.updated_at, Some(1));
        assert_eq!(state.error_at, Some(2));
    }
    #[test]
    fn strict_numeric_fields_and_minimal_persisted_data() {
        for data in [
            json!({"today_checked_in":false,"today_credit":"10"}),
            json!({"today_checked_in":false,"today_credit":-1}),
            json!({"today_checked_in":false,"active":"false"}),
            json!({"today_checked_in":false,"streak_days":1.5}),
        ] {
            assert!(normalize(&data).is_err());
        }
        let data = normalize(
            &json!({"today_checked_in":true,"today_credit":10,"access_token":"not-persisted"}),
        )
        .unwrap();
        assert!(data.get("access_token").is_none());
    }
    #[test]
    fn disabling_auto_or_changing_window_cancels_submission_permission() {
        let mut store = crate::pool::Store::default();
        store.accounts.push(crate::pool::Account {
            id: "a".into(),
            name: "a".into(),
            source: "switch".into(),
            source_id: "a".into(),
            variant: "cn".into(),
            enabled: false,
            checkin_enabled: true,
            note: String::new(),
            display_field: String::new(),
            expires_at: None,
        });
        store.checkin_config = Config {
            start: String::new(),
            end: String::new(),
        };
        store.auto_checkin = true;
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-21T12:00:00+08:00").unwrap();
        assert!(auto_allowed(&store, "a", now));
        store.auto_checkin = false;
        assert!(!auto_allowed(&store, "a", now));
        store.auto_checkin = true;
        store.checkin_config = Config {
            start: "13:00".into(),
            end: "14:00".into(),
        };
        assert!(!auto_allowed(&store, "a", now));
    }
    #[test]
    fn windows_validate() {
        for (start, end, valid) in [
            ("09:00", "24:00", true),
            ("", "", true),
            ("", "10:00", false),
            ("23:00", "01:00", false),
            ("09:00", "09:00", false),
            ("aa:00", "10:00", false),
        ] {
            assert_eq!(
                Config {
                    start: start.into(),
                    end: end.into()
                }
                .validate()
                .is_ok(),
                valid
            );
        }
    }
    #[test]
    fn slots_and_boundaries() {
        let c = Config {
            start: "09:00".into(),
            end: "10:00".into(),
        };
        let parse = |s| chrono::DateTime::parse_from_rfc3339(s).unwrap();
        assert!(!c.due("a", parse("2026-10-21T08:59:00+08:00")));
        assert!(c.due("a", parse("2026-10-21T09:59:00+08:00")));
        assert!(!c.due("a", parse("2026-10-21T10:00:00+08:00")));
    }
    #[test]
    fn recovery_is_bounded_and_resets_daily() {
        let mut a = Attempt {
            day: "today".into(),
            at: 0,
            failures: 1,
        };
        assert!(!retry_due(Some(&a), "today", 299999));
        assert!(retry_due(Some(&a), "today", 300000));
        a.failures = 13;
        assert!(!retry_due(Some(&a), "today", 300000));
        assert!(retry_due(Some(&a), "tomorrow", 0));
    }
    #[test]
    fn never_guess_unchecked_from_missing_status() {
        assert!(normalize(&json!({"active":true})).is_err());
        let s = normalize(&json!({"todayCheckedIn":true,"streakDays":4,"todayCredit":10})).unwrap();
        assert_eq!(s["today_checked_in"], true);
        assert_eq!(s["streak_days"], 4);
    }
}
