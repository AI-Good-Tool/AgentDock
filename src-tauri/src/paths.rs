//! Data directory. AgentDock keeps everything under its own directory and
//! never reads or copies the standalone WorkBuddy API, Qoder Switch or Trae
//! Switch data.
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("AGENTDOCK_HOME") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agentdock")
}

fn private_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|_| format!("无法创建数据目录 {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Create `dir` and the per-provider subdirectories (owner-only).
pub fn prepare(dir: &Path) -> Result<(), String> {
    private_dir(dir)?;
    for sub in ["workbuddy", "qoder", "trae"] {
        private_dir(&dir.join(sub))?;
    }
    Ok(())
}
