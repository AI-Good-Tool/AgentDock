//! Encrypted Trae account vault (AES-256-GCM). File layout matches the
//! standalone Trae Switch: iv(12) | tag(16) | ciphertext, key in `vault.key`.
use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub struct Store {
    pub dir: PathBuf,
    file: PathBuf,
    key: [u8; 32],
    data: Mutex<Value>,
    writer: Mutex<()>,
}

fn default_data() -> Value {
    json!({
        "accounts": [],
        "requests": [],
        "events": [],
        "checkinHistory": [],
        "settings": {}
    })
}

impl Store {
    pub fn open(dir: &Path) -> Result<Arc<Self>, String> {
        crate::client::write_private_dir(dir)?;
        let key_file = dir.join("vault.key");
        if !key_file.exists() {
            let key: [u8; 32] = rand::random();
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            use std::io::Write;
            options
                .open(&key_file)
                .and_then(|mut f| f.write_all(&key))
                .map_err(|_| "无法创建账号库密钥")?;
        }
        let raw = fs::read(&key_file).map_err(|_| "无法读取账号库密钥")?;
        let key: [u8; 32] = raw.try_into().map_err(|_| "账号库密钥格式无效")?;
        let file = dir.join("vault.enc");
        let mut data = if file.exists() {
            let bytes = fs::read(&file).map_err(|_| "无法读取账号库")?;
            decrypt(&key, &bytes)?
        } else {
            default_data()
        };
        for field in ["accounts", "requests", "events", "checkinHistory"] {
            if !data[field].is_array() {
                data[field] = json!([]);
            }
        }
        if !data["settings"].is_object() {
            data["settings"] = json!({});
        }
        // A request still pending at shutdown can never complete.
        for record in data["requests"].as_array_mut().unwrap() {
            if record["status"] == "pending" {
                record["status"] = json!("cancelled");
                record["error"] = json!("应用退出时请求尚未完成");
                record["errorCode"] = json!("app_restarted");
            }
        }
        Ok(Arc::new(Self {
            dir: dir.to_owned(),
            file,
            key,
            data: Mutex::new(data),
            writer: Mutex::new(()),
        }))
    }
    pub fn snapshot(&self) -> Value {
        self.data.lock().unwrap().clone()
    }
    /// Apply a change and persist it; on a write failure the live state is unchanged.
    pub fn update<T>(&self, change: impl FnOnce(&mut Value) -> Result<T, String>) -> Result<T, String> {
        let _writer = self.writer.lock().unwrap();
        let mut next = self.data.lock().unwrap().clone();
        let result = change(&mut next)?;
        self.write(&next)?;
        *self.data.lock().unwrap() = next;
        Ok(result)
    }
    fn write(&self, data: &Value) -> Result<(), String> {
        let bytes = encrypt(&self.key, &serde_json::to_vec(data).unwrap())?;
        let tmp = self.file.with_extension("enc.tmp");
        crate::client::write_private(&tmp, &bytes)?;
        fs::rename(&tmp, &self.file).map_err(|_| "无法保存账号库".into())
    }
    pub fn account(&self, id: &str) -> Result<Value, String> {
        self.data.lock().unwrap()["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .cloned()
            .ok_or_else(|| "账号不存在".into())
    }
    pub fn accounts(&self) -> Vec<Value> {
        self.data.lock().unwrap()["accounts"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    /// Merge fields into a stored account (by id); unknown ids fail.
    pub fn patch_account(&self, id: &str, patch: impl FnOnce(&mut Value)) -> Result<Value, String> {
        self.update(|data| {
            let account = data["accounts"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|a| a["id"] == id)
                .ok_or("账号不存在")?;
            patch(account);
            Ok(account.clone())
        })
    }
    pub fn remove_account(&self, id: &str) -> Result<(), String> {
        self.update(|data| {
            let accounts = data["accounts"].as_array_mut().unwrap();
            let before = accounts.len();
            accounts.retain(|a| a["id"] != id);
            if accounts.len() == before {
                return Err("账号不存在".into());
            }
            Ok(())
        })
    }
    /// Insert or update by (variant, userId); keeps the existing local id and settings.
    pub fn upsert(&self, mut account: Value) -> Result<Value, String> {
        let user = &account["user"];
        if !user["userId"].as_str().is_some_and(|s| !s.is_empty())
            || !user["token"].as_str().is_some_and(|s| !s.is_empty())
            || !user["account"].is_object()
        {
            return Err("账号数据缺少有效 userId、token 或 account".into());
        }
        if !user["expiredAt"]
            .as_str()
            .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
        {
            return Err("账号缺少有效的 expiredAt 到期时间".into());
        }
        self.update(|data| {
            let accounts = data["accounts"].as_array_mut().unwrap();
            if let Some(old) = accounts.iter_mut().find(|x| {
                x["variant"] == account["variant"] && x["user"]["userId"] == account["user"]["userId"]
            }) {
                let id = old["id"].clone();
                for (k, v) in account.as_object().unwrap() {
                    old[k] = v.clone();
                }
                old["id"] = id;
                Ok(old.clone())
            } else {
                account["id"] = json!(uuid::Uuid::new_v4().to_string());
                accounts.push(account.clone());
                Ok(account)
            }
        })
    }
    pub fn event(&self, kind: &str, message: &str) {
        let _ = self.update(|data| {
            let events = data["events"].as_array_mut().unwrap();
            events.insert(
                0,
                json!({"id":uuid::Uuid::new_v4().to_string(),"time":chrono::Utc::now().to_rfc3339(),"type":kind,"message":message}),
            );
            events.truncate(200);
            Ok(())
        });
    }
}

fn encrypt(key: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>, String> {
    let iv: [u8; 12] = rand::random();
    let sealed = Aes256Gcm::new(key.into())
        .encrypt(Nonce::from_slice(&iv), plain)
        .map_err(|_| "账号库加密失败")?;
    let (cipher, tag) = sealed.split_at(sealed.len() - 16);
    let mut out = iv.to_vec();
    out.extend_from_slice(tag);
    out.extend_from_slice(cipher);
    Ok(out)
}
fn decrypt(key: &[u8; 32], bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() < 28 {
        return Err("账号库已损坏".into());
    }
    let mut sealed = bytes[28..].to_vec();
    sealed.extend_from_slice(&bytes[12..28]);
    let plain = Aes256Gcm::new(key.into())
        .decrypt(Nonce::from_slice(&bytes[..12]), sealed.as_slice())
        .map_err(|_| "账号库解密失败（密钥不匹配或文件已损坏）")?;
    serde_json::from_slice(&plain).map_err(|_| "账号库内容无效".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn user() -> Value {
        json!({"userId":"test-user","token":"secret-token","refreshToken":"secret-refresh","expiredAt":"2030-01-01T00:00:00Z","account":{"username":"test","storeRegion":"CN"}})
    }
    #[test]
    fn vault_is_encrypted_private_and_tamper_evident() {
        let dir = std::env::temp_dir().join(format!("agentdock-trae-{}", uuid::Uuid::new_v4()));
        let s = Store::open(&dir).unwrap();
        let a = s.upsert(json!({"variant":"cn","user":user()})).unwrap();
        let again = s.upsert(json!({"variant":"cn","user":user(),"note":"x"})).unwrap();
        assert_eq!(a["id"], again["id"]);
        let bytes = fs::read(dir.join("vault.enc")).unwrap();
        assert!(!bytes.windows(12).any(|w| w == b"secret-token"));
        let reopened = Store::open(&dir).unwrap();
        assert_eq!(reopened.accounts()[0]["user"]["token"], "secret-token");
        assert_eq!(reopened.accounts().len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(dir.join("vault.enc")).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let mut raw = bytes.clone();
        raw[30] ^= 1;
        fs::write(dir.join("vault.enc"), raw).unwrap();
        assert!(Store::open(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn invalid_accounts_and_pending_requests() {
        let dir = std::env::temp_dir().join(format!("agentdock-trae-{}", uuid::Uuid::new_v4()));
        let s = Store::open(&dir).unwrap();
        assert!(s.upsert(json!({"variant":"cn","user":{"userId":"x"}})).is_err());
        let mut bad = user();
        bad["expiredAt"] = json!("never");
        assert!(s.upsert(json!({"variant":"cn","user":bad})).is_err());
        s.update(|d| {
            d["requests"] = json!([{"id":"r","status":"pending"}]);
            Ok(())
        })
        .unwrap();
        let reopened = Store::open(&dir).unwrap();
        assert_eq!(reopened.snapshot()["requests"][0]["status"], "cancelled");
        assert_eq!(reopened.snapshot()["requests"][0]["errorCode"], "app_restarted");
        fs::remove_dir_all(dir).unwrap();
    }
}
