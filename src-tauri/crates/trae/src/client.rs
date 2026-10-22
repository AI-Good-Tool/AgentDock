//! Local Trae / Trae CN clients: product metadata, login-state envelope
//! encoding, account capture from storage.json and account switching.
//! Installed application code is read as data only and never evaluated.
use aes::Aes128;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use serde_json::{json, Value};
use sha2::{Digest, Sha512};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub const AUTH_KEY: &str = "iCubeAuthInfo://icube.cloudide";
const SERVER_DATA_KEY: &str = "iCubeServerData://icube.cloudide";
const DEVICE_PREFIX: &str = "iCubeAuthInfo://icube-dc:";
const MAGIC: [u8; 6] = [116, 99, 5, 16, 0, 0];

pub const VARIANTS: [&str; 2] = ["cn", "intl"];
pub fn variant_name(variant: &str) -> &'static str {
    if variant == "cn" {
        "Trae CN"
    } else {
        "Trae"
    }
}

#[derive(Clone)]
pub struct Profile {
    pub variant: String,
    pub name: String,
    /// `.app` bundle on macOS, install folder on Windows.
    pub app: PathBuf,
    /// `resources/app` inside the installation.
    pub root: PathBuf,
    pub executable: PathBuf,
    pub dir: PathBuf,
    pub product: Value,
    pub storage: PathBuf,
}
impl Profile {
    /// Login-state salt, derived only when login data is encoded or decoded so
    /// routing, refresh and credits never depend on the client's private format.
    pub fn salt(&self) -> Result<Vec<u8>, String> {
        let source = fs::read_to_string(self.root.join("out/main.js"))
            .map_err(|_| "无法读取客户端主程序")?;
        salt_from_source(&source)
    }
}

static PROFILES: Mutex<Option<HashMap<String, Profile>>> = Mutex::new(None);

fn sha(bytes: &[u8]) -> Vec<u8> {
    Sha512::digest(bytes).to_vec()
}


/// The client XORs two 64-byte constants in the `AES` branch of its key
/// helper (`…AES)for(let r=0;r<t;r++)i[r]=Gle[r]^zle[r]`). Minified names change
/// between builds (Trae 3.6.4: Gle/zle, Trae CN 3.4.6: Yle/Xle), so the pair is
/// located from that expression; the historical names remain a fallback.
fn salt_from_source(source: &str) -> Result<Vec<u8>, String> {
    let names = aes_xor_names(source).unwrap_or(("Gle".into(), "zle".into()));
    let a = byte_array(source, &names.0)?;
    let b = byte_array(source, &names.1)?;
    if a.len() != b.len() || a.is_empty() {
        return Err("客户端加密格式已变化，请重新分析版本".into());
    }
    Ok(a.iter().zip(&b).map(|(x, y)| x ^ y).collect())
}

fn aes_xor_names(source: &str) -> Option<(String, String)> {
    let ident = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') && !s.is_empty();
    for (at, _) in source.match_indices(".AES)for(let ") {
        let rest = &source[at + ".AES)for(let ".len()..];
        let var = rest.split('=').next()?;
        // `r=0;r<t;r++)i[r]=A[r]^B[r]`
        let body = rest.split_once(')')?.1;
        let (_, rhs) = body.split_once(&format!("[{var}]="))?;
        let (lhs, tail) = rhs.split_once('^')?;
        let a = lhs.strip_suffix(&format!("[{var}]"))?;
        let b = tail.split_once(&format!("[{var}]"))?.0;
        if ident(a) && ident(b) && ident(var) {
            return Some((a.to_owned(), b.to_owned()));
        }
    }
    None
}

/// Parse `<key>=Uint8Array.from([...])` / `new Uint8Array([...])` as JSON.
fn byte_array(source: &str, key: &str) -> Result<Vec<u8>, String> {
    for prefix in [
        format!("{key}=Uint8Array.from("),
        format!("{key}=new Uint8Array("),
    ] {
        if let Some(at) = source.find(&prefix) {
            let rest = &source[at + prefix.len()..];
            if let Some(end) = rest.find(']') {
                if rest.starts_with('[') {
                    if let Ok(values) = serde_json::from_str::<Vec<u8>>(&rest[..=end]) {
                        return Ok(values);
                    }
                }
            }
        }
    }
    Err("客户端加密格式已变化，请重新分析版本".into())
}

pub fn profile(variant: &str) -> Result<Profile, String> {
    if !VARIANTS.contains(&variant) {
        return Err("未知客户端版本".into());
    }
    if let Some(p) = PROFILES
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(variant))
    {
        return Ok(p.clone());
    }
    let name = variant_name(variant);
    let found = crate::platform::candidates(variant, name).into_iter().find_map(|c| {
        let product: Value = serde_json::from_str(&fs::read_to_string(c.root.join("product.json")).ok()?).ok()?;
        // Both clients may share an install root: accept only the matching edition.
        let is_cn = product["nameShort"].as_str().unwrap_or("").to_lowercase().split_whitespace().any(|w| w == "cn");
        if (variant == "cn") != is_cn {
            return None;
        }
        let executable = crate::platform::resolve_executable(&c, &product, name)?;
        Some((c, product, executable))
    });
    let Some((c, product, executable)) = found else {
        let hint = if variant == "cn" { "TRAE_CN_PATH" } else { "TRAE_INTL_PATH" };
        return Err(format!("未安装 {name}（或设置 {hint} 指向安装位置）"));
    };
    let dir = crate::platform::data_dir(name);
    let p = Profile {
        variant: variant.into(),
        name: name.into(),
        app: c.app,
        root: c.root,
        executable,
        storage: dir.join("User/globalStorage/storage.json"),
        dir,
        product,
    };
    PROFILES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(variant.into(), p.clone());
    Ok(p)
}

fn key_iv(seed: &[u8], salt: &[u8]) -> Vec<u8> {
    let mut input = sha(seed);
    input.extend_from_slice(salt);
    sha(&input)
}

pub fn decode(value: &str, salt: &[u8]) -> Result<Value, String> {
    let raw = B64.decode(value).map_err(|_| "未知登录态加密版本")?;
    if raw.len() < 38 || raw[..6] != MAGIC {
        return Err("未知登录态加密版本".into());
    }
    let key = key_iv(&raw[6..38], salt);
    let data = cbc::Decryptor::<Aes128>::new(key[..16].into(), key[16..32].into())
        .decrypt_padded_vec_mut::<Pkcs7>(&raw[38..])
        .map_err(|_| "登录态解密失败")?;
    if data.len() < 64 || sha(&data[64..]) != data[..64] {
        return Err("登录态完整性校验失败".into());
    }
    serde_json::from_slice(&data[64..]).map_err(|_| "登录态内容无效".into())
}

pub fn encode(value: &Value, salt: &[u8]) -> String {
    let data = serde_json::to_vec(value).unwrap();
    let seed: [u8; 32] = rand::random();
    let key = key_iv(&seed, salt);
    let mut plain = sha(&data);
    plain.extend_from_slice(&data);
    let cipher = cbc::Encryptor::<Aes128>::new(key[..16].into(), key[16..32].into())
        .encrypt_padded_vec_mut::<Pkcs7>(&plain);
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&seed);
    out.extend_from_slice(&cipher);
    B64.encode(out)
}

/// Capture the signed-in account of a local client, with device keys and cached models.
pub fn read_local(variant: &str) -> Result<Value, String> {
    let p = profile(variant)?;
    let storage: Value = serde_json::from_str(
        &fs::read_to_string(&p.storage).map_err(|_| format!("请先登录 {}", p.name))?,
    )
    .map_err(|_| "客户端 storage.json 无效")?;
    let auth = storage[AUTH_KEY]
        .as_str()
        .ok_or_else(|| format!("请先登录 {}", p.name))?;
    let salt = p.salt()?;
    let user = decode(auth, &salt)?;
    let pair = storage
        .as_object()
        .and_then(|o| o.iter().find(|(k, _)| k.starts_with(DEVICE_PREFIX)));
    let cached = storage[SERVER_DATA_KEY]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(json!({}));
    let models = read_cached_models(&p, user["userId"].as_str().unwrap_or(""));
    Ok(json!({
        "variant": variant,
        "user": user,
        "models": models,
        "deviceId": pair.and_then(|(k, _)| k.rsplit(':').next()),
        "deviceKeys": pair.and_then(|(_, v)| v.as_str()).and_then(|v| decode(v, &salt).ok()),
        "machineId": storage["telemetry.machineId"].as_str().unwrap_or(""),
        "cached": cached,
        "capturedAt": chrono::Utc::now().to_rfc3339(),
    }))
}

/// Models cached by the client for this user (read-only SQLite access).
pub fn read_cached_models(p: &Profile, user_id: &str) -> Vec<Value> {
    let path = p.dir.join("User/globalStorage/state.vscdb");
    let Ok(db) =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return vec![];
    };
    let Ok(text) = db.query_row(
        "SELECT value FROM ItemTable WHERE key = ?1",
        [format!("{user_id}:AI.agent.model.model_list_map")],
        |row| row.get::<_, String>(0),
    ) else {
        return vec![];
    };
    let Ok(groups) = serde_json::from_str::<serde_json::Map<String, Value>>(&text) else {
        return vec![];
    };
    let mut models: Vec<Value> = vec![];
    for (group, list) in groups {
        for m in list.as_array().into_iter().flatten() {
            let Some(name) = m["name"].as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            if m["status"] == false {
                continue;
            }
            if let Some(found) = models.iter_mut().find(|x| x["model"] == name) {
                let functions = found["functions"].as_array_mut().unwrap();
                if !functions.iter().any(|f| f == &json!(group)) {
                    functions.push(json!(group));
                }
                continue;
            }
            models.push(json!({
                "id": m["config_name"].as_str().filter(|s| !s.is_empty()).unwrap_or(name),
                "name": m["display_name"].as_str().filter(|s| !s.is_empty()).unwrap_or(name),
                "model": name,
                "source": m["config_source"].as_i64().unwrap_or(1),
                "catalogSource": "client-cache",
                "functions": [group],
                "remote": m["use_remote_service"].as_bool().unwrap_or(false),
            }));
        }
    }
    models
}

/// Official host for an account, from the client's boot configuration.
pub fn host_for(account: &Value, category: &str) -> Result<String, String> {
    let p = profile(account["variant"].as_str().unwrap_or(""))?;
    let boot = &p.product["bootConfig"];
    let hosts = if category == "account" {
        &boot["account"]["trae"]
    } else {
        &boot[category]["trae"]
    };
    let region = account["user"]["account"]["storeRegion"]
        .as_str()
        .unwrap_or("");
    let host = hosts[region]
        .as_str()
        .or_else(|| hosts["normal"].as_str())
        .ok_or("客户端服务地址缺失")?;
    let url = reqwest::Url::parse(host).map_err(|_| "客户端服务地址无效")?;
    if url.scheme() != "https" {
        return Err("客户端服务地址必须是 HTTPS".into());
    }
    Ok(url.origin().ascii_serialization())
}

pub fn launch(variant: &str) -> Result<(), String> {
    let p = profile(variant)?;
    crate::platform::launch(&p.app, &p.executable)
}

/// Unknown process state counts as running so the login is never overwritten blindly.
pub fn is_running(variant: &str) -> bool {
    let Ok(p) = profile(variant) else {
        return false;
    };
    crate::platform::processes(&p.app, &p.executable, false).map_or(true, |ps| ps.iter().any(|x| x.main))
}

/// Pids of every process inside this client's installation (main app and helpers).
fn bundle_pids(p: &Profile) -> Result<Vec<u32>, String> {
    crate::platform::processes(&p.app, &p.executable, true).map(|ps| ps.into_iter().map(|x| x.pid).collect())
}

fn wait_until(deadline: std::time::Duration, done: impl Fn() -> bool) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(crate::platform::QUIT_POLL_MS));
    }
    done()
}

/// Quit a running client so it cannot overwrite the login we write: ask it to
/// quit (it saves its own state), then SIGTERM / SIGKILL only this bundle's
/// processes. Returns whether the client had been running.
pub fn quit(variant: &str) -> Result<bool, String> {
    let p = profile(variant)?;
    let running = crate::platform::processes(&p.app, &p.executable, true)
        .map_err(|e| format!("{e}，未修改 {} 登录态", p.name))?;
    if running.is_empty() {
        return Ok(false);
    }
    let mains: Vec<u32> = running.iter().filter(|x| x.main).map(|x| x.pid).collect();
    crate::platform::request_quit(&p.app, &mains);
    let gone = || bundle_pids(&p).is_ok_and(|v| v.is_empty());
    if !wait_until(std::time::Duration::from_secs(10), &gone) {
        for force in [false, true] {
            let pids = bundle_pids(&p).unwrap_or_default();
            if pids.is_empty() {
                break;
            }
            crate::platform::terminate(&pids, force);
            if wait_until(std::time::Duration::from_secs(4), &gone) {
                break;
            }
        }
    }
    if !gone() {
        return Err(format!("无法退出 {}，请手动退出后重试", p.name));
    }
    // Let the file system settle after the client's own shutdown writes.
    std::thread::sleep(std::time::Duration::from_millis(500));
    Ok(true)
}

/// Write the selected identity into the client (quitting it first when it
/// runs), after a private backup, then reopen it.
pub fn switch_account(account: &Value, backup_dir: &Path) -> Result<Value, String> {
    let variant = account["variant"].as_str().unwrap_or("");
    let p = profile(variant)?;
    // A running client would overwrite the login on exit: quit it first.
    let restarted = quit(variant)?;
    let original = fs::read_to_string(&p.storage).map_err(|_| "无法读取客户端登录存储")?;
    let mut data: serde_json::Map<String, Value> =
        serde_json::from_str(&original).map_err(|_| "客户端 storage.json 无效")?;
    write_private_dir(backup_dir)?;
    let backup = backup_dir.join(format!(
        "{variant}-{}.json",
        chrono::Utc::now().timestamp_millis()
    ));
    let salt = p.salt()?;
    write_private(&backup, original.as_bytes())?;
    data.insert(AUTH_KEY.into(), json!(encode(&account["user"], &salt)));
    // Cached entitlement and device credentials must follow the selected identity.
    data.remove(SERVER_DATA_KEY);
    data.retain(|k, _| !k.starts_with(DEVICE_PREFIX));
    if let (Some(keys), Some(id)) = (
        account.get("deviceKeys").filter(|v| v.is_object()),
        account["deviceId"].as_str(),
    ) {
        data.insert(format!("{DEVICE_PREFIX}{id}"), json!(encode(keys, &salt)));
    }
    if let Some(cached) = account.get("cached").filter(|v| v.is_object()) {
        data.insert(SERVER_DATA_KEY.into(), json!(cached.to_string()));
    }
    let tmp = p.storage.with_extension("json.agentdock.tmp");
    // Match the client's own tab-indented storage.json formatting.
    let mut text = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
    let mut serializer = serde_json::Serializer::with_formatter(&mut text, formatter);
    serde::Serialize::serialize(&Value::Object(data), &mut serializer)
        .map_err(|_| "无法生成客户端登录态")?;
    write_private(&tmp, &text)?;
    fs::rename(&tmp, &p.storage).map_err(|_| "无法写入客户端登录态")?;
    launch(variant)?;
    Ok(json!({"backup": backup, "restarted": restarted}))
}

pub fn write_private_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|_| "无法创建数据目录")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

pub fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| "无法写入本地文件")?;
    file.write_all(data).map_err(|_| "无法写入本地文件")?;
    file.sync_all().map_err(|_| "无法同步本地文件".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_envelope_roundtrip_and_integrity() {
        let salt: Vec<u8> = (0..64u32).map(|i| (i * 7 % 251) as u8).collect();
        let user = json!({"userId":"test-user","token":"secret-token","expiredAt":"2030-01-01T00:00:00Z","account":{"username":"test","storeRegion":"CN"}});
        let encoded = encode(&user, &salt);
        assert_eq!(decode(&encoded, &salt).unwrap(), user);
        let mut raw = B64.decode(&encoded).unwrap();
        raw[45] ^= 1;
        assert!(decode(&B64.encode(raw), &salt).is_err());
        assert!(decode(&encoded, &[0u8; 64]).is_err());
    }
    #[test]
    fn constant_arrays_are_parsed_as_data() {
        let src = "x;Gle=Uint8Array.from([1,2,3]),zle=new Uint8Array([4,5,6]);";
        assert_eq!(byte_array(src, "Gle").unwrap(), vec![1, 2, 3]);
        assert_eq!(byte_array(src, "zle").unwrap(), vec![4, 5, 6]);
    }
    #[test]
    fn salt_pair_is_found_from_the_aes_branch_whatever_the_names() {
        let src = "Kle=new Uint8Array([9,9]),Jle=new Uint8Array([1,1]),Yle=Uint8Array.from([1,2]),Xle=Uint8Array.from([3,4]);function h(t,e){const i=new Uint8Array(t);if(e===Qs.AES_PRIVATE)for(let r=0;r<t;r++)i[r]=Kle[r]^Jle[r];else if(e===Qs.AES)for(let r=0;r<t;r++)i[r]=Yle[r]^Xle[r];return i}";
        assert_eq!(aes_xor_names(src), Some(("Yle".into(), "Xle".into())));
        assert_eq!(salt_from_source(src).unwrap(), vec![1 ^ 3, 2 ^ 4]);
        // Old builds without the expression still use the historical names.
        assert_eq!(salt_from_source("Gle=Uint8Array.from([5]),zle=Uint8Array.from([6])").unwrap(), vec![5 ^ 6]);
        assert!(byte_array(src, "Missing").is_err());
    }
}
