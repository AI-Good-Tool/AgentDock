//! Manual GitHub release checks and verified updates. Never sends official account credentials.
//! DMG staging/rollback adapts the codex-account-switcher / WorkBuddy API implementation.
use futures_util::StreamExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
#[cfg(windows)]
use std::{os::windows::process::CommandExt, process::Stdio};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::{io::AsyncWriteExt, sync::Notify};

#[cfg(target_os = "macos")]
mod macos;
pub const RELEASES_URL: &str = "https://github.com/AI-Good-Tool/AgentDock/releases";
const LATEST_API: &str = "https://api.github.com/repos/AI-Good-Tool/AgentDock/releases/latest";
const DOWNLOAD_PROGRESS_EVENT: &str = "agentdock-update-progress";
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
#[cfg(target_os = "macos")]
const CLEANUP_MARKER_FILE: &str = ".cleanup-after-install";

#[derive(Default)]
pub struct Updater(Mutex<Control>);
#[derive(Default)]
struct Control {
    active: Option<Arc<Cancel>>,
    installing: bool,
    verified: Option<Receipt>,
}
#[derive(Default)]
struct Cancel {
    requested: AtomicBool,
    notify: Notify,
}
impl Cancel {
    fn cancel(&self) {
        self.requested.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }
    async fn run<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        if self.requested.load(Ordering::SeqCst) {
            return Err("下载已取消".into());
        }
        tokio::select! { biased;
            _ = self.notify.notified() => Err("下载已取消".into()),
            result = future => {
                if self.requested.load(Ordering::SeqCst) { Err("下载已取消".into()) } else { result }
            }
        }
    }
}
#[derive(Clone)]
struct Receipt {
    path: PathBuf,
    version: Version,
    sha256: String,
    size: u64,
}
#[derive(Clone, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}
#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    current_version: String,
    latest_version: Option<String>,
    release_url: String,
    target: String,
    asset_name: Option<String>,
    asset_size: Option<u64>,
    notes: String,
    has_update: bool,
    can_download: bool,
    reason: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResult {
    version: String,
    asset_name: String,
    path: String,
    size_bytes: u64,
    sha256: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    status: String,
    version: String,
    asset_name: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    message: Option<String>,
}

fn parse_version(raw: &str) -> Result<Version, String> {
    if raw.len() > 128 {
        return Err("版本号过长".into());
    }
    Version::parse(raw.trim().trim_start_matches(['v', 'V']))
        .map_err(|_| "发布版本号不是有效的语义版本".into())
}
fn target_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}
fn current_target() -> Option<&'static str> {
    target_for(std::env::consts::OS, std::env::consts::ARCH)
}
fn expected_name(version: &Version, target: &str) -> String {
    format!(
        "AgentDock_{version}_{target}.{}",
        if target.ends_with("windows-msvc") {
            "exe"
        } else {
            "dmg"
        }
    )
}
fn digest_value(raw: Option<&str>) -> Option<String> {
    let value = raw?.strip_prefix("sha256:")?;
    (value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}
fn selected_asset<'a>(release: &'a Release, version: &Version, target: &str) -> Option<&'a Asset> {
    let name = expected_name(version, target);
    let mut matches = release.assets.iter().filter(|a| a.name == name);
    let asset = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(asset)
}
fn build_info(
    release: Option<&Release>,
    current: &Version,
    target: Option<&str>,
) -> Result<UpdateInfo, String> {
    let mut info = UpdateInfo {
        current_version: current.to_string(),
        latest_version: None,
        release_url: RELEASES_URL.into(),
        target: target.unwrap_or("unsupported").into(),
        asset_name: None,
        asset_size: None,
        notes: String::new(),
        has_update: false,
        can_download: false,
        reason: "尚未发布正式 Release；Actions 构建产物不作为在线更新源".into(),
    };
    let Some(release) = release else {
        return Ok(info);
    };
    let version = parse_version(&release.tag_name)?;
    if release.draft || release.prerelease || !version.pre.is_empty() {
        return Err("在线更新只接受正式版本，不安装草稿或预发布版本".into());
    }
    info.latest_version = Some(version.to_string());
    info.notes = agentdock_core::diagnostics::bounded(release.body.as_deref().unwrap_or(""), 16000);
    info.has_update = version.cmp_precedence(current).is_gt();
    if !info.has_update {
        info.reason = if version.cmp_precedence(current).is_lt() {
            "当前版本高于已发布版本，不会降级"
        } else {
            "当前已是最新正式版本"
        }
        .into();
        return Ok(info);
    }
    let Some(target) = target else {
        info.reason = "当前系统或架构暂不支持在线更新".into();
        return Ok(info);
    };
    let Some(asset) = selected_asset(release, &version, target) else {
        info.reason = "该版本缺少唯一匹配当前平台的安装包，请前往 Releases 查看".into();
        return Ok(info);
    };
    info.asset_name = Some(asset.name.clone());
    info.asset_size = Some(asset.size);
    info.can_download = asset.size > 0
        && asset.size <= MAX_INSTALLER_BYTES
        && digest_value(asset.digest.as_deref()).is_some()
        && validate_release_download_url(&asset.browser_download_url).is_ok();
    info.reason = if info.can_download {
        "已找到匹配平台且提供 SHA256 的安装包"
    } else {
        "安装包缺少有效 SHA256、大小异常或来源不受信任，已禁止在线安装"
    }
    .into();
    Ok(info)
}
fn safe_file_name(name: &str) -> Result<String, String> {
    if name.is_empty()
        || name.len() > 180
        || name == "."
        || name == ".."
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err("更新文件名包含非法字符或路径".into());
    }
    Ok(name.into())
}
fn validate_release_download_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "更新地址无效")?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url
            .path()
            .starts_with("/AI-Good-Tool/AgentDock/releases/download/")
    {
        return Err("只接受本项目 GitHub Release 的 HTTPS 安装包".into());
    }
    Ok(url)
}
fn redirect_allowed(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
fn download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .user_agent("AgentDock-Updater")
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !redirect_allowed(attempt.url()) {
                attempt.error("不允许的更新下载重定向")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "初始化下载客户端失败".into())
}
async fn fetch_latest() -> Result<Option<Release>, String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .user_agent("AgentDock-Updater")
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "初始化版本检查失败")?;
    let response = client
        .get(LATEST_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| "无法连接 GitHub，请检查网络后重试")?;
    match response.status().as_u16() {
        404 => return Ok(None),
        403 | 429 => return Err("GitHub 限流或访问受限，请稍后重试或前往发布页".into()),
        200 => {}
        _ => return Err("GitHub 未返回有效的正式发布信息".into()),
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "读取版本信息失败")?;
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("版本元数据超过安全上限".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "发布信息格式无效".into())
}
#[tauri::command]
pub async fn app_update_check() -> Result<UpdateInfo, String> {
    let release = fetch_latest().await?;
    build_info(
        release.as_ref(),
        &parse_version(env!("CARGO_PKG_VERSION"))?,
        current_target(),
    )
}
fn updates_dir() -> Result<PathBuf, String> {
    Ok(dirs::data_local_dir()
        .ok_or("无法定位用户数据目录")?
        .join("AgentDock/updates"))
}
fn begin_download(state: &Updater) -> Result<Arc<Cancel>, String> {
    let mut c = state.0.lock().map_err(|_| "更新状态锁异常")?;
    if c.active.is_some() || c.installing {
        return Err("已有更新操作正在进行".into());
    }
    let cancel = Arc::new(Cancel::default());
    c.verified = None;
    c.active = Some(cancel.clone());
    Ok(cancel)
}
fn emit_progress(
    app: &AppHandle,
    status: &str,
    version: &str,
    asset_name: &str,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    message: Option<&str>,
) {
    let _ = app.emit(
        DOWNLOAD_PROGRESS_EVENT,
        Progress {
            status: status.into(),
            version: version.into(),
            asset_name: asset_name.into(),
            downloaded_bytes,
            total_bytes,
            message: message.map(str::to_owned),
        },
    );
}
#[tauri::command]
pub fn app_update_cancel_download(state: State<'_, Updater>) -> Result<(), String> {
    if let Some(active) = &state.0.lock().map_err(|_| "更新状态锁异常")?.active {
        active.cancel();
    }
    Ok(())
}
async fn download(
    app: &AppHandle,
    cancel: &Cancel,
    expected: &Version,
) -> Result<(DownloadResult, Receipt), String> {
    emit_progress(
        app,
        "checking",
        &expected.to_string(),
        "",
        0,
        None,
        Some("正在重新确认发布信息"),
    );
    let release = cancel
        .run(fetch_latest())
        .await?
        .ok_or("尚未发布正式版本")?;
    let info = build_info(
        Some(&release),
        &parse_version(env!("CARGO_PKG_VERSION"))?,
        current_target(),
    )?;
    if info.latest_version.as_deref() != Some(&expected.to_string()) {
        return Err("发布版本已变化，请重新检查更新".into());
    }
    if !info.has_update || !info.can_download {
        return Err(info.reason);
    }
    let asset = selected_asset(
        &release,
        expected,
        current_target().ok_or("不支持的更新平台")?,
    )
    .ok_or("缺少安装包")?;
    let expected_digest = digest_value(asset.digest.as_deref()).ok_or("发布未提供 SHA256")?;
    let name = safe_file_name(&asset.name)?;
    let root = updates_dir()?.join(expected.to_string());
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|_| "无法创建更新缓存目录")?;
    let temp = tempfile::NamedTempFile::new_in(&root).map_err(|_| "无法创建临时下载文件")?;
    let mut file =
        tokio::fs::File::from_std(temp.as_file().try_clone().map_err(|_| "无法打开下载文件")?);
    let response = cancel
        .run(async {
            download_client()?
                .get(validate_release_download_url(&asset.browser_download_url)?)
                .send()
                .await
                .map_err(|_| "下载安装包失败，请检查网络后重试".into())
        })
        .await?;
    if !response.status().is_success() {
        return Err("下载服务器拒绝请求".into());
    }
    if response.content_length().is_some_and(|n| n != asset.size) {
        return Err("安装包响应大小与发布信息不符".into());
    }
    let mut stream = response.bytes_stream();
    let mut sha = Sha256::new();
    let mut size = 0u64;
    let mut last = 0u64;
    emit_progress(
        app,
        "downloading",
        &expected.to_string(),
        &name,
        0,
        Some(asset.size),
        None,
    );
    loop {
        let next = cancel
            .run(async {
                stream
                    .next()
                    .await
                    .transpose()
                    .map_err(|_| "安装包下载中断".into())
            })
            .await?;
        let Some(bytes) = next else { break };
        size = size.checked_add(bytes.len() as u64).ok_or("安装包过大")?;
        if size > asset.size || size > MAX_INSTALLER_BYTES {
            return Err("安装包超过发布大小或安全上限".into());
        }
        sha.update(&bytes);
        file.write_all(&bytes)
            .await
            .map_err(|_| "写入下载缓存失败")?;
        if size == asset.size || size - last >= 256 * 1024 {
            emit_progress(
                app,
                "downloading",
                &expected.to_string(),
                &name,
                size,
                Some(asset.size),
                None,
            );
            last = size;
        }
    }
    if size != asset.size || format!("{:x}", sha.finalize()) != expected_digest {
        return Err("安装包大小或 SHA256 校验失败，已拒绝保存和安装".into());
    }
    file.sync_all().await.map_err(|_| "保存更新缓存失败")?;
    drop(file);
    if cancel.requested.load(Ordering::SeqCst) {
        return Err("下载已取消".into());
    }
    let path = root.join(&name);
    temp.persist(&path).map_err(|_| "无法保存已校验安装包")?;
    let path = path.canonicalize().map_err(|_| "无法确认更新文件位置")?;
    let receipt = Receipt {
        path: path.clone(),
        version: expected.clone(),
        sha256: expected_digest.clone(),
        size,
    };
    Ok((
        DownloadResult {
            version: expected.to_string(),
            asset_name: name,
            path: path.to_string_lossy().into(),
            size_bytes: size,
            sha256: expected_digest,
        },
        receipt,
    ))
}
#[tauri::command]
pub async fn app_update_download(
    app: AppHandle,
    expected_version: String,
    confirmed: bool,
) -> Result<DownloadResult, String> {
    if !confirmed {
        return Err("下载更新需要用户确认".into());
    }
    let version = parse_version(&expected_version)?;
    let state = app.state::<Updater>();
    let cancel = begin_download(&state)?;
    let result = download(&app, &cancel, &version).await;
    let mut c = state.0.lock().map_err(|_| "更新状态锁异常")?;
    c.active = None;
    match result {
        Ok((value, receipt)) => {
            c.verified = Some(receipt);
            emit_progress(
                &app,
                "completed",
                &value.version,
                &value.asset_name,
                value.size_bytes,
                Some(value.size_bytes),
                Some("SHA256 校验通过，准备安装更新"),
            );
            Ok(value)
        }
        Err(error) => {
            emit_progress(
                &app,
                if error == "下载已取消" {
                    "cancelled"
                } else {
                    "failed"
                },
                "",
                "",
                0,
                None,
                Some(&error),
            );
            Err(error)
        }
    }
}
fn verify_receipt(receipt: &Receipt, current: &Version) -> Result<(), String> {
    if !receipt.version.cmp_precedence(current).is_gt() {
        return Err("更新版本必须高于当前版本，禁止降级".into());
    }
    if receipt.path.canonicalize().map_err(|_| "更新文件不存在")? != receipt.path {
        return Err("更新文件路径已变化".into());
    }
    let mut file = fs::File::open(&receipt.path).map_err(|_| "无法读取更新文件")?;
    if file.metadata().map_err(|_| "无法读取更新文件信息")?.len() != receipt.size
        || receipt.size > MAX_INSTALLER_BYTES
    {
        return Err("更新文件大小已变化".into());
    }
    let mut sha = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut count = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(|_| "重新校验更新文件失败")?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > receipt.size {
            return Err("更新文件大小已变化".into());
        }
        sha.update(&buffer[..n]);
    }
    if count != receipt.size || format!("{:x}", sha.finalize()) != receipt.sha256 {
        return Err("安装包已被修改，SHA256 校验失败".into());
    }
    Ok(())
}
#[cfg(target_os = "macos")]
pub(super) fn system_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "macos")]
    {
        cmd.env_remove("DYLD_INSERT_LIBRARIES")
            .env_remove("DYLD_LIBRARY_PATH");
    }
    cmd
}
async fn drain_gateway(app: &AppHandle) {
    let hub = app.state::<crate::app::Shared>().inner().clone();
    crate::server::stop(&hub);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !hub.active.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
#[tauri::command]
pub async fn app_update_install(app: AppHandle, confirmed: bool) -> Result<(), String> {
    if !confirmed {
        return Err("安装更新并退出应用需要明确确认".into());
    }
    let receipt = {
        let state = app.state::<Updater>();
        let mut c = state.0.lock().map_err(|_| "更新状态锁异常")?;
        if c.active.is_some() || c.installing {
            return Err("已有更新操作正在进行".into());
        }
        let receipt = c.verified.clone().ok_or("请先在应用内下载并校验安装包")?;
        c.installing = true;
        receipt
    };
    // The macOS helper waits for this process to exit, so stage before stopping
    // the service. Windows installers may terminate the app themselves.
    #[cfg(windows)]
    let was_running = {
        let running = app.state::<crate::app::Shared>().running();
        drain_gateway(&app).await;
        running
    };
    let handle = app.clone();
    let result = tokio::task::spawn_blocking(move || {
        verify_receipt(&receipt, &parse_version(env!("CARGO_PKG_VERSION"))?)?;
        prepare_install(&handle, &receipt.path)
    })
    .await
    .map_err(|_| "准备更新安装任务失败".to_string())
    .and_then(|r| r);
    if result.is_ok() {
        #[cfg(not(windows))]
        drain_gateway(&app).await;
        app.exit(0);
    }
    if result.is_err() {
        app.state::<Updater>()
            .0
            .lock()
            .map_err(|_| "更新状态锁异常")?
            .installing = false;
        #[cfg(windows)]
        if was_running {
            let hub = app.state::<crate::app::Shared>().inner().clone();
            crate::server::start(hub).await?;
        }
    }
    result
}
fn prepare_install(app: &AppHandle, installer: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::prepare_install_and_restart(app, installer, &updates_dir()?)
    }
    #[cfg(windows)]
    {
        let _ = app;
        if installer.extension().and_then(|x| x.to_str()) != Some("exe") {
            return Err("Windows 更新只支持本项目 EXE 安装程序".into());
        }
        Command::new(installer)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x00000208)
            .spawn()
            .map_err(|_| "无法启动安装程序，当前应用保持运行")?;
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (app, installer);
        Err("当前系统不支持应用内安装".into())
    }
}
fn install_error_path() -> Result<PathBuf, String> {
    Ok(updates_dir()?.with_file_name(".last-app-update-error"))
}
#[tauri::command]
pub fn app_update_take_install_error() -> Result<Option<String>, String> {
    let path = install_error_path()?;
    match fs::read_to_string(&path) {
        Ok(s) => {
            fs::remove_file(path).map_err(|_| "无法清理更新错误标记")?;
            Ok(Some(agentdock_core::diagnostics::bounded(&s, 2000)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("无法读取上次更新状态".into()),
    }
}
#[tauri::command]
pub fn app_update_confirm_restart(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::confirm_update_restart(&app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn version(s: &str) -> Version {
        parse_version(s).unwrap()
    }
    fn release() -> Release {
        let v = version("0.2.0");
        Release { tag_name: "v0.2.0".into(), draft: false, prerelease: false, body: None,
            assets: vec![Asset { name: expected_name(&v, "aarch64-apple-darwin"),
                browser_download_url: "https://github.com/AI-Good-Tool/AgentDock/releases/download/v0.2.0/test.dmg".into(),
                size: 100, digest: Some(format!("sha256:{}", "a".repeat(64))) }] }
    }
    #[test]
    fn missing_release_is_not_latest() {
        let info = build_info(None, &version("0.1.0"), current_target()).unwrap();
        assert!(info.latest_version.is_none());
        assert!(!info.can_download);
        assert!(info.reason.contains("尚未发布"));
    }
    #[test]
    fn exact_platform_asset_and_digest_required() {
        let mut r = release();
        assert!(
            build_info(Some(&r), &version("0.1.0"), Some("aarch64-apple-darwin"))
                .unwrap()
                .can_download
        );
        for target in ["x86_64-apple-darwin", "x86_64-pc-windows-msvc"] {
            assert!(
                !build_info(Some(&r), &version("0.1.0"), Some(target))
                    .unwrap()
                    .can_download
            );
        }
        r.assets.push(r.assets[0].clone());
        assert!(selected_asset(&r, &version("0.2.0"), "aarch64-apple-darwin").is_none());
        r.assets.pop();
        r.assets[0].digest = None;
        assert!(
            !build_info(Some(&r), &version("0.1.0"), Some("aarch64-apple-darwin"))
                .unwrap()
                .can_download
        );
        r.assets[0].digest = Some(format!("sha256:{}", "a".repeat(64)));
        r.assets[0].size = MAX_INSTALLER_BYTES + 1;
        assert!(
            !build_info(Some(&r), &version("0.1.0"), Some("aarch64-apple-darwin"))
                .unwrap()
                .can_download
        );
    }
    #[test]
    fn no_downgrade_prerelease_or_build_metadata_update() {
        let mut r = release();
        for current in ["0.2.0", "0.3.0", "0.2.0+local"] {
            assert!(
                !build_info(Some(&r), &version(current), current_target())
                    .unwrap()
                    .has_update
            );
        }
        r.prerelease = true;
        assert!(build_info(Some(&r), &version("0.1.0"), current_target()).is_err());
        r.prerelease = false;
        r.tag_name = "v0.3.0-rc.1".into();
        assert!(build_info(Some(&r), &version("0.1.0"), current_target()).is_err());
    }
    #[test]
    fn rejects_untrusted_sources_and_paths() {
        for url in [
            "http://github.com/AI-Good-Tool/AgentDock/releases/download/v1/a",
            "https://github.com/evil/repo/releases/download/v1/a",
            "https://github.com@evil.example/AI-Good-Tool/AgentDock/releases/download/v1/a",
            "https://github.com/AI-Good-Tool/AgentDock/releases/download/v1/a?token=x",
        ] {
            assert!(validate_release_download_url(url).is_err(), "{url}");
        }
        for name in ["../a", "a/b", "a\\b", "..", "a;echo", "C:a"] {
            assert!(safe_file_name(name).is_err());
        }
        for url in [
            "https://localhost/a",
            "http://release-assets.githubusercontent.com/a",
            "https://github.com.evil.example/a",
        ] {
            assert!(!redirect_allowed(&url.parse().unwrap()));
        }
        assert!(redirect_allowed(
            &"https://release-assets.githubusercontent.com/a?signature=x"
                .parse()
                .unwrap()
        ));
        assert_eq!(target_for("macos", "aarch64"), Some("aarch64-apple-darwin"));
        assert_eq!(
            target_for("windows", "x86_64"),
            Some("x86_64-pc-windows-msvc")
        );
        assert_eq!(target_for("windows", "aarch64"), None);
    }
    #[test]
    fn installation_rehashes_download_and_refuses_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.dmg");
        fs::write(&path, b"fixture").unwrap();
        let receipt = Receipt {
            path: path.canonicalize().unwrap(),
            version: version("0.2.0"),
            sha256: format!("{:x}", Sha256::digest(b"fixture")),
            size: 7,
        };
        assert!(verify_receipt(&receipt, &version("0.1.0")).is_ok());
        assert!(verify_receipt(&receipt, &version("0.2.0")).is_err());
        fs::write(&path, b"changed").unwrap();
        assert!(verify_receipt(&receipt, &version("0.1.0")).is_err());
    }
    #[tokio::test]
    async fn cancel_before_and_during_wait_and_single_flight() {
        let state = Updater::default();
        let cancel = begin_download(&state).unwrap();
        assert!(begin_download(&state).is_err());
        cancel.cancel();
        assert!(cancel.run(async { Ok(1) }).await.is_err());
        let cancel = Arc::new(Cancel::default());
        let c = cancel.clone();
        let task =
            tokio::spawn(async move { c.run(std::future::pending::<Result<(), String>>()).await });
        cancel.cancel();
        assert!(tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
    }
}
