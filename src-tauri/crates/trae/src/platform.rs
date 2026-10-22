//! OS integration for the Trae clients (macOS and Windows), ported from
//! Trae Switch `platform.mjs` / `client-process.mjs`: install discovery,
//! data directory, process inspection and control, opening URLs.
use std::path::{Path, PathBuf};
use std::process::Command;

/// One possible installation: app folder, `resources/app` root and executable.
pub struct Candidate {
    pub app: PathBuf,
    pub root: PathBuf,
    pub executable: PathBuf,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn override_path(variant: &str) -> Option<PathBuf> {
    std::env::var_os(if variant == "cn" { "TRAE_CN_PATH" } else { "TRAE_INTL_PATH" }).map(PathBuf::from)
}

/// Client user data: `~/Library/Application Support/<name>` or `%APPDATA%\<name>`.
pub fn data_dir(name: &str) -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join("AppData").join("Roaming"))
            .join(name)
    } else {
        home().join("Library/Application Support").join(name)
    }
}

pub fn candidates(variant: &str, name: &str) -> Vec<Candidate> {
    let mut out = vec![];
    if cfg!(windows) {
        let ov = override_path(variant);
        let ov_exe = ov.as_ref().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))).cloned();
        let mut dirs: Vec<PathBuf> = vec![];
        if let Some(p) = &ov {
            dirs.push(if ov_exe.is_some() { p.parent().map(Path::to_path_buf).unwrap_or_default() } else { p.clone() });
        }
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Local"));
        dirs.push(local.join("Programs").join(name));
        for key in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(key) {
                dirs.push(PathBuf::from(root).join(name));
            }
        }
        for dir in dirs {
            for folder in [dir.clone(), dir.join("current")] {
                out.push(Candidate {
                    root: folder.join("resources").join("app"),
                    executable: ov_exe.clone().unwrap_or_else(|| folder.join(format!("{name}.exe"))),
                    app: folder,
                });
            }
        }
    } else {
        let mut apps: Vec<PathBuf> = override_path(variant).into_iter().collect();
        apps.push(PathBuf::from(format!("/Applications/{name}.app")));
        apps.push(home().join("Applications").join(format!("{name}.app")));
        for app in apps {
            out.push(Candidate {
                root: app.join("Contents/Resources/app"),
                executable: app.join("Contents/MacOS").join(name),
                app,
            });
        }
    }
    out
}

/// On Windows the executable may carry the product's own name.
pub fn resolve_executable(c: &Candidate, product: &serde_json::Value, name: &str) -> Option<PathBuf> {
    if !cfg!(windows) {
        // The bundle's binary is not always named after the app: take the one in Contents/MacOS.
        if c.executable.is_file() {
            return Some(c.executable.clone());
        }
        let dir = c.app.join("Contents/MacOS");
        return std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_file())
            .or(Some(c.executable.clone()));
    }
    [
        Some(c.executable.clone()),
        product["nameShort"].as_str().map(|n| c.app.join(format!("{n}.exe"))),
        Some(c.app.join(format!("{}.exe", product["applicationName"].as_str().unwrap_or("trae")))),
        Some(c.app.join(format!("{name}.exe"))),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_file())
}

#[cfg(windows)]
fn hidden(command: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000)
}

#[cfg(windows)]
fn powershell(script: &str) -> Command {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    let mut c = Command::new(root.join("System32\\WindowsPowerShell\\v1.0\\powershell.exe"));
    c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script]);
    hidden(&mut c);
    c
}

/// A process belonging to one installation; `main` marks the app executable itself.
pub struct Proc {
    pub pid: u32,
    pub main: bool,
}

/// Normalized Windows path for prefix comparison (separators, case, trailing slash).
#[cfg(windows)]
fn norm(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// One system-wide process scan (pid, lowercase name, normalized path), shared
/// for `max_age` so status polling does not launch PowerShell per client.
#[cfg(windows)]
fn scan(max_age: std::time::Duration) -> Result<std::sync::Arc<Vec<(u32, String, Option<String>)>>, String> {
    use std::sync::{Arc, Mutex};
    type Rows = Arc<Vec<(u32, String, Option<String>)>>;
    static CACHE: Mutex<Option<(std::time::Instant, Rows)>> = Mutex::new(None);
    if let Some((at, rows)) = CACHE.lock().unwrap().as_ref() {
        if at.elapsed() < max_age {
            return Ok(rows.clone());
        }
    }
    let script = "$ErrorActionPreference='Stop';[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false);@(Get-CimInstance Win32_Process | Select-Object ProcessId,Name,ExecutablePath) | ConvertTo-Json -Compress";
    let out = powershell(script).output().map_err(|_| "无法确认客户端进程状态")?;
    if !out.status.success() {
        return Err("无法确认客户端进程状态".into());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let raw: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}').trim()).unwrap_or(serde_json::Value::Null);
    let rows = match raw {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Null => vec![],
        one => vec![one],
    };
    let rows: Rows = Arc::new(
        rows.iter()
            .map(|r| {
                (
                    r["ProcessId"].as_u64().unwrap_or(0) as u32,
                    r["Name"].as_str().unwrap_or("").to_lowercase(),
                    r["ExecutablePath"].as_str().map(|p| norm(Path::new(p))),
                )
            })
            .collect(),
    );
    *CACHE.lock().unwrap() = Some((std::time::Instant::now(), rows.clone()));
    Ok(rows)
}

/// Processes inside the installation folder. Errors mean the state is unknown:
/// callers must then refuse to write the login. `fresh` bypasses the shared
/// scan (used before writing a login), otherwise a scan up to 3 s old is reused.
pub fn processes(app: &Path, executable: &Path, fresh: bool) -> Result<Vec<Proc>, String> {
    #[cfg(windows)]
    {
        let rows = scan(std::time::Duration::from_secs(if fresh { 0 } else { 3 }))?;
        let root = format!("{}\\", norm(app));
        let main = norm(executable);
        let exe_name = executable.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let mut found = vec![];
        for (pid, name, path) in rows.iter() {
            match path {
                Some(path) if *pid > 4 && path.starts_with(&root) => found.push(Proc { pid: *pid, main: *path == main }),
                Some(_) => {}
                // A client process whose path cannot be read makes the state unknown.
                None if *name == exe_name => return Err("无法确认客户端可执行文件路径".into()),
                None => {}
            }
        }
        Ok(found)
    }
    #[cfg(not(windows))]
    {
        let _ = (executable, fresh);
        let prefix = format!("{}/Contents/", app.display());
        let out = Command::new("/bin/ps")
            .args(["-axo", "pid=,command="])
            .output()
            .map_err(|_| "无法确认客户端进程状态")?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| {
                let (pid, cmd) = l.trim_start().split_once(' ')?;
                let cmd = cmd.trim_start();
                let pid: u32 = pid.parse().ok().filter(|p| *p > 1)?;
                cmd.starts_with(&prefix).then(|| Proc { pid, main: cmd.starts_with(&format!("{prefix}MacOS/")) })
            })
            .collect())
    }
}

/// Milliseconds between process checks while waiting for a client to exit
/// (a Windows scan launches PowerShell, a macOS `ps` is cheap).
pub const QUIT_POLL_MS: u64 = if cfg!(windows) { 1000 } else { 250 };

/// Ask the app to quit (it saves its own state first).
pub fn request_quit(app: &Path, mains: &[u32]) {
    #[cfg(windows)]
    {
        let _ = app;
        for pid in mains {
            let script = format!("$p=Get-Process -Id {pid} -ErrorAction SilentlyContinue; if($p){{[void]$p.CloseMainWindow()}}");
            let _ = powershell(&script).output();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = mains;
        let script = format!("tell application \"{}\" to quit", app.display());
        let _ = Command::new("/usr/bin/osascript").args(["-e", &script]).output();
    }
}

/// Terminate (`force` = kill) the given processes.
pub fn terminate(pids: &[u32], force: bool) {
    if pids.is_empty() {
        return;
    }
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        for pid in pids {
            let mut c = Command::new(root.join("System32\\taskkill.exe"));
            c.args(["/PID", &pid.to_string(), "/T"]);
            if force {
                c.arg("/F");
            }
            let _ = hidden(&mut c).output();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("/bin/kill")
            .arg(if force { "-KILL" } else { "-TERM" })
            .args(pids.iter().map(u32::to_string))
            .output();
    }
}

pub fn launch(app: &Path, executable: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = app;
        let mut c = Command::new(executable);
        if let Some(dir) = executable.parent() {
            c.current_dir(dir);
        }
        c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        hidden(&mut c).spawn().map(|_| ()).map_err(|_| "无法打开客户端".into())
    }
    #[cfg(not(windows))]
    {
        let _ = executable;
        Command::new("/usr/bin/open").arg("-a").arg(app).status().map(|_| ()).map_err(|_| "无法打开客户端".into())
    }
}

/// Open an HTTP(S) URL in the default browser.
pub fn open_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("只支持 HTTP / HTTPS 链接".into());
    }
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let mut c = Command::new(root.join("System32\\rundll32.exe"));
        c.args(["url.dll,FileProtocolHandler", url]);
        hidden(&mut c).status().map(|_| ()).map_err(|_| "无法打开浏览器".into())
    }
    #[cfg(not(windows))]
    {
        Command::new("/usr/bin/open").arg(url).status().map(|_| ()).map_err(|_| "无法打开浏览器".into())
    }
}

pub fn hostname() -> String {
    if cfg!(windows) {
        return std::env::var("COMPUTERNAME").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "localhost".into());
    }
    Command::new("/bin/hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".into())
}
