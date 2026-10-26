//! Local proxy credentials only. Never changes official account credentials.
//!
//! Several keys may call the gateway; each can narrow the providers/regions it
//! uses and pick its own scheduling strategy. Keys live in `api-keys.json`
//! (owner-only). The legacy single-key file `api-key` keeps mirroring the
//! first key so tools that read it keep working.
use crate::settings::{Settings, Strategy};
use agentdock_core::provider::ProviderKind;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};
use uuid::Uuid;

pub const MAX_KEYS: usize = 64;

pub fn generate() -> String {
    format!("ad-{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
pub fn validate(value: &str) -> Result<(), String> {
    if !(16..=256).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("API Key 必须为 16–256 个非空白 ASCII 字符，不能包含空格或换行".into());
    }
    Ok(())
}
/// Timing depends only on the lengths, never on where the bytes differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
/// Short, non-reversible form shown in records ("ad-1…9f0e").
pub fn abbreviate(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let (head, tail) = if chars.len() >= 24 { (4, 4) } else { (2, 2) };
    if chars.len() <= head + tail {
        return "…".into();
    }
    format!("{}…{}", chars[..head].iter().collect::<String>(), chars[chars.len() - tail..].iter().collect::<String>())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ApiKey {
    pub id: String,
    pub key: String,
    pub alias: String,
    pub enabled: bool,
    /// `None` follows the global strategy.
    pub strategy: Option<Strategy>,
    /// Allowed regions per provider; `None` follows the global scope. Only
    /// narrows: globally disabled providers/regions stay off.
    pub scope: Option<BTreeMap<ProviderKind, Vec<String>>>,
    pub created_at: i64,
}
impl ApiKey {
    pub fn hint(&self) -> String {
        abbreviate(&self.key)
    }
    /// Alias, else the abbreviated key.
    pub fn label(&self) -> String {
        if self.alias.is_empty() { self.hint() } else { self.alias.clone() }
    }
    /// The gateway settings requests with this key run under.
    pub fn apply(&self, mut settings: Settings) -> Settings {
        if let Some(strategy) = self.strategy {
            settings.strategy = strategy;
        }
        if let Some(scope) = &self.scope {
            for (provider, s) in settings.providers.iter_mut() {
                let allowed = scope.get(provider).map(Vec::as_slice).unwrap_or_default();
                s.regions.retain(|r| allowed.contains(r));
                s.enabled &= !s.regions.is_empty();
            }
        }
        settings
    }
}

/// Editable fields of a key (everything but id, value and creation time).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KeyFields {
    pub alias: String,
    pub enabled: bool,
    pub strategy: Option<Strategy>,
    pub scope: Option<BTreeMap<ProviderKind, Vec<String>>>,
}
impl KeyFields {
    fn validate(&self) -> Result<(), String> {
        let alias = self.alias.trim();
        if alias.chars().count() > 64 || alias.chars().any(char::is_control) {
            return Err("API Key 别名最多 64 个字符，不能包含控制字符".into());
        }
        if let Some(scope) = &self.scope {
            if scope.values().flatten().any(|r| !["cn", "intl"].contains(&r.as_str())) {
                return Err("API Key 的地区设置无效".into());
            }
            if scope.values().all(Vec::is_empty) {
                return Err("请至少为此 API Key 选择一个端与地区".into());
            }
        }
        Ok(())
    }
}

pub struct KeyStore {
    dir: PathBuf,
    keys: Mutex<Vec<ApiKey>>,
    writer: Mutex<()>,
}
impl KeyStore {
    fn path(dir: &Path) -> PathBuf {
        dir.join("api-keys.json")
    }
    /// Load `api-keys.json`, else start from the legacy `api-key` (or a new
    /// key). An unreadable file is set aside, never overwritten.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let path = Self::path(dir);
        let aside = || dir.join(format!("api-keys.json.unreadable-{}", chrono::Utc::now().timestamp()));
        let mut keys: Vec<ApiKey> = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Vec<ApiKey>>(&bytes) {
                Ok(keys) => keys,
                Err(_) => {
                    std::fs::rename(&path, aside()).map_err(|_| "API Key 文件无法读取，也无法移开")?;
                    vec![]
                }
            },
            Err(_) => vec![],
        };
        let loaded = keys.len();
        let mut seen = std::collections::HashSet::new();
        keys.retain(|k| validate(&k.key).is_ok() && !k.id.is_empty() && seen.insert(k.id.clone()) && seen.insert(format!("\0{}", k.key)));
        let dropped = keys.len() < loaded;
        if dropped {
            std::fs::copy(&path, aside()).map_err(|_| "API Key 文件无法备份")?;
        }
        let revived = !keys.is_empty() && !keys.iter().any(|k| k.enabled);
        if revived {
            keys[0].enabled = true;
        }
        let store = Self { dir: dir.to_owned(), keys: Mutex::new(vec![]), writer: Mutex::new(()) };
        if !keys.is_empty() && (dropped || revived) {
            // The original is kept aside, so the repaired list can replace it.
            store.persist(&keys)?;
        }
        if keys.is_empty() {
            let legacy = std::fs::read_to_string(dir.join("api-key")).ok().map(|k| k.trim().to_owned())
                .filter(|k| validate(k).is_ok());
            keys.push(ApiKey {
                id: "default".into(),
                key: legacy.unwrap_or_else(generate),
                enabled: true,
                created_at: chrono::Utc::now().timestamp_millis(),
                ..ApiKey::default()
            });
            store.persist(&keys)?;
        }
        *store.keys.lock().unwrap() = keys;
        Ok(store)
    }
    /// In-memory store (tests).
    pub fn fixed(dir: &Path, value: &str) -> Self {
        let key = ApiKey { id: "default".into(), key: value.into(), enabled: true, ..ApiKey::default() };
        Self { dir: dir.to_owned(), keys: Mutex::new(vec![key]), writer: Mutex::new(()) }
    }
    fn persist(&self, keys: &[ApiKey]) -> Result<(), String> {
        // Disk failure leaves the live keys unchanged. Never print key-bearing I/O errors.
        crate::app::write_private(&Self::path(&self.dir), &serde_json::to_vec_pretty(keys).unwrap())
            .map_err(|_| "无法保存本地 API Key".to_owned())?;
        let legacy = self.dir.join("api-key");
        if std::fs::read_to_string(&legacy).ok().as_deref().map(str::trim) != Some(keys[0].key.as_str()) {
            crate::app::write_private(&legacy, keys[0].key.as_bytes()).map_err(|_| "无法同步本地 API Key 文件".to_owned())?;
        }
        Ok(())
    }
    /// Apply `change` to a copy, validate, persist, then publish.
    fn edit(&self, change: impl FnOnce(&mut Vec<ApiKey>) -> Result<(), String>) -> Result<(), String> {
        let _writer = self.writer.lock().unwrap();
        let mut next = self.list();
        change(&mut next)?;
        if !next.iter().any(|k| k.enabled) {
            return Err("至少保留一个启用的 API Key".into());
        }
        if next.len() > MAX_KEYS {
            return Err(format!("最多 {MAX_KEYS} 个 API Key"));
        }
        for (i, k) in next.iter().enumerate() {
            if next[..i].iter().any(|o| o.key == k.key) {
                return Err("API Key 与已有的 Key 重复".into());
            }
            if !k.alias.is_empty() && next[..i].iter().any(|o| o.alias == k.alias) {
                return Err(format!("API Key 别名重复：{}", k.alias));
            }
        }
        self.persist(&next)?;
        *self.keys.lock().unwrap() = next;
        Ok(())
    }
    pub fn list(&self) -> Vec<ApiKey> {
        self.keys.lock().unwrap().clone()
    }
    /// The first key (mirrored to the legacy `api-key` file).
    pub fn primary(&self) -> String {
        self.keys.lock().unwrap()[0].key.clone()
    }
    /// Compared in constant time, since LAN access exposes the gateway to other devices.
    pub fn find(&self, value: &str) -> Option<ApiKey> {
        self.keys.lock().unwrap().iter().find(|k| constant_time_eq(k.key.as_bytes(), value.as_bytes())).cloned()
    }
    pub fn create(&self, value: Option<String>, fields: KeyFields) -> Result<ApiKey, String> {
        fields.validate()?;
        let key = ApiKey {
            id: format!("k{}", &Uuid::new_v4().simple().to_string()[..12]),
            key: value.map(|v| v.trim().to_owned()).unwrap_or_else(generate),
            alias: fields.alias.trim().to_owned(),
            enabled: fields.enabled,
            strategy: fields.strategy,
            scope: fields.scope,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        validate(&key.key)?;
        let created = key.clone();
        self.edit(|keys| {
            keys.push(key);
            Ok(())
        })?;
        Ok(created)
    }
    pub fn update(&self, id: &str, fields: KeyFields) -> Result<(), String> {
        fields.validate()?;
        self.edit(|keys| {
            let k = keys.iter_mut().find(|k| k.id == id).ok_or("API Key 不存在，请刷新")?;
            k.alias = fields.alias.trim().to_owned();
            k.enabled = fields.enabled;
            k.strategy = fields.strategy;
            k.scope = fields.scope;
            Ok(())
        })
    }
    /// Replace a key's value; `expected` guards against stale edits.
    pub fn replace(&self, id: &str, value: String, expected: &str) -> Result<(), String> {
        let value = value.trim().to_owned();
        validate(&value)?;
        self.edit(|keys| {
            let k = keys.iter_mut().find(|k| k.id == id).ok_or("API Key 不存在，请刷新")?;
            if k.key != expected {
                return Err("API Key 已被其他操作修改，请刷新后重新编辑".into());
            }
            k.key = value;
            Ok(())
        })
    }
    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.edit(|keys| {
            let before = keys.len();
            keys.retain(|k| k.id != id);
            if keys.len() == before {
                return Err("API Key 不存在，请刷新".into());
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agentdock-key-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    #[test]
    fn random_and_validation() {
        let a = generate();
        let b = generate();
        assert_ne!(a, b);
        assert_eq!(a.len(), 67);
        validate(&a).unwrap();
        for s in ["short", "123456789012345 ", "123456789012345\n", "123456789012345中"] {
            assert!(validate(s).is_err());
        }
        assert!(validate(&"x".repeat(257)).is_err());
        assert_eq!(abbreviate("ad-0123456789abcdef0123456789"), "ad-0…6789");
        assert_eq!(abbreviate("0123456789abcdef"), "01…ef");
        assert!(constant_time_eq(a.as_bytes(), a.clone().as_bytes()));
        assert!(!constant_time_eq(a.as_bytes(), b.as_bytes()));
        assert!(!constant_time_eq(a.as_bytes(), &a.as_bytes()[..66]));
    }
    #[test]
    fn legacy_key_migrates_and_the_first_key_stays_mirrored_privately() {
        let dir = temp();
        let old = generate();
        std::fs::write(dir.join("api-key"), format!("{old}\n")).unwrap();
        let store = KeyStore::open(&dir).unwrap();
        assert_eq!(store.primary(), old);
        assert_eq!(store.list()[0].id, "default");
        let second = store.create(None, KeyFields { alias: "ci".into(), enabled: true, ..KeyFields::default() }).unwrap();
        assert_eq!(KeyStore::open(&dir).unwrap().list().len(), 2);
        // Replacing the first key rewrites the legacy file; stale edits fail.
        let new = generate();
        store.replace("default", new.clone(), &old).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("api-key")).unwrap(), new);
        assert!(store.replace("default", generate(), &old).is_err());
        // Duplicates, empty scopes and deleting the last key are refused.
        assert!(store.replace(&second.id, new.clone(), &second.key).is_err());
        assert!(store.create(None, KeyFields { alias: "ci".into(), ..KeyFields::default() }).is_err());
        let empty = KeyFields { scope: Some(BTreeMap::new()), ..KeyFields::default() };
        assert!(store.update(&second.id, empty).is_err());
        let off = KeyFields { enabled: false, ..KeyFields::default() };
        store.update(&second.id, off.clone()).unwrap();
        assert!(store.update("default", off).is_err(), "one key must stay enabled");
        assert!(store.delete("default").is_err());
        store.update(&second.id, KeyFields { alias: "ci".into(), enabled: true, ..KeyFields::default() }).unwrap();
        store.delete("default").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("api-key")).unwrap(), second.key);
        assert!(store.delete(&second.id).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for f in ["api-key", "api-keys.json"] {
                assert_eq!(std::fs::metadata(dir.join(f)).unwrap().permissions().mode() & 0o777, 0o600);
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn unreadable_file_is_set_aside_and_disk_failure_publishes_nothing() {
        let dir = temp();
        std::fs::write(dir.join("api-keys.json"), b"{broken").unwrap();
        let store = KeyStore::open(&dir).unwrap();
        assert_eq!(store.list().len(), 1);
        assert!(std::fs::read_dir(&dir).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("api-keys.json.unreadable-")));
        // Invalid and duplicate entries are skipped (original kept aside); one key stays usable.
        let other = temp();
        let k = generate();
        let entries = json!([{"id":"a","key":k,"enabled":false},{"id":"b","key":k},{"id":"c","key":"short"}]);
        std::fs::write(other.join("api-keys.json"), entries.to_string()).unwrap();
        let loaded = KeyStore::open(&other).unwrap().list();
        assert_eq!((loaded.len(), loaded[0].id.as_str(), loaded[0].enabled), (1, "a", true));
        let asides = |d: &Path| std::fs::read_dir(d).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("api-keys.json.unreadable-")).count();
        assert_eq!(asides(&other), 1);
        // The repaired list is saved, so reopening sets nothing else aside.
        assert_eq!(KeyStore::open(&other).unwrap().list(), loaded);
        assert_eq!(asides(&other), 1);
        std::fs::remove_dir_all(other).unwrap();
        let file = dir.join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        let broken = KeyStore::fixed(&file, &generate());
        let before = broken.list();
        assert!(broken.create(None, KeyFields { enabled: true, ..KeyFields::default() }).is_err());
        assert_eq!(broken.list(), before);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn key_scope_only_narrows_and_strategy_overrides() {
        let mut global = Settings::default();
        global.providers.get_mut(&ProviderKind::Qoder).unwrap().regions = vec!["cn".into()];
        let key = ApiKey {
            strategy: Some(Strategy::Random),
            scope: Some(BTreeMap::from([
                (ProviderKind::Qoder, vec!["cn".into(), "intl".into()]),
                (ProviderKind::Trae, vec!["intl".into()]),
            ])),
            ..ApiKey::default()
        };
        let s = key.apply(global.clone());
        assert_eq!(s.strategy, Strategy::Random);
        assert_eq!(s.scope(ProviderKind::Qoder).regions, ["cn"], "globally off regions stay off");
        assert_eq!(s.scope(ProviderKind::Trae).regions, ["intl"]);
        assert!(!s.scope(ProviderKind::Workbuddy).enabled);
        assert_eq!(s.scope(ProviderKind::Trae).concurrency, global.scope(ProviderKind::Trae).concurrency);
        assert_eq!(ApiKey::default().apply(global.clone()), global);
    }
}
