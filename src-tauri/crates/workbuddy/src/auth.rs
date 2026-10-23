//! Official login import and token renewal. Source credentials stay in the Switch
//! library. A per-account process lock serializes rotating refreshes; the vault
//! write lock is held only during commits, which also compare the old tokens.
use crate::pool::{self, Account, Shared};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use wb_switch_core::modules::{account, auth_file, variant::WbVariant};

#[derive(Default)]
pub struct Renewal {
    gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    attempts: Mutex<HashMap<String, (String, i64, bool)>>,
}

pub async fn login_headers(state: &Shared, variant: &str) -> HashMap<String, String> {
    let sdk = state.store.lock().unwrap().settings.use_device_sdk;
    let mut headers = if sdk {
        state.official_transport.login_device_headers(variant).await
    } else {
        HashMap::new()
    };
    headers.insert("X-Custom-Channel-Id".into(), "unknown".into());
    headers.insert("X-Request-Id".into(), uuid::Uuid::new_v4().to_string());
    headers.insert("User-Agent".into(), crate::official::USER_AGENT.into());
    // Host-provided identity only; never manufacture a replacement device ID.
    if let Ok(qimei) = std::env::var("CODEBUDDY_QIMEI36") {
        if !qimei.trim().is_empty() {
            headers.insert(
                "X-Device-Id-List".into(),
                json!({"qimei36":qimei.trim()}).to_string(),
            );
        }
    }
    headers
}

pub fn installed_version(variant: WbVariant) -> Option<String> {
    let app = auth_file::workbuddy_app_path(variant);
    #[cfg(target_os = "macos")]
    let root = app.join("Contents/Resources");
    #[cfg(not(target_os = "macos"))]
    let root = app.parent()?.join("resources");
    let product: Value = serde_json::from_slice(
        &std::fs::read(root.join("app.asar.unpacked/cli/product.json")).ok()?,
    )
    .ok()?;
    product["genieVersion"].as_str().map(str::to_owned)
}

fn raw_account_at(a: &Account, path: &Path) -> Result<Value, String> {
    if a.source != "switch" {
        return Err("请重新导入当前登录账号，以启用自动续期".into());
    }
    pool::read_json(&path.to_path_buf())?
        .as_array()
        .ok_or("账号库格式错误")?
        .iter()
        .find(|r| {
            r["id"] == a.source_id
                && r["uid"].as_str() == a.id.split_once(':').map(|(_, uid)| uid)
                && account::variant_of(r).as_str() == a.variant
        })
        .cloned()
        .ok_or("账号来源已移除".into())
}

fn due(raw: &Value, now: i64) -> bool {
    let last = raw["refreshedAt"]
        .as_i64()
        .or_else(|| {
            raw.pointer("/auth_raw/lastRefreshTime")
                .and_then(Value::as_i64)
        })
        .or_else(|| raw["createdAt"].as_i64())
        .unwrap_or(0);
    const DAY: i64 = 86_400_000;
    if now.saturating_sub(last) >= DAY {
        return true;
    }
    raw["expiresAt"].as_i64().is_some_and(|exp| {
        // For short-lived tokens renew near the end of their lifetime, not on
        // every request. Unknown expiry uses the daily interval above.
        let window = exp.saturating_sub(last).clamp(0, DAY * 10) / 10;
        exp.saturating_sub(now) <= window
    })
}

#[cfg(test)]
mod renewal_regressions {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn renewed_short_and_unknown_expiries_wait_before_next_automatic_refresh() {
        let now = 1_800_000_000_000;
        let raw = json!({"access_token":"old","refresh_token":"old-refresh"});
        for ttl in [Some(3600), None] {
            let fresh = renewed(&raw, &json!({"accessToken":"new","expiresIn":ttl}), now).unwrap();
            assert!(!due(&fresh, now + 1));
            assert!(!due(&fresh, now + 300_000));
            assert!(due(&fresh, now + 86_400_000));
            if ttl.is_some() {
                assert!(!due(&fresh, now + 3_000_000));
                assert!(due(&fresh, now + 3_300_000));
            }
        }
        assert!(due(&raw, now));
        assert!(due(&json!({"refreshedAt":now,"expiresAt":now - 1}), now));
    }

    // Invoked as a child of the test below, with synthetic vaults and loopback
    // HTTP only. Separate OS processes deliberately cannot share Renewal.gates.
    #[tokio::test]
    async fn renewal_process_child() {
        let Some(dir) = std::env::var_os("AGENTDOCK_TEST_RENEWAL_DIR") else { return; };
        let dir = std::path::PathBuf::from(dir);
        let endpoint = std::env::var("AGENTDOCK_TEST_RENEWAL_ENDPOINT").unwrap();
        assert!(endpoint.starts_with("http://127.0.0.1:"));
        let state = pool::AppState::new(dir.join(format!("meta-{}.json", std::process::id()))).unwrap();
        let a: Account = serde_json::from_value(json!({"id":"cn:u","sourceId":"s","source":"switch","variant":"cn","name":"fixture","enabled":true})).unwrap();
        for _ in 0..3 {
            ensure_at(&state, &a, None, &dir.join("accounts.json"), &endpoint).await.unwrap();
        }
    }

    #[tokio::test]
    async fn independent_processes_share_rotation_and_recent_success() {
        for ttl in [Some(3600), None] {
            let dir = std::env::temp_dir().join(format!("agentdock-renew-process-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("accounts.json");
            std::fs::write(&path, serde_json::to_vec(&json!([{"id":"s","uid":"u","variant":"cn","access_token":"old","refresh_token":"shared","expiresAt":1}])).unwrap()).unwrap();
            let count = Arc::new(AtomicUsize::new(0));
            let hits = count.clone();
            let router = axum::Router::new().route("/v2/plugin/auth/token/refresh", axum::routing::post(move |headers: axum::http::HeaderMap| {
                let hits = hits.clone();
                async move {
                    assert_eq!(headers["x-refresh-token"], "shared");
                    hits.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    axum::Json(json!({"code":0,"data":{"accessToken":"fresh","refreshToken":"rotated","expiresIn":ttl}}))
                }
            }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let mut children = vec![];
            for _ in 0..2 {
                children.push(tokio::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "auth::renewal_regressions::renewal_process_child", "--nocapture"])
                    .env("AGENTDOCK_TEST_RENEWAL_DIR", &dir)
                    .env("AGENTDOCK_TEST_RENEWAL_ENDPOINT", &endpoint)
                    .kill_on_drop(true).spawn().unwrap());
            }
            for child in &mut children {
                assert!(tokio::time::timeout(Duration::from_secs(15), child.wait()).await.unwrap().unwrap().success());
            }
            assert_eq!(count.load(Ordering::SeqCst), 1, "one refresh across two processes and six calls, ttl={ttl:?}");
            assert_eq!(pool::read_json(&path).unwrap()[0]["refresh_token"], "rotated");
            server.abort();
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

async fn lock_renewal(path: &Path, id: &str) -> Result<std::fs::File, String> {
    // Hex encoding is stable across processes/platforms and cannot escape the
    // vault directory. Never unlink a lock file: waiters may hold its inode.
    let name: String = id.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
    let path = path.with_extension(format!("renewal-{name}.lock"));
    tokio::task::spawn_blocking(move || {
        let file = std::fs::OpenOptions::new()
            .create(true).truncate(false).read(true).write(true).open(path)
            .map_err(|_| "无法打开账号续期锁")?;
        file.lock().map_err(|_| "无法锁定账号续期任务")?;
        Ok(file)
    }).await.map_err(|_| "账号续期锁任务失败")?
}

fn renewed(raw: &Value, data: &Value, now: i64) -> Result<Value, String> {
    let token = data["accessToken"]
        .as_str()
        .or_else(|| data["access_token"].as_str())
        .filter(|s| !s.is_empty())
        .ok_or("官方续期响应缺少访问令牌")?;
    let mut next = raw.clone();
    next["access_token"] = json!(token);
    for (camel, snake) in [
        ("refreshToken", "refresh_token"),
        ("tokenType", "token_type"),
    ] {
        if let Some(v) = data[camel]
            .as_str()
            .or_else(|| data[snake].as_str())
            .filter(|s| !s.is_empty())
        {
            next[snake] = json!(v);
        }
    }
    for (absolute, relative) in [
        ("expiresAt", "expiresIn"),
        ("refreshExpiresAt", "refreshExpiresIn"),
    ] {
        let exp = wb_switch_core::modules::config::norm_ts(data.get(absolute)).or_else(|| {
            data[relative]
                .as_i64()
                .filter(|s| *s > 0)
                .and_then(|s| s.checked_mul(1000)?.checked_add(now))
        });
        if let Some(exp) = exp {
            next[absolute] = json!(exp);
        } else if absolute == "expiresAt" {
            next.as_object_mut().unwrap().remove(absolute);
        }
    }
    let auth = raw
        .pointer("/auth_raw/auth")
        .unwrap_or(&raw["auth_raw"])
        .clone();
    let mut merged = if auth.is_object() {
        auth
    } else {
        raw["auth_raw"].clone()
    };
    if !merged.is_object() {
        merged = json!({});
    }
    for (k, v) in data.as_object().ok_or("官方续期响应格式错误")? {
        merged[k] = v.clone();
    }
    merged["accessToken"] = next["access_token"].clone();
    merged["refreshToken"] = next["refresh_token"].clone();
    merged["expiresAt"] = next["expiresAt"].clone();
    if next["expiresAt"].is_null() {
        merged.as_object_mut().unwrap().remove("expiresIn");
    }
    merged["refreshExpiresAt"] = next["refreshExpiresAt"].clone();
    merged["lastRefreshTime"] = json!(now);
    next["auth_raw"] = merged;
    next["refreshedAt"] = json!(now);
    next["needs_relogin"] = json!(false);
    next.as_object_mut().unwrap().remove("needs_relogin_reason");
    Ok(next)
}

fn commit_at(path: &Path, previous: &Value, next: &Value) -> Result<bool, String> {
    account::mutate_accounts_at(path, |rows| {
        let Some(current) = rows.iter_mut().find(|r| {
            r["id"] == previous["id"]
                && r["uid"] == previous["uid"]
                && account::variant_of(r) == account::variant_of(previous)
        }) else {
            return Ok(false);
        };
        if current["access_token"] != previous["access_token"]
            || current["refresh_token"] != previous["refresh_token"]
        {
            return Ok(false);
        }
        // Merge only authentication fields; concurrent note/profile edits survive.
        for key in [
            "access_token",
            "refresh_token",
            "token_type",
            "expiresAt",
            "refreshExpiresAt",
            "refreshedAt",
            "auth_raw",
            "needs_relogin",
        ] {
            if let Some(v) = next.get(key) {
                current[key] = v.clone();
            } else if key == "expiresAt" {
                // A renewed token must not inherit the retired token's expiry.
                current.as_object_mut().unwrap().remove(key);
            }
        }
        current
            .as_object_mut()
            .unwrap()
            .remove("needs_relogin_reason");
        Ok(true)
    })
    .map_err(|_| "续期凭据保存失败".into())
}

/// `rejected_token` is set only for a confirmed 401 before any client output.
/// A concurrent successful renewal is reused rather than refreshed again.
pub async fn ensure(
    state: &Shared,
    a: &Account,
    rejected_token: Option<&str>,
) -> Result<bool, String> {
    ensure_at(
        state,
        a,
        rejected_token,
        &pool::switch_file()?,
        pool::endpoint(a),
    )
    .await
}

/// Explicit user action: works while automatic renewal is off and bypasses backoff.
pub async fn renew_now(state: &Shared, id: &str) -> Result<Value, String> {
    let a = state
        .store
        .lock()
        .unwrap()
        .accounts
        .iter()
        .find(|a| a.id == id)
        .cloned()
        .ok_or("账号不存在")?;
    let path = pool::switch_file()?;
    let raw = raw_account_at(&a, &path)?;
    let observed = raw["access_token"]
        .as_str()
        .ok_or("请先导入当前登录账号或重新授权")?;
    refresh_at(state, &a, Some(observed), &path, pool::endpoint(&a), true).await?;
    let current = raw_account_at(&a, &path)?;
    if let Some(token) = current["access_token"].as_str() {
        let sink = state.login_status_sink.lock().unwrap().clone();
        if let Some(sink) = sink {
            let id = id.to_owned();
            let revision = agentdock_core::provider::credential_revision(token);
            tokio::task::spawn_blocking(move || sink(&id, false, Some(revision)))
                .await
                .map_err(|_| "恢复登录状态失败")?;
        }
    }
    Ok(json!({"ok":true,"expiresAt":current["expiresAt"],"message":"Token 已续期，登录状态已刷新"}))
}

async fn ensure_at(
    state: &Shared,
    a: &Account,
    rejected_token: Option<&str>,
    path: &Path,
    endpoint: &str,
) -> Result<bool, String> {
    refresh_at(state, a, rejected_token, path, endpoint, false).await
}

async fn refresh_at(
    state: &Shared,
    a: &Account,
    rejected_token: Option<&str>,
    path: &Path,
    endpoint: &str,
    manual: bool,
) -> Result<bool, String> {
    // A disconnected chat must not cancel an in-flight rotating refresh and lose
    // the new refresh token after the server has already issued it.
    let state = state.clone();
    let a = a.clone();
    let rejected_token = rejected_token.map(str::to_owned);
    let path = path.to_path_buf();
    let endpoint = endpoint.to_owned();
    tokio::spawn(async move {
        renew_inner(
            &state,
            &a,
            rejected_token.as_deref(),
            &path,
            &endpoint,
            manual,
        )
        .await
    })
    .await
    .map_err(|_| "Token 续期任务异常")?
}

async fn renew_inner(
    state: &Shared,
    a: &Account,
    rejected_token: Option<&str>,
    path: &Path,
    endpoint: &str,
    manual: bool,
) -> Result<bool, String> {
    let gate = state
        .renewal
        .gates
        .lock()
        .unwrap()
        .entry(a.id.clone())
        .or_default()
        .clone();
    let _guard = gate.lock().await;
    let _process_guard = lock_renewal(path, &a.id).await?;
    // Another app or --claim-due process may have rotated the credentials while
    // this task waited. All decisions must use the newly loaded record.
    let raw = raw_account_at(a, path)?;
    let token = raw["access_token"].as_str().unwrap_or("");
    if rejected_token.is_some_and(|old| old != token) {
        return Ok(true);
    }
    if !manual && !state.store.lock().unwrap().settings.token_auto_renew {
        return Ok(false);
    }
    let now = chrono::Utc::now().timestamp_millis();
    if !manual && rejected_token.is_none() && !due(&raw, now) {
        return Ok(false);
    }
    if !manual && raw["needs_relogin"] == true {
        return Ok(false);
    }
    let Some(refresh) = raw["refresh_token"].as_str().filter(|s| !s.is_empty()) else {
        return if manual {
            Err("缺少可用刷新凭据，请重新登录".into())
        } else {
            Ok(false)
        };
    };
    if raw["refreshExpiresAt"]
        .as_i64()
        .is_some_and(|exp| exp <= now)
    {
        return if manual {
            Err("刷新凭据已过期，请重新登录".into())
        } else {
            Ok(false)
        };
    }
    {
        let mut attempts = state.renewal.attempts.lock().unwrap();
        let identity = format!("{token}:{refresh}");
        let unauthorized = rejected_token.is_some();
        if !manual
            && attempts.get(&a.id).is_some_and(|(old, at, forced)| {
                old == &identity && now.saturating_sub(*at) < 300_000 && (!unauthorized || *forced)
            })
        {
            return Ok(false);
        }
        attempts.insert(a.id.clone(), (identity, now, unauthorized));
    }
    let url = format!("{endpoint}/v2/plugin/auth/token/refresh");
    let mut req = state.client.post(url).timeout(Duration::from_secs(30));
    for (k, v) in account::build_auth_headers(&raw) {
        req = req.header(k, v);
    }
    for (k, v) in login_headers(state, &a.variant).await {
        req = req.header(k, v);
    }
    let response = req
        .header("X-Refresh-Token", refresh)
        .header("X-Auth-Refresh-Source", "plugin")
        .header("X-Request-Id", uuid::Uuid::new_v4().to_string())
        .json(&json!({}))
        .send()
        .await
        .map_err(|_| "Token 续期连接失败，稍后自动重试".to_string())?;
    let status = response.status();
    let data: Value = response
        .json()
        .await
        .map_err(|_| "Token 续期响应无效，保留现有凭据".to_string())?;
    if !status.is_success() || !matches!(data["code"].as_i64(), Some(0 | 200)) {
        // A failed refresh alone does not prove the current access token is invalid.
        // Chat/credit 401 still follows the normal permanent-disable path.
        let code = data["code"].as_i64();
        return Err(
            if status == reqwest::StatusCode::UNAUTHORIZED || code == Some(12153) {
                "官方刷新凭据已失效，请重新登录"
            } else {
                "官方暂未接受 Token 续期，保留现有凭据"
            }
            .into(),
        );
    }
    let next = renewed(&raw, &data["data"], now)?;
    let revision =
        agentdock_core::provider::credential_revision(next["access_token"].as_str().unwrap());
    let path = path.to_path_buf();
    let metadata_path = path.clone();
    let committed = tokio::task::spawn_blocking(move || commit_at(&path, &raw, &next))
        .await
        .map_err(|_| "保存续期任务失败")??;
    if let Ok(current) = raw_account_at(a, &metadata_path) {
        let mut rt = state.runtime.lock().unwrap();
        let p = rt
            .presentations
            .entry(a.id.clone())
            .or_insert_with(|| json!({}));
        p["expiresAt"] = current["expiresAt"].clone();
        p["phoneNumber"] = json!(account::profile_phone_number(&current));
        p["needsRelogin"] = json!(current["needs_relogin"] == true);
    }
    if committed {
        let sink = state.login_status_sink.lock().unwrap().clone();
        if let Some(sink) = sink {
            let id = a.id.clone();
            tokio::task::spawn_blocking(move || sink(&id, false, Some(revision)))
                .await
                .map_err(|_| "更新登录状态失败")?;
        }
    }
    Ok(committed
        || rejected_token.is_some_and(|old| {
            raw_account_at(a, &metadata_path)
                .ok()
                .is_some_and(|r| r["access_token"].as_str().is_some_and(|token| token != old))
        }))
}

pub async fn run_scheduler(state: Shared) {
    tokio::time::sleep(Duration::from_secs(10)).await;
    loop {
        let accounts = state.store.lock().unwrap().accounts.clone();
        for a in accounts.into_iter().filter(|a| a.source == "switch") {
            if let Err(error) = ensure(&state, &a, None).await {
                agentdock_core::applog::warn("workbuddy.token", || {
                    (error, json!({"accountId":a.id}))
                });
            }
        }
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}

fn has_encrypted_fields(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.get("$wbEncrypted").is_some() || map.values().any(has_encrypted_fields)
        }
        Value::Array(items) => items.iter().any(has_encrypted_fields),
        _ => false,
    }
}

pub async fn import_current(state: &Shared, variant: WbVariant) -> Result<String, String> {
    let path = auth_file::auth_file_path(variant);
    let raw = auth_file::read_auth_file(variant)
        .ok_or("未找到当前 WorkBuddy 登录态，请先在官方客户端登录")?;
    if path
        .with_file_name(format!(
            "{}.logged-out",
            path.file_name().unwrap().to_string_lossy()
        ))
        .exists()
    {
        return Err("官方客户端已退出登录，请先重新登录".into());
    }
    let root = if !has_encrypted_fields(&raw["auth"]) && !has_encrypted_fields(&raw["account"]) {
        raw
    } else {
        let executable = auth_file::workbuddy_app_path(variant);
        #[cfg(target_os = "macos")]
        let executable = executable.join("Contents/MacOS/Electron");
        let mut cmd = tokio::process::Command::new(executable);
        cmd.args(["-e", include_str!("../resources/official-session.cjs")])
            .arg(path)
            .env("ELECTRON_RUN_AS_NODE", "1")
            .env("AGENTDOCK_READ_SESSION", "1")
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_EXTRA_CA_CERTS")
            .env_remove("DYLD_INSERT_LIBRARIES")
            .env_remove("DYLD_LIBRARY_PATH")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(20), cmd.output())
            .await
            .map_err(|_| "读取官方加密登录态超时")?
            .map_err(|_| "无法启动本机 WorkBuddy 登录态读取器")?;
        if !output.status.success() {
            return Err("无法读取官方加密登录态，请确认本机 WorkBuddy 已登录且版本受支持；也可使用 OAuth 登录添加".into());
        }
        serde_json::from_slice(&output.stdout).map_err(|_| "官方登录态读取失败")?
    };
    let collected =
        auth_file::imported_account_from_root(root, variant).ok_or("官方登录态缺少凭据")?;
    if !collected["access_token"]
        .as_str()
        .is_some_and(|s| !s.is_empty())
        || !collected["uid"].is_string()
    {
        return Err("当前登录态无法用于反代，请重新授权".into());
    }
    if !variant.matches_domain(collected["domain"].as_str().unwrap_or("")) {
        return Err("当前登录态与所选地区不符，未导入".into());
    }
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        import_record_at(&state, variant, collected, &pool::switch_file()?)
    })
    .await
    .map_err(|_| "导入当前账号失败")?
}

fn import_record_at(
    state: &Shared,
    variant: WbVariant,
    collected: Value,
    path: &Path,
) -> Result<String, String> {
    let saved = account::mutate_accounts_at(path, |rows| {
        Ok(account::upsert_collected_account(rows, collected))
    })
    .map_err(|_| "保存当前登录凭据失败")?;
    let source_id = saved["id"].as_str().ok_or("缺少来源账号 ID")?;
    state.import_source_at("switch", Some(source_id), &path.to_path_buf())?;
    let id = format!(
        "{}:{}",
        variant.as_str(),
        saved["uid"].as_str().ok_or("缺少账号 UID")?
    );
    let mut rt = state.runtime.lock().unwrap();
    let p = rt
        .presentations
        .entry(id.clone())
        .or_insert_with(|| json!({}));
    p["expiresAt"] = saved["expiresAt"].clone();
    p["phoneNumber"] = json!(account::profile_phone_number(&saved));
    p["needsRelogin"] = json!(false);
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renewal_without_expiry_clears_retired_expiry_on_disk_and_in_pool() {
        let dir = std::env::temp_dir().join(format!("agentdock-renewal-expiry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        let raw = json!({"id":"source","uid":"fixture","variant":"cn","access_token":"old","refresh_token":"rt","expiresAt":1,"note":"keep"});
        std::fs::write(&path, serde_json::to_vec(&json!([raw])).unwrap()).unwrap();
        let state = pool::AppState::new(dir.join("metadata.json")).unwrap();
        state.import_source_at("switch", None, &path).unwrap();
        let next = renewed(&raw, &json!({"accessToken":"fresh","refreshToken":"rotated"}), 1000).unwrap();
        assert!(commit_at(&path, &raw, &next).unwrap());
        let saved = pool::read_json(&path).unwrap();
        assert_eq!(saved[0]["access_token"], "fresh");
        assert_eq!(saved[0]["refresh_token"], "rotated");
        assert_eq!(saved[0]["note"], "keep");
        assert!(saved[0].get("expiresAt").is_none());
        state.import_source_at("switch", None, &path).unwrap();
        assert!(state.pool_accounts()[0].expires_at.is_none());
        assert!(state.pool_accounts()[0].enabled);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn partial_encryption_requires_decoding_and_extreme_dates_do_not_overflow() {
        assert!(has_encrypted_fields(
            &json!({"accessToken":"plain","refreshToken":{"$wbEncrypted":1,"envelope":"opaque"}})
        ));
        assert!(has_encrypted_fields(
            &json!({"phoneNumber":{"$wbEncrypted":1,"envelope":"opaque"}})
        ));
        assert!(!has_encrypted_fields(
            &json!({"accessToken":"plain","refreshToken":"plain"})
        ));
        assert!(due(
            &json!({"expiresAt":i64::MIN,"refreshedAt":i64::MAX}),
            1000
        ));
    }

    #[tokio::test]
    async fn unauthorized_can_recover_after_failed_background_refresh_and_manual_works_when_auto_off(
    ) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir =
            std::env::temp_dir().join(format!("agentdock-auth-manual-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        std::fs::write(&path, serde_json::to_vec(&json!([{"id":"source","uid":"u1","variant":"cn","access_token":"old","refresh_token":"rt","expiresAt":1}])).unwrap()).unwrap();
        let state = pool::AppState::new(dir.join("metadata.json")).unwrap();
        let a: Account = serde_json::from_value(json!({"id":"cn:u1","source":"switch","sourceId":"source","name":"fixture","variant":"cn","enabled":false})).unwrap();
        state
            .update_store(|s| {
                s.accounts.push(a.clone());
                Ok(())
            })
            .unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let requests = count.clone();
        let router = axum::Router::new().route("/v2/plugin/auth/token/refresh", axum::routing::post(move || {
            let requests = requests.clone(); async move {
                let n = requests.fetch_add(1, Ordering::SeqCst);
                if n == 0 { (axum::http::StatusCode::BAD_GATEWAY, axum::Json(json!({"code":503}))) }
                else { (axum::http::StatusCode::OK, axum::Json(json!({"code":0,"data":{"accessToken":format!("token-{n}"),"refreshToken":format!("rt-{n}"),"expiresIn":172800}}))) }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        assert!(ensure_at(&state, &a, None, &path, &endpoint).await.is_err());
        assert!(!ensure_at(&state, &a, None, &path, &endpoint).await.unwrap());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(ensure_at(&state, &a, Some("old"), &path, &endpoint)
            .await
            .unwrap());
        assert_eq!(count.load(Ordering::SeqCst), 2);
        state
            .update_store(|s| {
                s.settings.token_auto_renew = false;
                Ok(())
            })
            .unwrap();
        assert!(!ensure_at(&state, &a, Some("token-1"), &path, &endpoint)
            .await
            .unwrap());
        assert!(
            refresh_at(&state, &a, Some("token-1"), &path, &endpoint, true)
                .await
                .unwrap()
        );
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert_eq!(pool::read_json(&path).unwrap()[0]["refresh_token"], "rt-2");
        assert!(!state.store.lock().unwrap().accounts[0].enabled);
        account::mutate_accounts_at(&path, |rows| {
            rows[0]["refresh_token"] = Value::Null;
            Ok(())
        })
        .unwrap();
        assert!(
            refresh_at(&state, &a, Some("token-2"), &path, &endpoint, true)
                .await
                .unwrap_err()
                .contains("重新登录")
        );
        assert_eq!(count.load(Ordering::SeqCst), 3);
        task.abort();
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn client_cancellation_does_not_lose_rotated_refresh_token() {
        let dir =
            std::env::temp_dir().join(format!("agentdock-auth-cancel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        std::fs::write(&path, serde_json::to_vec(&json!([{"id":"source","uid":"u1","variant":"cn","access_token":"old","refresh_token":"rt","expiresAt":1}])).unwrap()).unwrap();
        let state = pool::AppState::new(dir.join("metadata.json")).unwrap();
        let a: Account = serde_json::from_value(json!({"id":"cn:u1","source":"switch","sourceId":"source","name":"fixture","variant":"cn","enabled":true})).unwrap();
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (entered, proceed) = (started.clone(), release.clone());
        let router = axum::Router::new().route("/v2/plugin/auth/token/refresh", axum::routing::post(move || {
            let (entered, proceed) = (entered.clone(), proceed.clone()); async move {
                entered.notify_one(); proceed.notified().await;
                axum::Json(json!({"code":0,"data":{"accessToken":"new","refreshToken":"rotated","expiresIn":172800}}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let fixture_path = path.clone();
        let caller =
            tokio::spawn(
                async move { ensure_at(&state, &a, None, &fixture_path, &endpoint).await },
            );
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        caller.abort();
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if pool::read_json(&path).unwrap()[0]["refresh_token"] == "rotated" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        server.abort();
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn current_login_import_replaces_old_credentials_without_duplicate_or_proxy_reenable() {
        let dir =
            std::env::temp_dir().join(format!("agentdock-auth-import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        let old = json!({"id":"stable","uid":"u1","variant":"cn","access_token":"old","refresh_token":"old-rt","needs_relogin":true,"note":"keep"});
        std::fs::write(&path, serde_json::to_vec(&json!([old])).unwrap()).unwrap();
        let state = pool::AppState::new(dir.join("metadata.json")).unwrap();
        state.import_source_at("switch", None, &path).unwrap();
        state.set_enabled("cn:u1", false).unwrap();
        let new: Value = auth_file::imported_account_from_root(json!({"account":{"uid":"u1","nickname":"new name","phoneNumber":"13800138000"},"auth":{"accessToken":"fresh","refreshToken":"fresh-rt","domain":"www.workbuddy.cn","expiresAt":9000000000000i64}}), WbVariant::Cn).unwrap();
        assert_eq!(
            import_record_at(&state, WbVariant::Cn, new, &path).unwrap(),
            "cn:u1"
        );
        let rows = pool::read_json(&path).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["id"], "stable");
        assert_eq!(rows[0]["access_token"], "fresh");
        assert_eq!(rows[0]["refresh_token"], "fresh-rt");
        assert_eq!(rows[0]["note"], "keep");
        assert_ne!(rows[0]["needs_relogin"], true);
        assert_eq!(
            state.snapshot()["accounts"][0]["expiresAt"],
            9000000000000i64
        );
        assert_eq!(
            state.snapshot()["accounts"][0]["presentation"]["phoneNumber"],
            "13800138000"
        );
        assert!(!state.store.lock().unwrap().accounts[0].enabled);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn concurrent_401_renewals_share_one_request_and_keep_proxy_disabled() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir =
            std::env::temp_dir().join(format!("agentdock-auth-http-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        std::fs::write(&path, serde_json::to_vec(&json!([{"id":"source","uid":"u1","variant":"cn","access_token":"old","refresh_token":"rt","expiresAt":1}])).unwrap()).unwrap();
        let state = pool::AppState::new(dir.join("metadata.json")).unwrap();
        let a: Account = serde_json::from_value(json!({"id":"cn:u1","source":"switch","sourceId":"source","name":"fixture","variant":"cn","enabled":false})).unwrap();
        state
            .update_store(|s| {
                s.accounts.push(a.clone());
                Ok(())
            })
            .unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let hits = count.clone();
        let router = axum::Router::new().route("/v2/plugin/auth/token/refresh", axum::routing::post(move |headers: axum::http::HeaderMap| {
            let hits = hits.clone(); async move {
                hits.fetch_add(1, Ordering::SeqCst);
                assert_eq!(headers["authorization"], "Bearer old");
                assert_eq!(headers["x-refresh-token"], "rt");
                assert_eq!(headers["x-auth-refresh-source"], "plugin");
                tokio::time::sleep(Duration::from_millis(30)).await;
                axum::Json(json!({"code":0,"data":{"accessToken":"new","refreshToken":"rotated","expiresIn":172800}}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let (one, two) = tokio::join!(
            ensure_at(&state, &a, Some("old"), &path, &endpoint),
            ensure_at(&state, &a, Some("old"), &path, &endpoint)
        );
        assert!(one.unwrap());
        assert!(two.unwrap());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(
            pool::read_json(&path).unwrap()[0]["refresh_token"],
            "rotated"
        );
        assert!(!state.store.lock().unwrap().accounts[0].enabled);
        assert!(!ensure_at(&state, &a, None, &path, &endpoint).await.unwrap());
        task.abort();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn failed_refresh_preserves_credentials_and_auth_status_and_backs_off() {
        let dir =
            std::env::temp_dir().join(format!("agentdock-auth-reject-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        let original = json!([{"id":"source","uid":"u1","variant":"cn","access_token":"old","refresh_token":"rt","expiresAt":1}]);
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        let state = pool::AppState::new(dir.join("metadata.json")).unwrap();
        let a: Account = serde_json::from_value(json!({"id":"cn:u1","source":"switch","sourceId":"source","name":"fixture","variant":"cn","enabled":true})).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let router = axum::Router::new().route(
            "/v2/plugin/auth/token/refresh",
            axum::routing::post(|| async { (axum::http::StatusCode::BAD_GATEWAY, "not json") }),
        );
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        assert!(ensure_at(&state, &a, None, &path, &endpoint).await.is_err());
        assert!(!ensure_at(&state, &a, None, &path, &endpoint).await.unwrap());
        assert_eq!(pool::read_json(&path).unwrap(), original);
        assert!(state.runtime.lock().unwrap().auth_failed.is_empty());
        task.abort();
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn renewal_preserves_rotating_token_and_normalizes_relative_expiries() {
        let raw = json!({"access_token":"old","refresh_token":"rt","auth_raw":{"scope":"profile"},"refreshExpiresAt":900000,"needs_relogin":true});
        let next = renewed(&raw, &json!({"accessToken":"new","expiresIn":3600,"refreshToken":"rotated","refreshExpiresIn":7200}), 1000).unwrap();
        assert_eq!(next["refresh_token"], "rotated");
        assert_eq!(next["refreshExpiresAt"], 7201000);
        assert_eq!(next["auth_raw"]["scope"], "profile");
        assert_eq!(next["needs_relogin"], false);
        let no_expiry = renewed(
            &raw,
            &json!({"accessToken":"new", "refreshToken":"rotated"}),
            1000,
        )
        .unwrap();
        assert!(no_expiry["expiresAt"].is_null());
        assert_eq!(no_expiry["refresh_token"], "rotated");
    }
    #[test]
    fn refresh_commit_rejects_newer_login_and_keeps_other_accounts_and_notes() {
        let dir = std::env::temp_dir().join(format!("agentdock-renewal-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        let old = json!({"id":"one","uid":"u1","variant":"cn","access_token":"old","refresh_token":"rt","note":"before"});
        std::fs::write(
            &path,
            serde_json::to_vec(&json!([old, {"id":"two","uid":"u2"}])).unwrap(),
        )
        .unwrap();
        let next = renewed(&old, &json!({"accessToken":"new","expiresIn":3600}), 1000).unwrap();
        account::mutate_accounts_at(&path, |rows| {
            rows[0]["note"] = json!("edited");
            Ok(())
        })
        .unwrap();
        assert!(commit_at(&path, &old, &next).unwrap());
        assert!(!commit_at(&path, &old, &next).unwrap());
        let rows = pool::read_json(&path).unwrap();
        assert_eq!(rows[0]["note"], "edited");
        assert_eq!(rows[1]["uid"], "u2");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
