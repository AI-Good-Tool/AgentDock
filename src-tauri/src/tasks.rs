//! Background schedulers: the unified check-in schedule for all three
//! platforms plus WorkBuddy's credit refresh and local client monitor.
use crate::app::Shared;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use crate::checkin_schedule::{self, Poll, RunLock};

pub async fn spawn_all(hub: Shared) {
    // Debugging aid: inspect the UI without running any scheduled account action.
    if std::env::var_os("AGENTDOCK_NO_TASKS").is_some() {
        return;
    }
    agentdock_core::applog::info("app", || {
        (format!("AgentDock {} 已启动", env!("CARGO_PKG_VERSION")), Value::Null)
    });
    // WorkBuddy: scheduled credit refresh and local client monitor.
    tokio::spawn(workbuddy::settings::run_scheduler(hub.wb.clone()));
    tokio::spawn(workbuddy::switch_bridge::run_local_monitor(hub.wb.clone()));
    // Unified daily check-in / credit claim for all three platforms.
    tokio::spawn(checkin_loop(hub.clone()));
    if hub.settings().auto_start {
        let _ = crate::server::start(hub.clone()).await;
    }
}

const STARTUP_DELAY: Duration = Duration::from_secs(10);

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The same durable plan is evaluated by the application and system task.
async fn checkin_loop(hub: Shared) {
    let launched = Instant::now();
    let mut startup_pending = true;
    loop {
        let settings = hub.settings();
        let c = &settings.checkin;
        if !c.on_startup {
            startup_pending = false;
        }
        if startup_pending && launched.elapsed() >= STARTUP_DELAY {
            startup_pending = false;
            run_pass(&hub, "startup").await;
            continue;
        }
        let now = chrono::Utc::now();
        let next = match checkin_schedule::poll(&hub.dir, c, &settings.timezone, now) {
            Ok(Poll::Due { lock, next }) => {
                hub.checkin.lock().unwrap()["nextRun"] = json!(next);
                run_locked(&hub, "scheduled", lock).await;
                continue;
            }
            Ok(Poll::Waiting(next)) => next,
            Ok(Poll::Busy) => hub.checkin.lock().unwrap()["nextRun"].as_i64().filter(|next| *next > now.timestamp_millis()),
            Err(error) => {
                agentdock_core::applog::info("checkin", || (error, Value::Null));
                None
            }
        };
        let startup = startup_pending.then(|| now.timestamp_millis() + (STARTUP_DELAY.saturating_sub(launched.elapsed())).as_millis() as i64);
        let next = [startup, next].into_iter().flatten().min();
        hub.checkin.lock().unwrap()["nextRun"] = json!(next);
        // Re-evaluate wall time after sleep/clock changes and external passes.
        let delay = next.map(|next| (next - now.timestamp_millis()).clamp(250, 30_000) as u64).unwrap_or(30_000);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(delay)) => {}
            _ = hub.checkin_wake.notified() => {}
        }
    }
}

/// One check-in pass over every enabled platform. Overlapping passes are skipped.
pub async fn run_pass(hub: &Shared, trigger: &str) -> Value {
    match checkin_schedule::try_run_lock(&hub.dir) {
        Ok(Some(lock)) => run_locked(hub, trigger, lock).await,
        Ok(None) => json!({"skipped":"running"}),
        Err(error) => json!({"ok":false,"message":error}),
    }
}

async fn run_locked(hub: &Shared, trigger: &str, _lock: RunLock) -> Value {
    let c = hub.settings().checkin;
    hub.checkin.lock().unwrap()["running"] = json!(true);
    agentdock_core::applog::info("checkin", || {
        (
            format!("开始签到（{}）", trigger_label(trigger)),
            json!({"workbuddy": c.workbuddy, "qoder": c.qoder, "trae": c.trae}),
        )
    });
    let workbuddy = if c.workbuddy { json!(workbuddy_pass(hub).await) } else { json!("disabled") };
    let qoder = if c.qoder { qoder_pass(hub).await } else { json!("disabled") };
    let trae = if c.trae { json!(hub.trae.auto_checkin_tick().await) } else { json!("disabled") };
    for (source, rows) in [("checkin.workbuddy", &workbuddy), ("checkin.qoder", &qoder), ("checkin.trae", &trae)] {
        for row in rows.as_array().into_iter().flatten() {
            let ok = row["ok"] != false && row["phase"] != "error" && row["phase"] != "login_required";
            agentdock_core::applog::emit(
                if ok { agentdock_core::applog::Level::Info } else { agentdock_core::applog::Level::Warn },
                source,
                || {
                    let detail = row["message"]
                        .as_str()
                        .or(row["msg"].as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| match (row["phase"].as_str(), row["credits"].as_f64()) {
                            (Some(phase), Some(c)) if c > 0.0 => format!("{phase} +{c}"),
                            (Some(phase), _) => phase.to_owned(),
                            _ => String::new(),
                        });
                    (
                        format!("{}：{detail}", row["name"].as_str().unwrap_or("账号")),
                        row.clone(),
                    )
                },
            );
        }
    }
    let result = json!({
        "trigger": trigger,
        "lastRun": now_ms(),
        "results": {"workbuddy": workbuddy, "qoder": qoder, "trae": trae},
    });
    {
        let mut status = hub.checkin.lock().unwrap();
        let next = status["nextRun"].clone();
        *status = result.clone();
        status["nextRun"] = next;
        status["running"] = json!(false);
    }
    result
}

fn trigger_label(trigger: &str) -> &'static str {
    match trigger {
        "startup" => "启动后自动执行",
        "scheduled" => "定时执行",
        "launchd" => "系统定时任务",
        _ => "手动执行",
    }
}

/// Opted-in WorkBuddy CN accounts whose official status is not yet "checked
/// in today" (Shanghai day); each check queries status before submitting.
async fn workbuddy_pass(hub: &Shared) -> Vec<Value> {
    let shanghai = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    let today = chrono::Utc::now().with_timezone(&shanghai).date_naive();
    let due: Vec<(String, String)> = {
        let store = hub.wb.store.lock().unwrap();
        store
            .accounts
            .iter()
            .filter(|a| a.checkin_enabled && a.variant == "cn")
            .filter(|a| {
                let state = store.checkin_states.get(&a.id);
                let done = state.is_some_and(|s| {
                    s.status.as_ref().is_some_and(|v| v["today_checked_in"] == true)
                        && s.updated_at
                            .and_then(chrono::DateTime::from_timestamp_millis)
                            .is_some_and(|t| t.with_timezone(&shanghai).date_naive() == today)
                });
                !done
            })
            .map(|a| (a.id.clone(), if a.note.is_empty() { a.name.clone() } else { a.note.clone() }))
            .collect()
    };
    let mut out = vec![];
    let mut done = vec![];
    for (id, name) in due {
        let r = workbuddy::proxy::checkin(hub.wb.clone(), id.clone(), true).await;
        if r.is_ok() {
            done.push(id.clone());
        }
        out.push(match r {
            Ok(_) => json!({"id": id, "name": name, "ok": true, "message": "签到状态已确认"}),
            Err(e) => json!({"id": id, "name": name, "ok": false, "message": e}),
        });
    }
    if !done.is_empty() {
        // Rewards land in the credit balance shown by the pool and check-in pages.
        let _ = workbuddy::settings::refresh_accounts(hub.wb.clone(), done).await;
    }
    out
}

async fn qoder_pass(hub: &Shared) -> Value {
    match qoder_auto_claim(hub).await {
        Some(results) => json!(results),
        None => json!([]),
    }
}

/// Claim only when a campaign is claimable (or a token needs refreshing).
pub async fn qoder_auto_claim(hub: &Shared) -> Option<Vec<Value>> {
    let results = tokio::task::spawn_blocking(|| {
        use qoder::qcore as qc;
        let accounts = qc::load_accounts();
        let need = accounts.iter().filter(|a| !a.token.is_empty()).any(|acc| {
            let (status, data) = qc::get_campaigns(acc);
            (status == 200 && !qc::claimable_campaigns(&data).is_empty())
                || (status == 401 && !acc.refreshToken.is_empty())
        });
        need.then(|| qc::claim_all(None, false))
    })
    .await
    .ok()
    .flatten();
    if let Some(results) = &results {
        *hub.qoder_scheduler.lock().unwrap() = json!({
            "lastRun": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            "lastResults": results,
        });
    }
    results
}

/// The system scheduler wakes once per minute; it executes only due slots.
pub async fn claim_due(hub: &Shared) -> Result<Value, String> {
    let settings = hub.settings();
    match checkin_schedule::poll(&hub.dir, &settings.checkin, &settings.timezone, chrono::Utc::now())? {
        Poll::Due { lock, next } => {
            hub.checkin.lock().unwrap()["nextRun"] = json!(next);
            Ok(run_locked(hub, "launchd", lock).await)
        }
        Poll::Waiting(next) => Ok(json!({"skipped":"not_due","nextRun":next})),
        Poll::Busy => Ok(json!({"skipped":"running"})),
    }
}
