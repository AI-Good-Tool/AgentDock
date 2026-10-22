//! AgentDock: one local workspace and model gateway for WorkBuddy, Qoder and
//! Trae (domestic and international editions).
pub mod account_json;
pub mod api_key;
pub mod app;
pub mod app_update;
pub mod applog;
pub mod backup;
pub mod catalog;
pub mod checkin_schedule;
pub mod commands;
pub mod logs;
pub mod paths;
pub mod scheduler;
pub mod server;
pub mod settings;
pub mod system_checkin;
pub mod tasks;
#[cfg(test)]
mod tests;

use std::sync::Arc;

/// Open the data directory and every provider.
pub fn open_hub() -> Result<app::Shared, String> {
    let dir = paths::data_dir();
    paths::prepare(&dir)?;
    app::Hub::open(&dir)
}

pub fn run_desktop() {
    let hub = open_hub().expect("初始化 AgentDock 数据目录失败");
    tauri::Builder::default()
        .manage(hub.clone())
        .manage(app_update::Updater::default())
        .setup(move |_| {
            let hub = hub.clone();
            tauri::async_runtime::spawn(async move { tasks::spawn_all(Arc::clone(&hub)).await });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_snapshot,
            commands::gateway_toggle,
            commands::save_settings,
            commands::generate_api_key,
            commands::set_api_key,
            commands::pool_snapshot,
            commands::set_account_enabled,
            commands::clear_account_runtime,
            commands::catalog_snapshot,
            commands::sync_catalog,
            commands::save_route,
            commands::create_route,
            commands::reset_route,
            commands::request_logs,
            commands::clear_logs,
            commands::checkin_run_now,
            commands::open_about_link,
            app_update::app_update_check,
            app_update::app_update_download,
            app_update::app_update_cancel_download,
            app_update::app_update_install,
            app_update::app_update_confirm_restart,
            app_update::app_update_take_install_error,
            commands::account_json,
            commands::account_json_save,
            commands::account_json_import,
            commands::wb_session_accounts,
            commands::wb_list_sessions,
            commands::wb_copy_sessions,
            commands::wb_session_groups,
            commands::wb_session_group_detail,
            commands::wb_sync_session_group,
            commands::wb_unlink_session_group,
            commands::app_logs,
            commands::clear_app_logs,
            commands::backup_export,
            commands::backup_inspect,
            commands::backup_import,
            commands::proxy_statistics,
            commands::open_url,
            commands::reveal_data_dir,
            commands::launchd_status,
            commands::launchd_install,
            commands::wb_import_accounts,
            commands::wb_oauth_start,
            commands::wb_oauth_poll,
            commands::wb_open_oauth_link,
            commands::wb_set_auto_checkin,
            commands::wb_set_checkin_config,
            commands::wb_set_account_checkin,
            commands::wb_update_display,
            commands::wb_set_runtime_settings,
            commands::wb_refresh_statuses,
            commands::wb_refresh_credits,
            commands::wb_switch_client,
            commands::wb_checkin,
            commands::wb_credit_statistics,
            commands::wb_token_statistics,
            commands::wb_proxy_statistics,
            commands::wb_refresh_official_statistics,
            commands::wb_prepare_device,
            commands::qoder_state,
            commands::qoder_capture,
            commands::qoder_switch_account,
            commands::qoder_claim,
            commands::qoder_credits,
            commands::qoder_set_note,
            commands::qoder_delete_account,
            commands::qoder_set_auto_claim,
            commands::qoder_open_app,
            commands::qoder_export_accounts,
            commands::qoder_import_accounts,
            commands::qoder_list_sessions,
            commands::qoder_copy_session,
            commands::qoder_delete_session,
            commands::qoder_session_links,
            commands::qoder_sync_session_group,
            commands::qoder_credit_stats,
            commands::qoder_token_stats,
            commands::qoder_login_begin,
            commands::qoder_login_poll,
            commands::qoder_refresh_account,
            commands::qoder_clear_claim_log,
            commands::qoder_save_gateway,
            commands::trae_state,
            commands::trae_capture,
            commands::trae_login,
            commands::trae_launch,
            commands::trae_refresh,
            commands::trae_refresh_token,
            commands::trae_checkin,
            commands::trae_update_account,
            commands::trae_switch,
            commands::trae_sync_models,
            commands::trae_import,
            commands::trae_delete,
        ])
        .run(tauri::generate_context!())
        .expect("启动 AgentDock 失败");
}
