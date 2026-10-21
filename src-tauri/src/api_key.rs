//! Local proxy credentials only. Never changes official account credentials.
use std::{path::Path, sync::Mutex};
use uuid::Uuid;

pub fn generate() -> String {
    format!("ad-{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
pub fn validate(value: &str) -> Result<(), String> {
    if !(16..=256).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("API Key 必须为 16–256 个非空白 ASCII 字符，不能包含空格或换行".into());
    }
    Ok(())
}
pub struct LocalKey {
    current: Mutex<String>,
    writer: Mutex<()>,
}
impl LocalKey {
    pub fn new(value: String) -> Self {
        Self {
            current: Mutex::new(value),
            writer: Mutex::new(()),
        }
    }
    pub fn get(&self) -> String {
        self.current.lock().unwrap().clone()
    }
    pub fn replace(&self, path: &Path, value: String, expected: &str) -> Result<(), String> {
        validate(&value)?;
        let _writer = self.writer.lock().unwrap();
        if self.get() != expected {
            return Err("API Key 已被其他操作修改，请刷新后重新编辑".into());
        }
        // Disk failure leaves the live key unchanged. Do not print key-bearing I/O errors.
        write_atomic(path, &value)?;
        *self.current.lock().unwrap() = value;
        Ok(())
    }
}
fn write_atomic(path: &Path, value: &str) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("无法定位本地密钥目录")?;
    std::fs::create_dir_all(parent).map_err(|_| "无法创建本地密钥目录")?;
    let temp = parent.join(format!(".api-key-{}.tmp", Uuid::new_v4().simple()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temp).map_err(|_| "无法创建临时密钥文件")?;
        file.write_all(value.as_bytes())
            .map_err(|_| "无法写入本地密钥")?;
        file.sync_all().map_err(|_| "无法同步本地密钥")?;
        std::fs::rename(&temp, path).map_err(|_| "无法替换本地密钥文件")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn random_and_validation() {
        let a = generate();
        let b = generate();
        assert_ne!(a, b);
        assert_eq!(a.len(), 67);
        validate(&a).unwrap();
        for s in [
            "short",
            "123456789012345 ",
            "123456789012345\n",
            "123456789012345中",
        ] {
            assert!(validate(s).is_err());
        }
        assert!(validate(&"x".repeat(257)).is_err());
    }
    #[test]
    fn replacement_persists_with_private_permissions_and_rejects_stale_edits() {
        let root = std::env::temp_dir().join(format!("agentdock-key-test-{}", Uuid::new_v4()));
        let path = root.join("api-key");
        let old = generate();
        let new = generate();
        let key = LocalKey::new(old.clone());
        key.replace(&path, new.clone(), &old).unwrap();
        assert_eq!(key.get(), new);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), new);
        assert!(key.replace(&path, generate(), &old).is_err());
        assert_eq!(key.get(), new);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn disk_failure_never_publishes_new_key() {
        let old = generate();
        let key = LocalKey::new(old.clone());
        let root = std::env::temp_dir().join(format!("agentdock-key-fail-{}", Uuid::new_v4()));
        std::fs::write(&root, b"not a directory").unwrap();
        assert!(key
            .replace(&root.join("api-key"), generate(), &old)
            .is_err());
        assert_eq!(key.get(), old);
        std::fs::remove_file(root).unwrap();
    }
}
