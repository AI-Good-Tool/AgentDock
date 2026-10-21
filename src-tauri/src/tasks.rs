//! Background schedulers carried over from the three applications. Each one
//! keeps its original opt-in rules; nothing new runs without the user's switch.
use crate::app::Shared;
use serde_json::{json, Value};
use std::time::Duration;

pub async fn spawn_all(hub: Shared) {
    // Debugging aid: inspect the UI without running any scheduled account action.
    if std::env::var_os("AGENTDOCK_NO_TASKS").is_some() {
        return;
    }
    // WorkBuddy: daily check-in window, scheduled credit refresh, local client monitor.
    tokio::spawn(workbuddy::proxy::run_checkin_scheduler(hub.wb.clone()));
    tokio::spawn(workbuddy::settings::run_scheduler(hub.wb.clone()));
    tokio::spawn(workbuddy::switch_bridge::run_local_monitor(hub.wb.clone()));
    // Qoder: daily credits auto-claim (checks every 10 minutes, first after 30 s).
    let q = hub.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            qoder_auto_claim(&q).await;
            tokio::time::sleep(Duration::from_secs(600)).await;
        }
    });
    // Trae: startup catch-up, then every 10 minutes for opted-in CN accounts.
    let t = hub.trae.clone();
    tokio::spawn(async move {
        loop {
            t.auto_checkin_tick().await;
            tokio::time::sleep(Duration::from_secs(600)).await;
        }
    });
    if hub.settings().auto_start {
        let _ = crate::server::start(hub.clone()).await;
    }
}

/// Claim only when a campaign is claimable (or a token needs refreshing).
pub async fn qoder_auto_claim(hub: &Shared) {
    let results = tokio::task::spawn_blocking(|| {
        use qoder::qcore as qc;
        if !qc::load_settings()["autoClaim"].as_bool().unwrap_or(true) {
            return None;
        }
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
    if let Some(results) = results {
        *hub.qoder_scheduler.lock().unwrap() = json!({
            "lastRun": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            "lastResults": results,
        });
    }
}

/// `--claim`: one unattended pass for launchd (Qoder campaigns, Trae and
/// WorkBuddy opted-in check-ins). Returns printable results.
pub async fn claim_once(hub: &Shared) -> Value {
    let qoder = tokio::task::spawn_blocking(|| qoder::qcore::claim_all(None, false))
        .await
        .unwrap_or_default();
    hub.trae.auto_checkin_tick().await;
    let mut workbuddy = vec![];
    let ids: Vec<String> = {
        let store = hub.wb.store.lock().unwrap();
        if store.auto_checkin {
            store
                .accounts
                .iter()
                .filter(|a| a.checkin_enabled && a.variant == "cn")
                .map(|a| a.id.clone())
                .collect()
        } else {
            vec![]
        }
    };
    for id in ids {
        let r = workbuddy::proxy::checkin(hub.wb.clone(), id.clone(), true).await;
        workbuddy.push(json!({"id":id,"ok":r.is_ok(),"error":r.err()}));
    }
    json!({"qoder":qoder,"workbuddy":workbuddy,"trae":"checked opted-in accounts"})
}
