//! Desktop commands. General gateway/catalog/pool commands plus every
//! provider-specific operation from the original three applications.
use crate::{app::Shared, catalog, settings::RouteOverride};
use agentdock_core::provider::ProviderKind;
use serde_json::{json, Value};
use tauri::State;

type Res = Result<Value, String>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| "后台任务失败".to_owned())
}
fn provider(p: &str) -> Result<ProviderKind, String> {
    ProviderKind::parse(p).ok_or_else(|| "未知的端".to_owned())
}

// ───────────────────────── 通用 ─────────────────────────

#[tauri::command]
pub fn app_snapshot(hub: State<'_, Shared>) -> Value {
    let settings = hub.settings();
    let running = hub.server.lock().unwrap().as_ref().map(|s| s.port);
    let active: Vec<_> = hub.active.lock().unwrap().values().cloned().collect();
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "dataDir": hub.dir,
        "running": running.is_some(),
        "port": running.unwrap_or(settings.port),
        "apiKey": hub.api_key(),
        "error": *hub.error.lock().unwrap(),
        "settings": settings,
        "active": active,
        "checkin": *hub.checkin.lock().unwrap(),
    })
}

// ── unified check-in ──
#[tauri::command]
pub async fn checkin_run_now(hub: State<'_, Shared>) -> Res {
    let r = crate::tasks::run_pass(hub.inner(), "manual").await;
    if r["skipped"] == "running" {
        return Err("签到正在执行中，请稍后再试".into());
    }
    Ok(r)
}

// ── runtime log ──
#[tauri::command]
pub fn app_logs(hub: State<'_, Shared>, filter: Option<Value>) -> Value {
    let mut v = hub.applog.query(&filter.unwrap_or_default());
    v["enabled"] = json!(agentdock_core::applog::enabled());
    v
}
#[tauri::command]
pub fn clear_app_logs(hub: State<'_, Shared>) -> Value {
    hub.applog.clear();
    json!({"ok": true})
}

// ── all-platform backup ──
#[tauri::command]
pub async fn backup_export(hub: State<'_, Shared>, parts: Option<crate::backup::Parts>) -> Res {
    let hub = hub.inner().clone();
    blocking(move || crate::backup::export(&hub, &parts.unwrap_or_default())).await?
}
#[tauri::command]
pub async fn backup_inspect(path: String) -> Res {
    blocking(move || {
        let path = crate::backup::expand(&path);
        crate::backup::read(&path).map(|b| json!({"path": path, "summary": crate::backup::summary(&b)}))
    })
    .await?
}
#[tauri::command]
pub async fn backup_import(hub: State<'_, Shared>, path: String, parts: Option<crate::backup::Parts>) -> Res {
    let hub = hub.inner().clone();
    blocking(move || {
        let bundle = crate::backup::read(&crate::backup::expand(&path))?;
        crate::backup::import(&hub, &bundle, &parts.unwrap_or_default())
    })
    .await?
}

#[tauri::command]
pub async fn gateway_toggle(hub: State<'_, Shared>, enabled: bool) -> Res {
    if enabled {
        let port = crate::server::start(hub.inner().clone()).await?;
        Ok(json!({"running":true,"port":port}))
    } else {
        crate::server::stop(hub.inner());
        Ok(json!({"running":false}))
    }
}

/// Merge a partial settings object (routes are edited through route commands).
#[tauri::command]
pub async fn save_settings(hub: State<'_, Shared>, patch: Value) -> Res {
    let hub = hub.inner().clone();
    blocking(move || {
        hub.update_settings(|s| {
            let mut value = serde_json::to_value(&*s).unwrap();
            for (k, v) in patch.as_object().ok_or("设置格式无效")? {
                if k == "routes" {
                    continue;
                }
                // `checkin` accepts partial patches so concurrent switches never clobber each other.
                match (value[k].as_object_mut(), v.as_object()) {
                    (Some(dst), Some(src)) if k == "checkin" => {
                        for (field, x) in src {
                            dst.insert(field.clone(), x.clone());
                        }
                    }
                    _ => value[k] = v.clone(),
                }
            }
            *s = serde_json::from_value(value).map_err(|e| format!("设置格式无效：{e}"))?;
            Ok(())
        })
        .map(|s| json!(s))
    })
    .await?
}

#[tauri::command]
pub fn generate_api_key() -> String {
    crate::api_key::generate()
}

#[tauri::command]
pub async fn set_api_key(hub: State<'_, Shared>, value: String, expected: String) -> Res {
    let hub = hub.inner().clone();
    blocking(move || hub.set_api_key(value, &expected).map(|_| json!({"ok":true}))).await?
}

#[tauri::command]
pub async fn pool_snapshot(hub: State<'_, Shared>) -> Res {
    let h = hub.inner().clone();
    let (wb, qoder, trae, runtime) = blocking(move || {
        (
            h.wb.snapshot(),
            qoder_build_state(&h),
            trae_build_state(&h),
            h.account_runtime(),
        )
    })
    .await?;
    Ok(json!({"workbuddy":wb,"qoder":qoder,"trae":trae,"runtime":runtime}))
}

#[tauri::command]
pub async fn set_account_enabled(hub: State<'_, Shared>, provider_key: String, id: String, enabled: bool) -> Res {
    let p = provider(&provider_key)?;
    let hub = hub.inner().clone();
    blocking(move || hub.set_account_enabled(p, &id, enabled).map(|_| json!({"ok":true}))).await?
}

#[tauri::command]
pub fn clear_account_runtime(hub: State<'_, Shared>, key: String) -> Value {
    let mut rt = hub.runtime.lock().unwrap();
    rt.cooldown.remove(&key);
    rt.auth_failed.remove(&key);
    json!({"ok":true})
}

#[tauri::command]
pub async fn catalog_snapshot(hub: State<'_, Shared>) -> Res {
    let hub = hub.inner().clone();
    blocking(move || {
        let settings = hub.settings();
        let models = hub.upstream_models();
        let routes = catalog::build(&models, &settings);
        let scope = catalog::Scope::default();
        let wb_store = hub.wb.store.lock().unwrap();
        let wb_catalogs = workbuddy::model_registry::catalogs(&wb_store);
        let qoder_updated = hub.qoder.catalog_updated();
        let status = json!({
            "workbuddy": {
                "cn": {"count": models.iter().filter(|m| m.provider()==ProviderKind::Workbuddy && m.region=="cn").count(), "synced": wb_store.model_catalogs.contains_key("cn"), "bundled": wb_store.model_catalogs.is_empty() && wb_catalogs.contains_key("cn")},
                "intl": {"count": models.iter().filter(|m| m.provider()==ProviderKind::Workbuddy && m.region=="intl").count(), "synced": wb_store.model_catalogs.contains_key("ai")},
            },
            "qoder": {
                "cn": {"count": models.iter().filter(|m| m.provider()==ProviderKind::Qoder && m.region=="cn").count(), "updated": qoder_updated.get("cn")},
                "intl": {"count": models.iter().filter(|m| m.provider()==ProviderKind::Qoder && m.region=="intl").count(), "updated": qoder_updated.get("intl")},
            },
            "trae": {
                "cn": {"count": models.iter().filter(|m| m.provider()==ProviderKind::Trae && m.region=="cn").count()},
                "intl": {"count": models.iter().filter(|m| m.provider()==ProviderKind::Trae && m.region=="intl").count()},
            },
        });
        json!({
            "routes": routes.iter().map(|r| r.view(&scope)).collect::<Vec<_>>(),
            "upstreams": models.iter().map(catalog::target_view).collect::<Vec<_>>(),
            "status": status,
        })
    })
    .await
}

/// Sync catalogs: a provider (optionally one region) or every provider.
#[tauri::command]
pub async fn sync_catalog(hub: State<'_, Shared>, provider_key: Option<String>, region: Option<String>) -> Res {
    let hub = hub.inner().clone();
    let targets: Vec<ProviderKind> = match provider_key.as_deref() {
        Some(p) => vec![provider(p)?],
        None => ProviderKind::ALL.to_vec(),
    };
    let settings = hub.settings();
    let mut synced = vec![];
    let mut warnings = vec![];
    for p in targets {
        let scope = settings.scope(p);
        let regions: Vec<String> = match &region {
            Some(r) => vec![r.clone()],
            None => scope.regions.clone(),
        };
        match p {
            ProviderKind::Workbuddy => {
                for r in &regions {
                    let variant = workbuddy::pool::variant_of_region(r);
                    let account = {
                        let store = hub.wb.store.lock().unwrap();
                        store
                            .accounts
                            .iter()
                            .find(|a| a.variant == variant && a.enabled)
                            .or_else(|| store.accounts.iter().find(|a| a.variant == variant))
                            .map(|a| a.id.clone())
                    };
                    let Some(id) = account else {
                        warnings.push(format!("WorkBuddy {}：没有可用账号", agentdock_core::provider::region_label(r)));
                        continue;
                    };
                    match workbuddy::proxy::sync_models(hub.wb.clone(), Some(id)).await {
                        Ok(n) => synced.push(json!({"provider":"workbuddy","region":r,"count":n})),
                        Err(e) => warnings.push(format!("WorkBuddy {}：{e}", agentdock_core::provider::region_label(r))),
                    }
                }
            }
            ProviderKind::Qoder => {
                let refs: Vec<&str> = regions.iter().map(String::as_str).collect();
                match hub.qoder.refresh_models(&refs).await {
                    Ok(v) => {
                        synced.push(json!({"provider":"qoder","count":v["count"],"regions":v["synced"]}));
                        for w in v["warnings"].as_array().into_iter().flatten() {
                            warnings.push(w.as_str().unwrap_or("").to_owned());
                        }
                    }
                    Err(e) => warnings.push(e),
                }
            }
            ProviderKind::Trae => {
                let ids: Vec<String> = hub
                    .trae
                    .store
                    .accounts()
                    .iter()
                    .filter(|a| regions.iter().any(|r| a["variant"] == r.as_str()))
                    .filter_map(|a| a["id"].as_str().map(str::to_owned))
                    .collect();
                if ids.is_empty() {
                    warnings.push("Trae：没有可用账号".into());
                }
                for id in ids {
                    match hub.trae.sync_models(&id).await {
                        Ok(m) => synced.push(json!({"provider":"trae","account":id,"count":m.len()})),
                        Err(e) => warnings.push(format!("Trae：{e}")),
                    }
                }
            }
        }
    }
    if synced.is_empty() && !warnings.is_empty() {
        return Err(warnings.join("；"));
    }
    Ok(json!({"synced":synced,"warnings":warnings}))
}

#[tauri::command]
pub async fn save_route(hub: State<'_, Shared>, route_id: String, alias: String, targets: Option<Vec<String>>, disabled: bool) -> Res {
    let hub = hub.inner().clone();
    blocking(move || {
        let custom = hub.settings().routes.get(&route_id).is_some_and(|o| o.custom);
        hub.save_route(&route_id, RouteOverride { alias: alias.trim().to_owned(), targets, disabled, custom })
            .map(|_| json!({"ok":true}))
    })
    .await?
}

#[tauri::command]
pub async fn create_route(hub: State<'_, Shared>, alias: String, targets: Vec<String>) -> Res {
    let hub = hub.inner().clone();
    blocking(move || {
        let id = format!("custom:{}", uuid::Uuid::new_v4().simple());
        if alias.trim().is_empty() {
            return Err("请填写对外模型别名".into());
        }
        hub.save_route(&id, RouteOverride { alias: alias.trim().to_owned(), targets: Some(targets), disabled: false, custom: true })
            .map(|_| json!({"ok":true,"routeId":id}))
    })
    .await?
}

#[tauri::command]
pub async fn reset_route(hub: State<'_, Shared>, route_id: String) -> Res {
    let hub = hub.inner().clone();
    blocking(move || hub.reset_route(&route_id).map(|_| json!({"ok":true}))).await?
}

#[tauri::command]
pub fn request_logs(hub: State<'_, Shared>) -> Value {
    let active: Vec<_> = hub.active.lock().unwrap().values().cloned().collect();
    json!({"logs":hub.logs.list(),"active":active})
}

#[tauri::command]
pub fn clear_logs(hub: State<'_, Shared>) -> Res {
    hub.logs.clear().map(|_| json!({"ok":true}))
}

#[tauri::command]
pub async fn proxy_statistics(hub: State<'_, Shared>, filter: Value, offset_minutes: Option<i64>) -> Res {
    let hub = hub.inner().clone();
    blocking(move || crate::logs::statistics(&hub.logs.usage_rows(), &filter, offset_minutes.unwrap_or(480))).await
}

#[tauri::command]
pub async fn open_url(url: String) -> Res {
    let parsed = reqwest::Url::parse(&url).map_err(|_| "链接无效")?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("仅允许打开 HTTPS 链接".into());
    }
    qoder::platform::open_url(parsed.as_str()).map_err(|_| "打开浏览器失败，请复制链接")?;
    Ok(json!({"ok":true}))
}

/// Fixed links of the About page (never a caller-supplied URL).
#[tauri::command]
pub async fn open_about_link(destination: String) -> Res {
    let url = match destination.as_str() {
        "author" => "https://github.com/vs2pk0",
        "repository" => "https://github.com/AI-Good-Tool/AgentDock",
        "sponsor" => "https://github.com/vs2pk0/codex-switcher/blob/main/doc/sponsor.md",
        "issues" => "https://github.com/AI-Good-Tool/AgentDock/issues",
        "releases" => crate::app_update::RELEASES_URL,
        _ => return Err("未知的关于页面链接".into()),
    };
    open_url(url.into()).await
}

#[tauri::command]
pub fn reveal_data_dir(hub: State<'_, Shared>) -> Res {
    // Finder / Explorer; `explorer.exe` exits 1 even on success, so only spawn errors count.
    #[cfg(windows)]
    let opened = std::process::Command::new("explorer.exe").arg(&hub.dir).spawn().map(|_| ());
    #[cfg(not(windows))]
    let opened = std::process::Command::new("/usr/bin/open").arg(&hub.dir).status().map(|_| ());
    opened.map_err(|_| "无法打开数据目录")?;
    Ok(json!({"ok":true}))
}

// ───────────── 系统定时签到（macOS launchd / Windows 任务计划程序） ─────────────
// Runs `AgentDock --claim` daily at 10:05 and 22:05 without the app open.

#[cfg(not(windows))]
fn launchd_plist() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_default().join("Library/LaunchAgents/com.agentdock.claim.plist")
}

#[cfg(windows)]
const WIN_TASKS: [(&str, &str); 2] = [("AgentDock\\Claim-1005", "10:05"), ("AgentDock\\Claim-2205", "22:05")];

#[cfg(windows)]
fn schtasks(args: &[&str]) -> Option<std::process::Output> {
    use std::os::windows::process::CommandExt;
    let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
    std::process::Command::new(root.join("System32\\schtasks.exe"))
        .args(args)
        .creation_flags(0x0800_0000)
        .output()
        .ok()
}

#[tauri::command]
pub fn launchd_status() -> Value {
    #[cfg(windows)]
    {
        let installed = WIN_TASKS
            .iter()
            .all(|(name, _)| schtasks(&["/Query", "/TN", name]).is_some_and(|o| o.status.success()));
        json!({"installed": installed, "path": "Task Scheduler\\AgentDock"})
    }
    #[cfg(not(windows))]
    json!({"installed": launchd_plist().exists(), "path": launchd_plist()})
}

#[cfg(windows)]
#[tauri::command]
pub async fn launchd_install(_hub: State<'_, Shared>, install: bool) -> Res {
    blocking(move || {
        for (name, _) in WIN_TASKS {
            let _ = schtasks(&["/Delete", "/TN", name, "/F"]);
        }
        if !install {
            return Ok(json!({"installed":false}));
        }
        let exe = std::env::current_exe().map_err(|_| "无法定位应用程序")?;
        let command = format!("\"{}\" --claim", exe.display());
        for (name, time) in WIN_TASKS {
            let ok = schtasks(&["/Create", "/TN", name, "/SC", "DAILY", "/ST", time, "/TR", &command, "/F"])
                .is_some_and(|o| o.status.success());
            if !ok {
                return Err("无法注册 Windows 定时任务".to_string());
            }
        }
        Ok(json!({"installed":true,"path":"Task Scheduler\\AgentDock"}))
    })
    .await?
}

#[cfg(not(windows))]
#[tauri::command]
pub async fn launchd_install(hub: State<'_, Shared>, install: bool) -> Res {
    let plist = launchd_plist();
    let uid = String::from_utf8_lossy(&std::process::Command::new("/usr/bin/id").arg("-u").output().map_err(|e| e.to_string())?.stdout).trim().to_owned();
    let _ = std::process::Command::new("/bin/launchctl").args(["bootout", &format!("gui/{uid}/com.agentdock.claim")]).status();
    if !install {
        let _ = std::fs::remove_file(&plist);
        return Ok(json!({"installed":false}));
    }
    let exe = std::env::current_exe().map_err(|_| "无法定位应用程序")?;
    let log = hub.dir.join("auto-claim.log");
    let xml = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.agentdock.claim</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>--claim</string></array>
  <key>StartCalendarInterval</key>
  <array>
    <dict><key>Hour</key><integer>10</integer><key>Minute</key><integer>5</integer></dict>
    <dict><key>Hour</key><integer>22</integer><key>Minute</key><integer>5</integer></dict>
  </array>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#, exe.display(), log.display(), log.display());
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "无法创建 LaunchAgents 目录")?;
    }
    std::fs::write(&plist, xml).map_err(|_| "无法写入定时任务")?;
    std::process::Command::new("/bin/launchctl")
        .args(["bootstrap", &format!("gui/{uid}")])
        .arg(&plist)
        .status()
        .map_err(|_| "无法注册定时任务")?;
    Ok(json!({"installed":true,"path":plist,"log":log}))
}

// ───────────────────────── WorkBuddy ─────────────────────────

#[tauri::command]
pub async fn wb_import_accounts(hub: State<'_, Shared>, source: String) -> Res {
    let wb = hub.wb.clone();
    blocking(move || wb.import_accounts(&source).map(|n| json!(n))).await?
}
#[tauri::command]
pub async fn wb_oauth_start(variant: String, confirmed: bool) -> Res {
    if !confirmed {
        return Err("请确认通过官方 OAuth 登录并保存账号到本机 Switch 账号库".into());
    }
    let variant = match variant.as_str() {
        "cn" => wb_switch_core::modules::variant::WbVariant::Cn,
        "ai" | "intl" => wb_switch_core::modules::variant::WbVariant::Ai,
        _ => return Err("不支持的账号区域".into()),
    };
    wb_switch_core::modules::oauth::oauth_start(variant)
        .await
        .map_err(|_| "无法发起官方登录，请检查网络后重试".into())
}
#[tauri::command]
pub async fn wb_oauth_poll(hub: State<'_, Shared>, login_id: String) -> Res {
    let result = wb_switch_core::modules::oauth::oauth_poll(&login_id).await;
    if result["error"].as_str().is_some() {
        return Ok(json!({"done":true,"error":"登录失败或已超时，请重新发起官方授权"}));
    }
    if result["done"] == true {
        let id = result["result"]["id"].as_str().ok_or("授权结果缺少账号标识")?.to_string();
        let wb = hub.wb.clone();
        let count = blocking(move || wb.import_account_source("switch", Some(&id))).await??;
        if count != 1 {
            return Err("账号已保存至 Switch，但尚未接入账号池，请手动导入".into());
        }
        return Ok(json!({"done":true,"added":true}));
    }
    Ok(json!({"done":false}))
}
#[tauri::command]
pub async fn wb_open_oauth_link(url: String) -> Res {
    let parsed = reqwest::Url::parse(&url).map_err(|_| "授权链接无效")?;
    let host = parsed.host_str().unwrap_or("");
    if parsed.scheme() != "https"
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !["workbuddy.cn", "www.workbuddy.cn", "workbuddy.ai", "www.workbuddy.ai", "codebuddy.cn", "www.codebuddy.cn"].contains(&host)
    {
        return Err("仅允许打开官方 HTTPS 授权链接，请复制链接核对".into());
    }
    open_url(url).await
}
#[tauri::command]
pub async fn wb_set_auto_checkin(hub: State<'_, Shared>, enabled: bool) -> Res {
    let hub = hub.inner().clone();
    blocking(move || {
        hub.wb.set_auto_checkin(enabled)?;
        hub.update_settings(|s| {
            s.checkin.workbuddy = enabled;
            Ok(())
        })?;
        Ok(json!({"ok":true}))
    })
    .await?
}
#[tauri::command]
pub async fn wb_set_checkin_config(hub: State<'_, Shared>, config: workbuddy::checkin::Config) -> Res {
    config.validate()?;
    let wb = hub.wb.clone();
    blocking(move || {
        wb.update_store(|store| {
            store.checkin_config = config;
            Ok(())
        })
        .map(|_| json!({"ok":true}))
    })
    .await?
}
#[tauri::command]
pub async fn wb_set_account_checkin(hub: State<'_, Shared>, id: String, enabled: bool) -> Res {
    let wb = hub.wb.clone();
    blocking(move || wb.set_checkin_enabled(&id, enabled).map(|_| json!({"ok":true}))).await?
}
#[tauri::command]
pub async fn wb_update_display(hub: State<'_, Shared>, id: String, note: String, field: String) -> Res {
    let wb = hub.wb.clone();
    blocking(move || wb.update_display(&id, note, field).map(|_| json!({"ok":true}))).await?
}
#[tauri::command]
pub async fn wb_set_runtime_settings(hub: State<'_, Shared>, settings: workbuddy::settings::Settings, confirmed: bool) -> Res {
    let wb = hub.wb.clone();
    blocking(move || wb.set_settings(settings, confirmed).map(|_| json!({"ok":true}))).await?
}
#[tauri::command]
pub async fn wb_refresh_statuses(hub: State<'_, Shared>) -> Res {
    let wb = hub.wb.clone();
    blocking(move || {
        wb_switch_core::modules::limits::invalidate_scan_cache();
        workbuddy::switch_bridge::refresh_local(&wb).map(|_| json!({"ok":true}))
    })
    .await?
}
#[tauri::command]
pub async fn wb_refresh_credits(hub: State<'_, Shared>, ids: Vec<String>) -> Res {
    workbuddy::settings::refresh_accounts(hub.wb.clone(), ids).await
}
#[tauri::command]
pub async fn wb_switch_client(hub: State<'_, Shared>, id: String, target: String, confirmed: bool) -> Res {
    let wb = hub.wb.clone();
    blocking(move || workbuddy::switch_bridge::switch_client(&wb, &id, &target, confirmed)).await?
}
#[tauri::command]
pub async fn wb_checkin(hub: State<'_, Shared>, id: String, submit: bool) -> Res {
    let r = workbuddy::proxy::checkin(hub.wb.clone(), id.clone(), submit).await;
    if submit && r.is_ok() {
        // The reward lands in the credit balance: refresh it so the pool and check-in views agree.
        let _ = workbuddy::settings::refresh_accounts(hub.wb.clone(), vec![id]).await;
    }
    r
}
#[tauri::command]
pub async fn wb_credit_statistics(hub: State<'_, Shared>) -> Res {
    let wb = hub.wb.clone();
    blocking(move || workbuddy::statistics::credit_statistics(&wb)).await
}
/// WorkBuddy rows of the unified ledger, in the WorkBuddy statistics format.
fn wb_proxy_source(hub: &Shared) -> Value {
    let rows: Vec<Value> = hub
        .logs
        .usage_rows()
        .into_iter()
        .filter(|r| r["provider"] == "workbuddy")
        .map(|mut r| {
            r["accountVariant"] = json!(if r["region"] == "intl" { "ai" } else { "cn" });
            r
        })
        .collect();
    let mut proxy = workbuddy::statistics::proxy_statistics(&rows);
    proxy["filesScanned"] = json!(1);
    proxy
}
#[tauri::command]
pub async fn wb_token_statistics(hub: State<'_, Shared>) -> Res {
    let hub = hub.inner().clone();
    blocking(move || {
        let mut result = wb_switch_core::modules::token_stats::get_statistics(None);
        if let Some(sources) = result["sources"].as_array_mut() {
            sources.push(wb_proxy_source(&hub));
        }
        result["timezone"] = json!("Asia/Shanghai");
        result
    })
    .await
}
#[tauri::command]
pub async fn wb_proxy_statistics(hub: State<'_, Shared>) -> Res {
    let hub = hub.inner().clone();
    blocking(move || wb_proxy_source(&hub)).await
}
#[tauri::command]
pub async fn wb_refresh_official_statistics(hub: State<'_, Shared>, confirmed: bool) -> Res {
    workbuddy::statistics::refresh_official(&hub.wb, confirmed).await
}
#[tauri::command]
pub async fn wb_prepare_device(hub: State<'_, Shared>) -> Res {
    hub.wb.official_transport.prepare_device().await.map(|_| json!({"ok":true}))
}

// ───────────────────────── Qoder ─────────────────────────

pub fn qoder_build_state(hub: &Shared) -> Value {
    use qoder::qcore as qc;
    let mut editions = serde_json::Map::new();
    let mut current = std::collections::HashMap::new();
    for ed in qc::EDITIONS {
        let installed = std::path::Path::new(ed.app_path).is_dir();
        let mut uid = String::new();
        let mut uname = String::new();
        if installed {
            if let Some(sess) = qc::decrypt_auth_dat(ed) {
                uid = sess.pointer("/user/id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                uname = sess
                    .pointer("/user/name")
                    .and_then(|x| x.as_str())
                    .or(sess.pointer("/user/email").and_then(|x| x.as_str()))
                    .or(sess.pointer("/user/phone").and_then(|x| x.as_str()))
                    .unwrap_or("")
                    .to_string();
            }
        }
        current.insert(ed.key.to_string(), uid.clone());
        editions.insert(ed.key.into(), json!({"label":ed.label,"installed":installed,"running":installed && qoder_running_cached(ed.key),"currentUid":uid,"currentName":uname}));
    }
    let credits = hub.qoder.credit_cache();
    let config = hub.qoder.config();
    let accounts: Vec<Value> = qc::load_accounts()
        .iter()
        .map(|a| {
            json!({"id":a.id(),"edition":a.edition,"editionLabel":qc::edition(&a.edition).map(|e| e.label).unwrap_or(""),
                "name":a.name,"email":a.email,"phone":a.phone,"note":a.note,"avatarUrl":a.avatarUrl,"uid":a.uid,
                "expiresAt":a.expiresAt,"refreshTokenExpiresAt":a.refreshTokenExpiresAt,"savedAt":a.savedAt,
                "hasToken":!a.token.is_empty(),"hasRefreshToken":!a.refreshToken.is_empty(),
                "isCurrent":current.get(&a.edition).is_some_and(|u| u == &a.uid),
                "enabled":!config.disabled_accounts.contains(&a.id()),"credit":credits.get(&a.id())})
        })
        .collect();
    let log = qc::load_claim_log();
    let tail: Vec<Value> = log.iter().rev().take(30).cloned().collect::<Vec<_>>().into_iter().rev().collect();
    json!({"editions":editions,"accounts":accounts,"settings":qc::load_settings(),"scheduler":*hub.qoder_scheduler.lock().unwrap(),"claimLog":tail,"gateway":config})
}

#[tauri::command]
pub async fn qoder_state(hub: State<'_, Shared>) -> Res {
    let hub = hub.inner().clone();
    blocking(move || qoder_build_state(&hub)).await
}
#[tauri::command]
pub async fn qoder_capture(edition: String) -> Res {
    blocking(move || match qoder::qcore::capture_current(&edition) {
        Ok((acc, is_new)) => json!({"ok":true,"message":format!("{} {}", if is_new {"已采集新账号"} else {"已更新账号"}, acc.name)}),
        Err(e) => json!({"ok":false,"message":e}),
    })
    .await
}
#[tauri::command]
pub async fn qoder_switch_account(id: String) -> Res {
    blocking(move || {
        let mut steps = vec![];
        match qoder::qcore::switch_account(&id, &mut steps) {
            Ok(msg) => json!({"ok":true,"message":msg,"steps":steps}),
            Err(e) => json!({"ok":false,"message":e,"steps":steps}),
        }
    })
    .await
}
pub fn qoder_claim_blocking(id: Option<String>) -> Value {
    use qoder::qcore as qc;
    if let Some(aid) = id {
        let mut accounts = qc::load_accounts();
        match accounts.iter().position(|a| a.id() == aid) {
            Some(i) => {
                let r = qc::claim_account(&mut accounts[i], false);
                let _ = qc::save_accounts(&accounts);
                qc::append_claim_log(&[r.clone()]);
                let ok = matches!(r["phase"].as_str(), Some("claimed" | "nothing" | "dry_run"));
                json!({"ok":ok,"message":format!("{}：{}", r["name"].as_str().unwrap_or(""), r["msg"].as_str().unwrap_or(""))})
            }
            None => json!({"ok":false,"message":"账号不存在"}),
        }
    } else {
        let results = qc::claim_all(None, false);
        let total: f64 = results.iter().filter_map(|r| r["credits"].as_f64()).sum();
        let lines: Vec<String> = results.iter().map(|r| format!("{} {}", r["name"].as_str().unwrap_or(""), r["msg"].as_str().unwrap_or(""))).collect();
        json!({"ok":true,"message":format!("共领取 {} Credits。{}", total, lines.join("；")),"results":results})
    }
}
#[tauri::command]
pub async fn qoder_claim(id: Option<String>) -> Res {
    blocking(move || qoder_claim_blocking(id)).await
}
#[tauri::command]
pub async fn qoder_credits(hub: State<'_, Shared>, id: String) -> Res {
    let hub = hub.inner().clone();
    blocking(move || {
        use qoder::qcore as qc;
        let mut accounts = qc::load_accounts();
        let Some(i) = accounts.iter().position(|a| a.id() == id) else {
            return json!({"ok":false,"message":"账号不存在"});
        };
        if let Err(e) = qc::ensure_token(&mut accounts[i]) {
            hub.qoder.cache_credits(&id, json!({"ok":false,"error":e}));
            return json!({"ok":false,"message":e});
        }
        let _ = qc::save_accounts(&accounts);
        let acc = &accounts[i];
        let (s1, summary) = qc::credits_summary(acc);
        let (s2, camps) = qc::get_campaigns(acc);
        let (s3, usage) = qc::quota_info(acc);
        let usage = if s3 == 200 { usage.get("qoderUsage").cloned().unwrap_or(usage) } else { Value::Null };
        let data = json!({"summary": if s1 == 200 { summary } else { Value::Null },
            "campaigns": if s2 == 200 { camps.get("campaigns").cloned().unwrap_or(json!([])) } else { json!([]) },
            "usage": usage});
        if data["usage"].is_object() {
            hub.qoder.cache_credits(&id, json!({"ok":true,"usage":data["usage"],"updatedAt":chrono::Utc::now().timestamp_millis()}));
        } else {
            hub.qoder.cache_credits(&id, json!({"ok":false,"error":"额度查询失败"}));
        }
        json!({"ok":true,"data":data})
    })
    .await
}
#[tauri::command]
pub async fn qoder_set_note(id: String, note: String) -> Res {
    blocking(move || {
        let mut accounts = qoder::qcore::load_accounts();
        for a in accounts.iter_mut().filter(|a| a.id() == id) {
            a.note = note.clone();
        }
        let _ = qoder::qcore::save_accounts(&accounts);
        json!({"ok":true,"message":"已保存备注"})
    })
    .await
}
#[tauri::command]
pub async fn qoder_delete_account(hub: State<'_, Shared>, id: String) -> Res {
    let hub = hub.inner().clone();
    blocking(move || {
        let accounts: Vec<_> = qoder::qcore::load_accounts().into_iter().filter(|a| a.id() != id).collect();
        let _ = qoder::qcore::save_accounts(&accounts);
        hub.qoder.forget_account(&id);
        json!({"ok":true,"message":"已删除"})
    })
    .await
}
#[tauri::command]
pub async fn qoder_set_auto_claim(hub: State<'_, Shared>, enabled: bool) -> Res {
    let hub = hub.inner().clone();
    blocking(move || {
        let mut s = qoder::qcore::load_settings();
        s["autoClaim"] = json!(enabled);
        qoder::qcore::save_settings(&s);
        hub.update_settings(|s| {
            s.checkin.qoder = enabled;
            Ok(())
        })?;
        Ok(json!({"ok":true,"message":format!("自动领取已{}", if enabled {"开启"} else {"关闭"})}))
    })
    .await?
}
/// Status-only check, reused for 3 s: on Windows each check launches PowerShell.
/// Switching and session writes keep calling `app_running` directly.
fn qoder_running_cached(key: &str) -> bool {
    use std::sync::Mutex;
    static CACHE: Mutex<Vec<(String, std::time::Instant, bool)>> = Mutex::new(Vec::new());
    if let Some((_, _, v)) = CACHE.lock().unwrap().iter().find(|(k, at, _)| k == key && at.elapsed().as_secs() < 3) {
        return *v;
    }
    let v = qoder::qcore::app_running(key);
    let mut cache = CACHE.lock().unwrap();
    cache.retain(|(k, _, _)| k != key);
    cache.push((key.to_owned(), std::time::Instant::now(), v));
    v
}

#[tauri::command]
pub async fn qoder_open_app(edition: String) -> Res {
    blocking(move || {
        qoder::qcore::launch_app(&edition)?;
        Ok(json!({"ok":true,"message":"已启动"}))
    })
    .await?
}
#[tauri::command]
pub async fn qoder_export_accounts() -> Res {
    blocking(move || {
        let path = dirs::download_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default()).join("qoder-accounts-backup.json");
        match qoder::qcore::export_accounts(path.to_string_lossy().as_ref()) {
            Ok(n) => json!({"ok":true,"count":n,"message":format!("已导出 {} 个账号到 {}", n, path.display())}),
            Err(e) => json!({"ok":false,"message":e}),
        }
    })
    .await
}
#[tauri::command]
pub async fn qoder_import_accounts(path: String) -> Res {
    let path = match path.trim().strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest).to_string_lossy().into_owned(),
        None => path.trim().to_owned(),
    };
    blocking(move || match qoder::qcore::import_accounts(&path) {
        Ok(n) => json!({"ok":n > 0,"message":format!("成功导入/更新 {} 个账号", n)}),
        Err(e) => json!({"ok":false,"message":e}),
    })
    .await
}
#[tauri::command]
pub async fn qoder_list_sessions(edition: String, query: Option<String>, limit: Option<i64>) -> Res {
    blocking(move || qoder::qsession::list_sessions(&edition, query.as_deref().unwrap_or(""), limit.unwrap_or(200))).await?
}
#[tauri::command]
pub async fn qoder_copy_session(edition: String, session_id: String) -> Res {
    blocking(move || qoder::qsession::copy_session(&edition, &session_id)).await?
}
#[tauri::command]
pub async fn qoder_delete_session(edition: String, session_id: String) -> Res {
    blocking(move || qoder::qsession::delete_session(&edition, &session_id)).await?
}
#[tauri::command]
pub async fn qoder_session_links() -> Res {
    blocking(qoder::qsession::session_links).await
}
#[tauri::command]
pub async fn qoder_sync_session_group(group_id: String) -> Res {
    blocking(move || qoder::qsession::sync_group(&group_id)).await?
}
#[tauri::command]
pub async fn qoder_credit_stats(tz_offset: Option<i64>) -> Res {
    blocking(move || qoder::qsession::credit_stats(tz_offset)).await
}
#[tauri::command]
pub async fn qoder_token_stats(edition: String, tz_offset: Option<i64>) -> Res {
    blocking(move || qoder::qstats::token_stats(&edition, tz_offset)).await?
}
#[tauri::command]
pub async fn qoder_login_begin(edition: String) -> Res {
    blocking(move || {
        let (attempt, url) = qoder::qcore::login_begin(&edition)?;
        Ok(json!({"ok":true,"url":url,"attempt":serde_json::to_value(&attempt).map_err(|e| e.to_string())?}))
    })
    .await?
}
#[tauri::command]
pub async fn qoder_login_poll(attempt: Value) -> Res {
    blocking(move || {
        let att: qoder::qcore::LoginAttempt = serde_json::from_value(attempt).map_err(|e| e.to_string())?;
        match qoder::qcore::login_poll(&att)? {
            None => Ok(json!({"done":false})),
            Some(sess) => {
                let (acc, is_new) = qoder::qcore::save_login_account(&att.edition, &sess)?;
                Ok(json!({"done":true,"ok":true,"isNew":is_new,"name":acc.name,"message":format!("{} {}", if is_new {"已添加账号"} else {"已更新账号"}, acc.name)}))
            }
        }
    })
    .await?
}
#[tauri::command]
pub async fn qoder_refresh_account(id: String) -> Res {
    blocking(move || {
        use qoder::qcore as qc;
        let mut accounts = qc::load_accounts();
        let Some(i) = accounts.iter().position(|a| a.id() == id) else {
            return json!({"ok":false,"message":"账号不存在"});
        };
        if accounts[i].refreshToken.is_empty() {
            return json!({"ok":false,"message":"该账号没有 refreshToken，无法刷新（请重新添加或采集）"});
        }
        if qc::refresh_account_token(&mut accounts[i]) {
            let acc = accounts[i].clone();
            qc::persist_and_writeback(&acc);
            json!({"ok":true,"expiresAt":acc.expiresAt,"message":format!("已刷新 {} 的登录凭证，有效期至 {}", acc.name, acc.expiresAt.get(..10).unwrap_or("未知"))})
        } else {
            json!({"ok":false,"message":"刷新失败：refreshToken 已失效或网络错误，请在客户端重新登录"})
        }
    })
    .await
}
#[tauri::command]
pub async fn qoder_clear_claim_log() -> Res {
    blocking(|| {
        qoder::qcore::write_claim_log(&json!([]));
        json!({"ok":true,"message":"领取记录已清除"})
    })
    .await
}
#[tauri::command]
pub async fn qoder_save_gateway(hub: State<'_, Shared>, config: qoder::gateway::Config) -> Res {
    let running = hub.running();
    let q = hub.qoder.clone();
    blocking(move || q.save_config(config, running).map(|_| json!({"ok":true}))).await?
}

// ───────────────────────── Trae ─────────────────────────

pub fn trae_build_state(hub: &Shared) -> Value {
    let data = hub.trae.store.snapshot();
    json!({
        "accounts": data["accounts"].as_array().into_iter().flatten().map(trae::public_account).collect::<Vec<_>>(),
        "events": data["events"],
        "checkinHistory": data["checkinHistory"],
        "clients": hub.trae.clients(),
        "logins": hub.trae.login.records(),
        "busy": hub.trae.busy_ids(),
    })
}
#[tauri::command]
pub async fn trae_state(hub: State<'_, Shared>) -> Res {
    let hub = hub.inner().clone();
    blocking(move || trae_build_state(&hub)).await
}
#[tauri::command]
pub async fn trae_capture(hub: State<'_, Shared>, variant: String) -> Res {
    let t = hub.trae.clone();
    blocking(move || t.capture(&variant)).await?
}
#[tauri::command]
pub async fn trae_login(hub: State<'_, Shared>, variant: String) -> Res {
    hub.trae.login.start(&variant).await
}
#[tauri::command]
pub async fn trae_launch(variant: String) -> Res {
    blocking(move || trae::client::launch(&variant).map(|_| json!({"ok":true}))).await?
}
#[tauri::command]
pub async fn trae_refresh(hub: State<'_, Shared>, id: String) -> Res {
    hub.trae.refresh(&id).await
}
#[tauri::command]
pub async fn trae_refresh_token(hub: State<'_, Shared>, id: String) -> Res {
    hub.trae.refresh_token(&id).await
}
#[tauri::command]
pub async fn trae_checkin(hub: State<'_, Shared>, id: String) -> Res {
    let _busy = hub.trae.exclusive(&id)?;
    let result = hub.trae.checkin(&id, "manual").await?;
    drop(_busy);
    if result["already"] != true {
        let _ = hub.trae.refresh(&id).await;
    }
    Ok(result)
}
#[tauri::command]
pub async fn trae_update_account(hub: State<'_, Shared>, id: String, patch: Value) -> Res {
    let t = hub.trae.clone();
    blocking(move || t.update_account(&id, &patch)).await?
}
#[tauri::command]
pub async fn trae_switch(hub: State<'_, Shared>, id: String) -> Res {
    let t = hub.trae.clone();
    blocking(move || t.switch(&id)).await?
}
#[tauri::command]
pub async fn trae_sync_models(hub: State<'_, Shared>, id: String) -> Res {
    hub.trae.sync_models(&id).await.map(|m| json!(m))
}
#[tauri::command]
pub async fn trae_import(hub: State<'_, Shared>, variant: String, account: Value) -> Res {
    let t = hub.trae.clone();
    blocking(move || t.import(&variant, account)).await?
}
#[tauri::command]
pub async fn trae_delete(hub: State<'_, Shared>, id: String) -> Res {
    let t = hub.trae.clone();
    blocking(move || t.delete(&id).map(|_| json!({"ok":true}))).await?
}

// ── WorkBuddy sessions (WorkBuddy Switch session core) ──
fn wb_variant(v: &str) -> Result<wb_switch_core::modules::variant::WbVariant, String> {
    use wb_switch_core::modules::variant::WbVariant;
    match v {
        "cn" => Ok(WbVariant::Cn),
        "ai" | "intl" => Ok(WbVariant::Ai),
        _ => Err("不支持的 WorkBuddy 版本".into()),
    }
}
/// Switch-library accounts of one variant, plus the client's current login when it is not in the library.
fn wb_session_account_list(variant: wb_switch_core::modules::variant::WbVariant) -> (Vec<Value>, Option<String>) {
    use wb_switch_core::modules::{account, session};
    let current = session::current_user_uid(variant);
    let mut list: Vec<Value> = account::load_accounts()
        .into_iter()
        .filter(|a| account::variant_of(a) == variant && a["uid"].as_str().is_some_and(|s| !s.is_empty()))
        .collect();
    if let Some(uid) = &current {
        if !list.iter().any(|a| a["uid"] == uid.as_str()) {
            list.insert(0, json!({"id": "", "uid": uid, "nickname": "当前登录账号", "variant": variant.as_str()}));
        }
    }
    (list, current)
}
fn wb_session_account(variant: &str, uid: &str) -> Result<Value, String> {
    let (list, _) = wb_session_account_list(wb_variant(variant)?);
    list.into_iter()
        .find(|a| a["uid"] == uid)
        .ok_or_else(|| "账号库中找不到该账号；请先在账号池导入或在客户端登录".into())
}
#[tauri::command]
pub async fn wb_session_accounts(variant: String) -> Res {
    blocking(move || {
        let v = wb_variant(&variant)?;
        let (list, current) = wb_session_account_list(v);
        let accounts: Vec<Value> = list
            .iter()
            .map(|a| {
                json!({
                    "uid": a["uid"],
                    "name": a["nickname"].as_str().or(a["email"].as_str()).or(a["uid"].as_str()).unwrap_or(""),
                    "current": current.as_deref() == a["uid"].as_str(),
                    "inLibrary": a["id"].as_str().is_some_and(|s| !s.is_empty()),
                })
            })
            .collect();
        Ok(json!({
            "variant": v.as_str(),
            "current": current,
            "running": wb_switch_core::modules::process::is_workbuddy_running(v),
            "accounts": accounts,
        }))
    })
    .await?
}
#[tauri::command]
pub async fn wb_list_sessions(variant: String, uid: String) -> Res {
    blocking(move || {
        let v = wb_variant(&variant)?;
        Ok(json!({"sessions": wb_switch_core::modules::session::list_sessions_for_user(v, &uid)}))
    })
    .await?
}
/// Copy sessions to another account (either version). With `quit`, a running
/// target WorkBuddy is closed first; the core refuses to write while it runs.
#[tauri::command]
pub async fn wb_copy_sessions(
    variant: String,
    uid: String,
    target_variant: String,
    target_uid: String,
    session_ids: Vec<String>,
    quit: bool,
) -> Res {
    blocking(move || {
        if session_ids.is_empty() {
            return Err("请选择要复制的会话".to_string());
        }
        let source = wb_session_account(&variant, &uid)?;
        let target = wb_session_account(&target_variant, &target_uid)?;
        let tv = wb_variant(&target_variant)?;
        if quit && wb_switch_core::modules::process::is_workbuddy_running(tv) {
            wb_switch_core::modules::process::close_workbuddy(tv, 15)?;
        }
        let report = wb_switch_core::modules::session::copy_sessions_cross(&source, &target, &session_ids)?;
        agentdock_core::applog::info("workbuddy.session", || {
            (format!("已复制 {} 个会话到 {}", session_ids.len(), target_uid), report.clone())
        });
        Ok(report)
    })
    .await?
}
#[tauri::command]
pub async fn wb_session_groups() -> Res {
    use wb_switch_core::modules::session_groups::{list, SessionClient};
    blocking(|| list(SessionClient::Workbuddy, None)).await?
}
#[tauri::command]
pub async fn wb_session_group_detail(group_id: String) -> Res {
    use wb_switch_core::modules::session_groups::{detail, SessionClient};
    blocking(move || detail(SessionClient::Workbuddy, None, &group_id)).await?
}
#[tauri::command]
pub async fn wb_sync_session_group(group_id: String) -> Res {
    use wb_switch_core::modules::session_groups::{sync_safe_batch, SessionClient};
    blocking(move || sync_safe_batch(SessionClient::Workbuddy, None, &group_id, false)).await?
}
#[tauri::command]
pub async fn wb_unlink_session_group(group_id: String) -> Res {
    use wb_switch_core::modules::session_groups::{delete_group, SessionClient};
    blocking(move || delete_group(SessionClient::Workbuddy, None, &group_id)).await?
}

// ── per-account JSON ──
#[tauri::command]
pub async fn account_json(hub: State<'_, Shared>, provider: String, id: String) -> Res {
    let hub = hub.inner().clone();
    blocking(move || crate::account_json::export(&hub, &provider, &id)).await?
}
#[tauri::command]
pub async fn account_json_save(hub: State<'_, Shared>, provider: String, id: String) -> Res {
    let hub = hub.inner().clone();
    blocking(move || crate::account_json::save(&hub, &provider, &id)).await?
}
#[tauri::command]
pub async fn account_json_import(hub: State<'_, Shared>, provider: String, region: Option<String>, json: Value) -> Res {
    let hub = hub.inner().clone();
    blocking(move || crate::account_json::import(&hub, &provider, region.as_deref(), &json)).await?
}
