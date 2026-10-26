//! WorkBuddy management operations: catalog sync, daily check-in and its
//! scheduler. Chat traffic is served by the unified gateway via `upstream`.
use crate::pool::{self, Account, Credential, Shared};
use serde_json::{json, Value};
use std::time::Duration;

pub fn official_request(
    state: &Shared,
    a: &Account,
    c: &Credential,
    route: &str,
) -> reqwest::RequestBuilder {
    crate::official::request(state, a, c, route)
}

pub async fn sync_models(state: Shared, id: Option<String>) -> Result<usize, String> {
    if let Some(id) = &id {
        let account = state.store.lock().unwrap().accounts.iter().find(|a| &a.id == id).cloned();
        if let Some(account) = account { let _ = crate::auth::ensure(&state, &account, None).await; }
    }
    let (account, mut lease) = state.acquire("catalog", id.as_deref())?;
    let _ = crate::auth::ensure(&state, &account, None).await;
    let cred = pool::credentials(&account)?;
    let req = official_request(&state, &account, &cred, "/v3/config")
        .build()
        .map_err(|_| "无法构造目录请求")?;
    let mut req = req;
    *req.method_mut() = reqwest::Method::GET;
    let response = state
        .client
        .execute(req)
        .await
        .map_err(|_| "目录请求连接失败")?;
    lease.status = response.status().as_u16();
    let data = response
        .json::<Value>()
        .await
        .map_err(|_| "目录响应不是 JSON")?;
    if ![json!(0), json!(200)].contains(&data["code"]) {
        lease.outcome = "rejected".into();
        return Err("上游拒绝了模型目录请求".into());
    }
    let models = data["data"]["models"]
        .as_array()
        .ok_or("目录中没有 models 数组")?
        .clone();
    if models.is_empty() {
        return Err("拒绝用空目录覆盖已有模型".into());
    }
    let n = models.len();
    // Keep public routing metadata only. Credentials and config prompts never enter the store.
    let allowed = [
        "id",
        "name",
        "modelId",
        "credits",
        "vendor",
        "tags",
        "supportsImages",
        "supportsToolCall",
        "supportsReasoning",
        "onlyReasoning",
        "maxInputTokens",
        "maxOutputTokens",
        "disabled",
        "disabledReason",
        "url",
        "useCustomProtocol",
        "temperature",
        "reasoning",
        "relatedModels",
    ];
    let models = models
        .into_iter()
        .filter_map(|m| {
            m.as_object().map(|o| {
                Value::Object(
                    o.iter()
                        .filter(|(k, _)| allowed.contains(&k.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                )
            })
        })
        .collect();
    let active = data["data"]["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["name"] == "cli")
        .flat_map(|a| a["models"].as_array().into_iter().flatten())
        .filter_map(|v| v.as_str().map(String::from))
        .collect::<Vec<_>>();
    if active.is_empty() {
        return Err("目录缺少 cli agent 的模型权限列表，未覆盖已有路由".into());
    }
    state.update_models(models, active, account.variant)?;
    lease.outcome = "completed".into();
    Ok(n)
}

pub async fn checkin(state: Shared, id: String, submit: bool) -> Result<Value, String> {
    checkin_run(state, id, submit, false, "manual").await
}
/// A pass of the unified check-in plan; `trigger` is kept in the record.
pub async fn checkin_by(state: Shared, id: String, submit: bool, trigger: &str) -> Result<Value, String> {
    checkin_run(state, id, submit, false, trigger).await
}
async fn checkin_run(
    state: Shared,
    id: String,
    submit: bool,
    automatic: bool,
    trigger: &str,
) -> Result<Value, String> {
    let _guard = crate::checkin::RunGuard::acquire(state.clone(), &id)?;
    let result = checkin_inner(state.clone(), id.clone(), submit, automatic).await;
    let now = chrono::Utc::now().timestamp_millis();
    let record = crate::checkin::Record {
        at: now,
        account_id: id.clone(),
        submit,
        status: result.as_ref().ok().cloned(),
        error: result.as_ref().err().cloned(),
        trigger: Some(trigger.to_owned()),
    };
    let saved = state.clone();
    let record_id = id.clone();
    tokio::task::spawn_blocking(move || {
        saved.update_store(|store| {
            let cutoff = now - 30 * 24 * 60 * 60 * 1000;
            store.checkin_records.retain(|r| r.at >= cutoff);
            // Account state must survive independent of the bounded operation history.
            store
                .checkin_states
                .entry(record_id)
                .or_default()
                .observe(&record);
            store.checkin_records.push(record);
            if store.checkin_records.len() > 1000 {
                store
                    .checkin_records
                    .drain(..store.checkin_records.len() - 1000);
            }
            Ok(())
        })
    })
    .await
    .map_err(|_| "保存签到记录任务失败")??;
    {
        let mut rt = state.runtime.lock().unwrap();
        match &result {
            Ok(_) => {
                rt.checkin_times.insert(id.clone(), now);
                rt.checkin_errors.remove(&id);
            }
            Err(message) => {
                rt.checkin_errors.insert(id, message.clone());
            }
        }
    }
    result
}
async fn query_checkin_status(
    state: &Shared,
    account: &pool::Account,
    credential: &pool::Credential,
) -> Result<Value, String> {
    for path in [
        "/v2/billing/meter/checkin-activity-status",
        "/v2/billing/meter/checkin-status",
    ] {
        let response = official_request(state, account, credential, path)
            .header("X-Client-Platform", "web")
            .header("Origin", pool::endpoint(account))
            .header(
                "Referer",
                format!("{}/profile/plans-usage", pool::endpoint(account)),
            )
            .json(&json!({}))
            .send()
            .await
            .map_err(|_| "签到状态连接失败")?;
        // Only route absence warrants a legacy fallback. Never hide auth/rate-limit failures.
        if response.status() == reqwest::StatusCode::NOT_FOUND
            && path.ends_with("checkin-activity-status")
        {
            continue;
        }
        if !response.status().is_success() {
            return Err("查询签到状态被上游拒绝，未执行签到".into());
        }
        let result: Value = response.json().await.map_err(|_| "签到状态不是 JSON")?;
        if ![json!(0), json!(200)].contains(&result["code"]) {
            return Err("查询签到状态失败，未执行签到".into());
        }
        return Ok(result);
    }
    Err("官方签到状态接口不可用".into())
}
async fn checkin_inner(
    state: Shared,
    id: String,
    submit: bool,
    automatic: bool,
) -> Result<Value, String> {
    let account = state.store.lock().unwrap().accounts.iter().find(|a| a.id == id).cloned();
    if let Some(account) = account { let _ = crate::auth::ensure(&state, &account, None).await; }
    let (a, mut lease) = state.acquire("checkin", Some(&id))?;
    if submit && !a.checkin_enabled {
        return Err("此账号签到开关已关闭；可单独查询状态".into());
    }
    if a.variant == "ai" {
        return Err("国际版暂不支持此签到接口".into());
    }
    if automatic
        && !crate::checkin::auto_allowed(
            &state.store.lock().unwrap(),
            &id,
            chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap()),
        )
    {
        return Err("自动签到已关闭或不在执行窗口内，未提交签到".into());
    }
    let cred = pool::credentials(&a)?;
    let mut result = query_checkin_status(&state, &a, &cred).await?;
    lease.status = 200;
    let normalized = crate::checkin::normalize(&result["data"])?;
    let checked = normalized["today_checked_in"] == true;
    if submit && normalized["active"] == false && !checked {
        return Err("官方签到活动未开放，未执行签到".into());
    }
    if submit && !checked {
        if !state
            .store
            .lock()
            .unwrap()
            .accounts
            .iter()
            .any(|current| current.id == id && current.checkin_enabled)
        {
            return Err("此账号签到开关已关闭，未执行签到".into());
        }
        if automatic
            && !crate::checkin::auto_allowed(
                &state.store.lock().unwrap(),
                &id,
                chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap()),
            )
        {
            return Err("自动签到已关闭或不在执行窗口内，未提交签到".into());
        }
        let res = official_request(&state, &a, &cred, "/v2/billing/meter/daily-checkin")
            .header("X-Client-Platform", "web")
            .header("Origin", pool::endpoint(&a))
            .json(&json!({}))
            .send()
            .await
            .map_err(|_| "签到连接失败，请先重新查询状态")?;
        lease.status = res.status().as_u16();
        result = res.json().await.map_err(|_| "签到响应不是 JSON")?;
        if ![json!(0), json!(200)].contains(&result["code"]) {
            let msg = result["message"]
                .as_str()
                .or_else(|| result["msg"].as_str())
                .unwrap_or("");
            if !msg.contains("已签到") && !msg.to_lowercase().contains("repeat") {
                return Err("签到被上游拒绝，请查询活动资格".into());
            }
        }
        // Query again to display authoritative post-checkin status rather than guess its schema.
        result = query_checkin_status(&state, &a, &cred)
            .await
            .map_err(|_| "签到已提交，但复查失败；请先查询状态，不要重复提交")?;
    }
    if ![json!(0), json!(200)].contains(&result["code"]) {
        return Err("签到已提交，但复查失败；请先查询状态，不要重复提交".into());
    }
    let status = crate::checkin::normalize(&result["data"])?;
    if submit && status["today_checked_in"] != true {
        return Err("签到提交后尚未确认成功，请重新查询官方状态".into());
    }
    state
        .runtime
        .lock()
        .unwrap()
        .checkins
        .insert(id, status.clone());
    lease.outcome = "completed".into();
    Ok(status)
}

/// Opt-in daily job while the process is open. Submission is always guarded by
/// the official today_checked_in response, even if the app restarts mid-day.
pub async fn run_checkin_scheduler(state: Shared) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        let now =
            chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap());
        let accounts = {
            let store = state.store.lock().unwrap();
            if !store.auto_checkin {
                continue;
            }
            store
                .accounts
                .iter()
                .filter(|a| {
                    a.checkin_enabled && a.variant == "cn" && store.checkin_config.due(&a.id, now)
                })
                .map(|a| a.id.clone())
                .collect::<Vec<_>>()
        };
        for id in accounts {
            let now =
                chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap());
            let day = now.format("%Y-%m-%d").to_string();
            // Settings may change during a batch: re-check immediately before each request.
            {
                let store = state.store.lock().unwrap();
                if !store.auto_checkin {
                    break;
                }
                if !store
                    .accounts
                    .iter()
                    .any(|a| a.id == id && a.checkin_enabled && a.variant == "cn")
                    || !store.checkin_config.due(&id, now)
                {
                    continue;
                }
            }
            let at = chrono::Utc::now().timestamp_millis();
            if !crate::checkin::retry_due(
                state.runtime.lock().unwrap().auto_checkin_attempts.get(&id),
                &day,
                at,
            ) {
                continue;
            }
            let result = checkin_run(state.clone(), id.clone(), true, true, "scheduled").await;
            {
                let mut rt = state.runtime.lock().unwrap();
                let attempt = rt.auto_checkin_attempts.entry(id).or_default();
                if attempt.day != day {
                    attempt.failures = 0;
                }
                attempt.day = day.clone();
                attempt.at = at;
                attempt.failures = if result.is_err() {
                    attempt.failures.saturating_add(1)
                } else {
                    0
                };
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}
