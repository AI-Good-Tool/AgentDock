//! One calendar/interval plan shared by the desktop loop and system task.
//! A durable cursor and an OS file lock prevent duplicate scheduled passes
//! when both processes are alive, including after application restarts.
use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::ErrorKind,
    path::Path,
};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Minutes,
    Hours,
    Days,
    #[default]
    Daily,
    Date,
    Weekly,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Schedule {
    pub kind: Kind,
    pub every: u32,
    pub time: String,
    /// ISO weekdays: Monday = 1, Sunday = 7.
    pub weekdays: Vec<u32>,
    /// A single date in the selected time zone, executed at `time` once.
    pub date: String,
}
impl Default for Schedule {
    fn default() -> Self {
        Self {
            kind: Kind::Daily,
            every: 1,
            time: "07:00".into(),
            weekdays: vec![1, 2, 3, 4, 5],
            date: String::new(),
        }
    }
}

pub fn validate_zone(zone: &str) -> Result<(), String> {
    if zone == "system" || zone.parse::<chrono_tz::Tz>().is_ok() {
        Ok(())
    } else {
        Err("签到时区无效，请在偏好设置中选择有效时区".into())
    }
}

impl Schedule {
    fn clock(&self) -> Option<NaiveTime> {
        let time = NaiveTime::parse_from_str(&self.time, "%H:%M").ok()?;
        (time.format("%H:%M").to_string() == self.time).then_some(time)
    }

    fn calendar_date(&self) -> Option<NaiveDate> {
        let date = NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").ok()?;
        (date.format("%Y-%m-%d").to_string() == self.date).then_some(date)
    }

    fn interval(&self) -> Option<Duration> {
        match self.kind {
            Kind::Minutes => Some(Duration::minutes(self.every.into())),
            Kind::Hours => Some(Duration::hours(self.every.into())),
            Kind::Days => Some(Duration::days(self.every.into())),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let max = match self.kind {
            Kind::Minutes => 1440,
            Kind::Hours => 8760,
            Kind::Days => 365,
            _ => 0,
        };
        if max > 0 {
            if self.every == 0 || self.every > max {
                return Err(format!("签到间隔需为 1–{max}"));
            }
            return Ok(());
        }
        if self.clock().is_none() {
            return Err("签到时间需为 HH:mm（00:00–23:59）".into());
        }
        if self.kind == Kind::Weekly
            && (self.weekdays.is_empty() || self.weekdays.iter().any(|day| !(1..=7).contains(day)))
        {
            return Err("请至少选择一个有效星期（周一至周日）".into());
        }
        if self.kind == Kind::Date && self.calendar_date().is_none() {
            return Err("请选择有效的签到日期（YYYY-MM-DD）".into());
        }
        Ok(())
    }

    fn in_zone<T: TimeZone>(
        &self,
        now: DateTime<Utc>,
        zone: &T,
        inclusive: bool,
    ) -> Option<DateTime<Utc>> {
        let clock = self.clock()?;
        let today = now.with_timezone(zone).date_naive();
        for offset in 0..=7 {
            let date = if self.kind == Kind::Date {
                self.calendar_date()?
            } else {
                today.checked_add_signed(Duration::days(offset))?
            };
            if self.kind == Kind::Weekly
                && !self.weekdays.contains(&date.weekday().number_from_monday())
            {
                continue;
            }
            // An ambiguous wall time runs once at its first occurrence. Skip
            // nonexistent DST times rather than silently moving the chosen time.
            let candidate = match zone.from_local_datetime(&date.and_time(clock)) {
                LocalResult::Single(time) => Some(time.with_timezone(&Utc)),
                LocalResult::Ambiguous(a, b) => Some(a.min(b).with_timezone(&Utc)),
                LocalResult::None => None,
            };
            if let Some(time) = candidate.filter(|time| *time > now || (inclusive && *time == now))
            {
                return Some(time);
            }
            if self.kind == Kind::Date {
                return None;
            }
        }
        None
    }

    pub fn next(&self, now: DateTime<Utc>, zone: &str, inclusive: bool) -> Option<DateTime<Utc>> {
        if let Some(interval) = self.interval() {
            return now.checked_add_signed(interval);
        }
        if zone == "system" {
            self.in_zone(now, &Local, inclusive)
        } else {
            self.in_zone(now, &zone.parse::<chrono_tz::Tz>().ok()?, inclusive)
        }
    }
}

/// Held across a pass. Closing the file also releases the lock after a crash.
#[derive(Debug)]
pub struct RunLock {
    _file: File,
}

pub fn try_run_lock(dir: &Path) -> Result<Option<RunLock>, String> {
    std::fs::create_dir_all(dir).map_err(|_| "无法创建签到状态目录")?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(dir.join("checkin-run.lock"))
        .map_err(|_| "无法打开签到运行锁")?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(RunLock { _file: file })),
        Err(error)
            if error.kind() == ErrorKind::WouldBlock
                || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
        {
            Ok(None)
        }
        Err(_) => Err("无法锁定签到任务".into()),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cursor {
    key: String,
    next_run: Option<i64>,
}

pub enum Poll {
    Busy,
    Waiting(Option<i64>),
    Due { lock: RunLock, next: Option<i64> },
}

/// Reserve one due slot before starting work. Missed repeating slots coalesce
/// into one pass, and completed one-off dates remain consumed across restarts.
pub fn poll(
    dir: &Path,
    settings: &crate::settings::CheckinSettings,
    zone: &str,
    now: DateTime<Utc>,
) -> Result<Poll, String> {
    if !settings.scheduled {
        return Ok(Poll::Waiting(None));
    }
    let Some(lock) = try_run_lock(dir)? else {
        return Ok(Poll::Busy);
    };
    let plan = settings.schedule();
    plan.validate()?;
    validate_zone(zone)?;
    let system_offset = (zone == "system" && plan.interval().is_none())
        .then(|| now.with_timezone(&Local).offset().local_minus_utc());
    let key =
        serde_json::to_string(&(&plan, zone, system_offset)).map_err(|_| "签到计划编码失败")?;
    let path = dir.join("checkin-schedule.json");
    let saved: Option<Cursor> = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let mut dirty = false;
    let mut cursor = match saved.filter(|cursor| cursor.key == key) {
        Some(cursor) => cursor,
        None => {
            dirty = true;
            Cursor {
                key,
                next_run: plan
                    .next(now, zone, true)
                    .map(|time| time.timestamp_millis()),
            }
        }
    };
    let due = cursor
        .next_run
        .is_some_and(|time| time <= now.timestamp_millis());
    if due {
        dirty = true;
        cursor.next_run = if plan.kind == Kind::Date {
            None
        } else {
            plan.next(now, zone, false)
                .map(|time| time.timestamp_millis())
        };
    }
    if dirty {
        crate::app::write_private(
            &path,
            &serde_json::to_vec(&cursor).map_err(|_| "签到状态编码失败")?,
        )?;
    }
    if due {
        Ok(Poll::Due {
            lock,
            next: cursor.next_run,
        })
    } else {
        Ok(Poll::Waiting(cursor.next_run))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn time(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn defaults_and_all_schedule_modes() {
        let now = time("2026-10-22T20:00:00Z");
        let mut plan = Schedule::default();
        assert_eq!(
            plan.next(now, "Asia/Shanghai", false),
            Some(time("2026-10-22T23:00:00Z"))
        );
        for (kind, seconds) in [
            (Kind::Minutes, 120),
            (Kind::Hours, 7200),
            (Kind::Days, 172800),
        ] {
            plan.kind = kind;
            plan.every = 2;
            assert_eq!(
                plan.next(now, "UTC", false),
                Some(now + Duration::seconds(seconds))
            );
        }
        plan.kind = Kind::Weekly;
        plan.weekdays = vec![1];
        assert_eq!(
            plan.next(now, "Asia/Shanghai", false),
            Some(time("2026-10-25T23:00:00Z"))
        );
        plan.kind = Kind::Date;
        plan.date = "2026-10-25".into();
        assert_eq!(
            plan.next(now, "Asia/Shanghai", false),
            Some(time("2026-10-24T23:00:00Z"))
        );
        assert_eq!(
            plan.next(time("2026-10-26T00:00:00Z"), "Asia/Shanghai", false),
            None
        );
    }

    #[test]
    fn calendar_times_follow_dst_and_run_once_on_ambiguous_days() {
        let mut plan = Schedule::default();
        assert_eq!(
            plan.next(time("2026-03-07T16:00:00Z"), "America/Los_Angeles", false),
            Some(time("2026-03-08T14:00:00Z"))
        );
        plan.time = "01:30".into();
        assert_eq!(
            plan.next(time("2026-11-01T08:30:00Z"), "America/Los_Angeles", false),
            Some(time("2026-11-02T09:30:00Z"))
        );
        plan.time = "02:30".into();
        assert_eq!(
            plan.next(time("2026-03-08T08:00:00Z"), "America/Los_Angeles", false),
            Some(time("2026-03-09T09:30:00Z"))
        );
    }

    #[test]
    fn shared_cursor_survives_restarts_and_excludes_concurrent_processes() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = crate::settings::CheckinSettings::default();
        settings.schedule = Some(Schedule {
            kind: Kind::Date,
            date: "2026-10-23".into(),
            ..Default::default()
        });
        assert!(matches!(
            poll(dir.path(), &settings, "UTC", time("2026-10-23T06:59:00Z")).unwrap(),
            Poll::Waiting(Some(_))
        ));
        let lock = match poll(dir.path(), &settings, "UTC", time("2026-10-23T07:00:00Z")).unwrap() {
            Poll::Due { lock, next: None } => lock,
            _ => panic!("expected one due slot"),
        };
        assert!(matches!(
            poll(dir.path(), &settings, "UTC", time("2026-10-23T07:00:00Z")).unwrap(),
            Poll::Busy
        ));
        drop(lock);
        assert!(matches!(
            poll(dir.path(), &settings, "UTC", time("2026-10-24T08:00:00Z")).unwrap(),
            Poll::Waiting(None)
        ));
        settings.schedule = Some(Schedule {
            kind: Kind::Minutes,
            every: 5,
            ..Default::default()
        });
        let now = time("2026-10-24T08:00:00Z");
        assert!(matches!(
            poll(dir.path(), &settings, "UTC", now).unwrap(),
            Poll::Waiting(Some(_))
        ));
        let future = now + Duration::hours(2);
        match poll(dir.path(), &settings, "UTC", future).unwrap() {
            Poll::Due { next, .. } => assert_eq!(
                next,
                Some((future + Duration::minutes(5)).timestamp_millis())
            ),
            _ => panic!("missed intervals should coalesce into one pass"),
        }
    }

    #[test]
    fn invalid_calendar_and_interval_settings_are_rejected() {
        for plan in [
            Schedule {
                kind: Kind::Minutes,
                every: 0,
                ..Default::default()
            },
            Schedule {
                time: "24:00".into(),
                ..Default::default()
            },
            Schedule {
                kind: Kind::Weekly,
                weekdays: vec![],
                ..Default::default()
            },
            Schedule {
                kind: Kind::Date,
                date: "2026-02-30".into(),
                ..Default::default()
            },
        ] {
            assert!(plan.validate().is_err());
        }
        assert!(validate_zone("not/a/zone").is_err());
    }
}
