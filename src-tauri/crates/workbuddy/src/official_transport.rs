//! Private Unix socket bridge to the installed WorkBuddy Electron/Node runtime.
//! No model traffic or native SDK calls occur until send() is explicitly used.
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::Mutex,
};
use uuid::Uuid;

const SCRIPT: &str = include_str!("../resources/official-transport.cjs");
#[derive(Default)]
pub struct Transport(Mutex<Option<Worker>>);
struct Worker {
    child: Child,
    _stdin: ChildStdin,
    client: reqwest::Client,
    directory: PathBuf,
    #[cfg(test)]
    socket: PathBuf,
    key: String,
    drain: tokio::task::JoinHandle<()>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        self.drain.abort();
        // Both paths are uniquely created and owned by this worker.
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
impl Worker {
    /// Persistent helper bundle installed at a fixed path. The keychain entry
    /// created by the official Turing SDK is ACL-bound to this launcher's code
    /// signature; regenerating it on every start (as before) made macOS treat
    /// each run as a new application and prompted for the login password again.
    /// Install once, reuse on every launch, and only re-sign when the official
    /// Electron binary has changed.
    #[cfg(target_os = "macos")]
    async fn helper_executable() -> Result<PathBuf, String> {
        use std::os::unix::fs::symlink;
        // Serialize concurrent installers (notably tests and a cold start racing
        // a worker respawn): two writers fighting over the same signed bundle
        // would leave a torn install and break the stable code signature the
        // keychain ACL depends on.
        static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _install_guard = INSTALL_LOCK.lock().await;
        let home = std::env::var("HOME").map_err(|_| "无法确定用户目录")?;
        let install_root = PathBuf::from(home).join("Library/Application Support/WorkBuddy API");
        let bundle = install_root.join("WorkBuddy API Transport.app");
        let contents = bundle.join("Contents");
        let macos = contents.join("MacOS");
        let executable = macos.join("workbuddy-api-transport");
        // Marker lives outside the signed bundle: codesign would otherwise
        // treat it as an unsigned subcomponent and refuse to seal the app.
        let marker = install_root.join(".transport-installed-version");
        let official = std::fs::read("/Applications/WorkBuddy.app/Contents/MacOS/Electron")
            .map_err(|_| "无法读取已安装 WorkBuddy 的 Node 启动器")?;
        let fingerprint = format!("{}:{}:{}", official.len(), env!("CARGO_PKG_VERSION"), {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::Hasher;
            let mut h = DefaultHasher::new();
            h.write(&official);
            h.finish()
        });
        let reusable = executable.exists()
            && std::fs::read_to_string(&marker)
                .map(|saved| saved.trim() == fingerprint)
                .unwrap_or(false);
        if !reusable {
            std::fs::create_dir_all(&macos).map_err(|_| "无法创建后台传输运行环境")?;
            std::fs::write(&executable, &official).map_err(|_| "无法写入后台传输启动器")?;
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
                    .map_err(|_| "无法设置后台传输启动器执行权限")?;
            }
            let frameworks = contents.join("Frameworks");
            if !frameworks.exists() {
                symlink(
                    "/Applications/WorkBuddy.app/Contents/Frameworks",
                    &frameworks,
                )
                .map_err(|_| "无法关联已安装 WorkBuddy 的运行框架")?;
            }
            std::fs::write(contents.join("Info.plist"), format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>local.workbuddy.api.transport</string>
<key>CFBundleName</key><string>WorkBuddy API Transport</string>
<key>CFBundleExecutable</key><string>workbuddy-api-transport</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>{}</string>
<key>LSBackgroundOnly</key><true/>
</dict></plist>"#, env!("CARGO_PKG_VERSION"))).map_err(|_| "无法创建后台传输配置")?;
            // Sign only our owned launcher bundle. Do not deep-sign or modify the
            // symlinked, already-installed official frameworks.
            let status = Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-", "--timestamp=none"])
                .arg(&bundle)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map_err(|_| "后台传输签名工具不可用")?;
            if !status.success() {
                return Err("后台传输启动器完整性签名失败".into());
            }
            std::fs::write(&marker, &fingerprint).map_err(|_| "无法记录后台传输版本")?;
        }
        Ok(executable)
    }
    #[cfg(unix)]
    async fn start() -> Result<Self, String> {
        Self::launch(SCRIPT).await
    }
    #[cfg(unix)]
    async fn launch(script: &str) -> Result<Self, String> {
        use std::os::unix::fs::DirBuilderExt;
        let directory = PathBuf::from(format!("/tmp/wb-api-{}", Uuid::new_v4().simple()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| "无法创建本机官方传输通道")?;
        let socket = directory.join("transport.sock");
        let key = Uuid::new_v4().simple().to_string();
        #[cfg(target_os = "macos")]
        let executable = match Self::helper_executable().await {
            Ok(executable) => executable,
            Err(message) => {
                let _ = std::fs::remove_dir_all(&directory);
                return Err(message);
            }
        };
        #[cfg(not(target_os = "macos"))]
        let executable = PathBuf::from("/Applications/WorkBuddy.app/Contents/MacOS/Electron");
        let child = Command::new(executable)
            .args(["-e", script])
            .env("ELECTRON_RUN_AS_NODE", "1")
            .env("WB_OFFICIAL_BRIDGE", "1")
            .env("WB_OFFICIAL_SOCKET", &socket)
            .env("WB_OFFICIAL_KEY", &key)
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_EXTRA_CA_CERTS")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(_) => {
                let _ = std::fs::remove_dir_all(&directory);
                return Err(
                    "请安装已验证的 WorkBuddy 5.8.3，以获取官方 Node 运行时与设备 SDK".into(),
                );
            }
        };
        let stdin = child.stdin.take().ok_or("本机官方传输初始化失败")?;
        let stdout = child.stdout.take().ok_or("本机官方传输初始化失败")?;
        let client = reqwest::Client::builder()
            .unix_socket(socket.clone())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(200))
            .build()
            .map_err(|_| "本机官方传输初始化失败")?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let ready =
            tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line)).await;
        // Discard native output, never persist it or leave a closed stdout pipe.
        let drain = tokio::spawn(async move {
            let _ = tokio::io::copy(&mut reader, &mut tokio::io::sink()).await;
        });
        let worker = Self {
            child,
            _stdin: stdin,
            client,
            directory,
            #[cfg(test)]
            socket,
            key,
            drain,
        };
        if !matches!(ready, Ok(Ok(_)))
            || serde_json::from_str::<serde_json::Value>(&line)
                .ok()
                .is_none_or(|v| v["ready"] != true)
        {
            return Err("本机 WorkBuddy 版本、Node 运行时或设备 SDK 与已验证配置不匹配".into());
        }
        Ok(worker)
    }
    #[cfg(not(unix))]
    async fn start() -> Result<Self, String> {
        Err("当前系统尚未接入官方设备 SDK 传输".into())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    #[tokio::test]
    async fn helper_has_separate_background_identity() {
        let module =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/official-transport.cjs");
        let script = format!(
            r#"
delete process.env.WB_OFFICIAL_BRIDGE;
const {{initializeBackgroundCocoa}}=require({module});
const k=require('/Applications/WorkBuddy.app/Contents/Resources/app.asar.unpacked/node_modules/koffi');
const policy=initializeBackgroundCocoa(k), repeatedPolicy=initializeBackgroundCocoa(k);
const o=k.load('/usr/lib/libobjc.A.dylib');
const cls=o.func('void *objc_getClass(const char *)'),sel=o.func('void *sel_registerName(const char *)');
const object=o.func('objc_msgSend','void *',['void *','void *']);
const current=object(cls('NSRunningApplication'),sel('currentApplication'));
const identifier=object(current,sel('bundleIdentifier'));
const bundleId=o.func('objc_msgSend','str',['void *','void *'])(identifier,sel('UTF8String'));
const server=require('node:http').createServer((req,res)=>res.end(JSON.stringify({{policy,repeatedPolicy,bundleId,sdkLoaded:false}})));
server.listen(process.env.WB_OFFICIAL_SOCKET,()=>console.log('{{"ready":true}}'));
process.stdin.resume();process.stdin.on('end',()=>process.exit(0));
"#,
            module = serde_json::to_string(&module).unwrap()
        );
        let worker = Worker::launch(&script).await.unwrap();
        let info: Value = worker
            .client
            .get("http://localhost/")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(info["policy"], 2);
        assert_eq!(info["repeatedPolicy"], 2);
        assert_eq!(info["bundleId"], "local.workbuddy.api.transport");
        assert_eq!(info["sdkLoaded"], false);
        let directory = worker.directory.clone();
        drop(worker);
        assert!(!directory.exists());
        assert!(std::path::Path::new(
            "/Applications/WorkBuddy.app/Contents/Frameworks/Electron Framework.framework"
        )
        .exists());
    }
    #[tokio::test]
    async fn installed_runtime_starts_offline_without_fetching_a_device_token_and_cleans_up() {
        use std::os::unix::fs::PermissionsExt;
        let worker = Worker::start().await.unwrap();
        assert_eq!(
            std::fs::metadata(&worker.directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&worker.socket)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let directory = worker.directory.clone();
        drop(worker);
        assert!(!directory.exists());
    }
    #[tokio::test]
    async fn rust_to_installed_node_bridge_matches_desktop_header_contract_and_node_gzip_bytes() {
        // Only the test harness supplies a fake device provider and a loopback
        // upstream. Production has no environment switch for these substitutions.
        let module =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/official-transport.cjs");
        let script = format!(
            r#"
delete process.env.WB_OFFICIAL_BRIDGE;
const {{createBridge,encode}} = require({module});
const http=require('node:http'),fs=require('node:fs'),z=require('node:zlib');
const upstream=http.createServer(async(req,res)=>{{
 const chunks=[];for await(const c of req)chunks.push(c);const bytes=Buffer.concat(chunks);
 const body=JSON.parse(z.gunzipSync(bytes));
 const headers=req.headers;
 const capture={{headers,body,gzipHeaderHex:bytes.subarray(0,10).toString('hex'),
   exactNodeBytes:bytes.equals(encode(body))}};
 res.writeHead(200,{{'content-type':'text/event-stream'}});
 res.end('data: '+JSON.stringify({{model:body.model,choices:[{{index:0,delta:{{content:'OK'}},finish_reason:'stop'}}],
 usage:{{prompt_tokens:31,completion_tokens:7,total_tokens:38}},capture}})+'\n\ndata: [DONE]\n\n');
}});
upstream.listen(0,'127.0.0.1',()=>{{
 const server=createBridge({{key:process.env.WB_OFFICIAL_KEY,variant:'domestic',tokens:{{get:async()=> 'fixture-native-device'}},
 connect:(options,cb)=>http.request({{...options,protocol:'http:',hostname:'127.0.0.1',port:upstream.address().port}},cb)}});
 server.listen(process.env.WB_OFFICIAL_SOCKET,()=>{{fs.chmodSync(process.env.WB_OFFICIAL_SOCKET,0o600);console.log('{{"ready":true}}');}});
 process.stdin.resume();process.stdin.on('end',()=>process.exit(0));
}});
"#,
            module = serde_json::to_string(&module).unwrap()
        );
        let worker = Worker::launch(&script).await.unwrap();
        let transport = Transport(Mutex::new(Some(worker)));
        let state = crate::pool::AppState::new(std::env::temp_dir().join(Uuid::new_v4().to_string()))
        .unwrap();
        let account: crate::pool::Account = serde_json::from_value(json!({"id":"fixture","name":"fixture","source":"fixture","sourceId":"fixture","variant":"cn","enabled":true})).unwrap();
        let credential = crate::pool::Credential {
            token: "fixture-credential".into(),
            uid: "fixture-uid".into(),
            domain: "www.codebuddy.cn".into(),
            enterprise: None,
        };
        let mut incoming = axum::http::HeaderMap::new();
        incoming.insert("authorization", "Bearer downstream-secret".parse().unwrap());
        incoming.insert("x-device-token", "untrusted-device".parse().unwrap());
        let mut body = json!({"model":"deepseek-v4.1-flash","max_tokens":512,"messages":[{"role":"system","content":"CLIENT_SYSTEM"},{"role":"user","content":"你好"}],"tools":[{"type":"function","function":{"name":"CLIENT_TOOL","parameters":{"type":"object"}}}]});
        crate::official::prepare_body(
            &mut body,
            Some(
                &json!({"temperature":1,"maxOutputTokens":128000,"reasoning":{"defaultEffort":"high","summary":"auto"}}),
            ),
        );
        let context = state
            .conversations
            .lock()
            .unwrap()
            .prepare(&incoming, &mut body);
        let request = crate::official::chat_request_with_context(
            crate::official::request(&state, &account, &credential, "/v2/chat/completions"),
            &context,
            "conversation",
            &body,
        )
        .unwrap()
        .build()
        .unwrap();
        let response = transport.send(request, true).await.unwrap();
        assert_eq!(response.status(), 200);
        let bytes = response.bytes().await.unwrap();
        let mut observer = crate::protocol::SseErrors::default();
        assert!(observer.feed(&bytes).is_empty());
        assert_eq!(observer.usage["total_tokens"], 38);
        let text = std::str::from_utf8(&bytes).unwrap();
        let event: Value =
            serde_json::from_str(text.lines().next().unwrap().strip_prefix("data: ").unwrap())
                .unwrap();
        let capture = &event["capture"];
        let headers = &capture["headers"];
        assert_eq!(capture["body"], body);
        assert_eq!(capture["exactNodeBytes"], true);
        assert_eq!(capture["gzipHeaderHex"], "1f8b0800000000000013");
        assert_eq!(headers["authorization"], "Bearer fixture-credential");
        assert_eq!(headers["x-device-token"], "fixture-native-device");
        assert_eq!(
            headers["x-request-id"],
            headers["x-conversation-message-id"]
        );
        assert_eq!(
            headers["x-conversation-request-id"],
            body["messages"][1]["conversationRequestId"]
        );
        assert_eq!(headers["x-b3-parentspanid"], context.parent);
        assert_eq!(headers["acp-connection-id"], context.connection);
        assert!(headers.get("x-wb-bridge-key").is_none());
        let official: Value = serde_json::from_str(include_str!(
            "../evidence/official-desktop-shadow-wire.json"
        ))
        .unwrap();
        let dynamic = [
            "authorization",
            "x-user-id",
            "x-domain",
            "x-device-token",
            "content-length",
            "x-conversation-id",
            "acp-connection-id",
            "x-request-id",
            "x-conversation-message-id",
            "x-conversation-request-id",
            "x-root-request-id",
            "traceparent",
            "b3",
            "x-b3-traceid",
            "x-b3-parentspanid",
            "x-b3-spanid",
            "x-trace-id",
        ];
        let observed = official["records"][0]["originalLogicalHeaders"]
            .as_object()
            .unwrap();
        for (name, value) in observed {
            if !dynamic.contains(&name.as_str()) {
                assert_eq!(&headers[name], value, "header mismatch: {name}");
            }
        }
        for name in headers.as_object().unwrap().keys() {
            assert!(
                observed.contains_key(name) || ["host", "connection"].contains(&name.as_str()),
                "extra header: {name}"
            );
        }
    }
}
impl Transport {
    /// Explicit startup authorization; never sends model traffic or returns the token.
    pub async fn prepare_device(&self) -> Result<(), String> {
        let mut slot = self.0.lock().await;
        if let Some(worker) = slot.as_mut() {
            if worker
                .child
                .try_wait()
                .map_err(|_| "本机传输进程状态异常")?
                .is_some()
            {
                *slot = None;
            }
        }
        if slot.is_none() {
            *slot = Some(Worker::start().await?);
        }
        let worker = slot.as_ref().unwrap();
        let response = worker
            .client
            .post("http://localhost/_device/prepare")
            .header("x-wb-bridge-key", &worker.key)
            .send()
            .await
            .map_err(|_| "设备 SDK 启动授权失败，请重试")?;
        if response.status() != 204 {
            return Err(
                "设备 SDK 未取得有效令牌，请完成系统授权后重新启动反代；也可关闭设备 SDK".into(),
            );
        }
        Ok(())
    }
    pub async fn send(
        &self,
        mut request: reqwest::Request,
        use_device_sdk: bool,
    ) -> Result<reqwest::Response, String> {
        if request.url().scheme() != "https"
            || ![Some("www.workbuddy.cn"), Some("www.workbuddy.ai")]
                .contains(&request.url().host_str())
            || request.url().path() != "/v2/chat/completions"
            || request.url().query().is_some()
            || request.url().port().is_some()
        {
            return Err("官方传输地址不受支持".into());
        }
        let (client, key) = {
            let mut slot = self.0.lock().await;
            if let Some(worker) = slot.as_mut() {
                if worker
                    .child
                    .try_wait()
                    .map_err(|_| "本机传输进程状态异常")?
                    .is_some()
                {
                    *slot = None;
                }
            }
            if slot.is_none() {
                *slot = Some(Worker::start().await?);
            }
            let worker = slot.as_ref().unwrap();
            (worker.client.clone(), worker.key.clone())
        };
        request
            .url_mut()
            .set_scheme("http")
            .map_err(|_| "本机传输地址无效")?;
        request.headers_mut().insert(
            "x-wb-bridge-key",
            key.parse().map_err(|_| "本机传输凭据无效")?,
        );
        // Only trusted application settings select SDK use, never client headers.
        request.headers_mut().remove("x-device-token");
        request.headers_mut().insert(
            "x-wb-device-sdk",
            if use_device_sdk { "1" } else { "0" }.parse().unwrap(),
        );
        // Never retry a sent request: its upstream outcome may be unknown.
        client
            .execute(request)
            .await
            .map_err(|_| "无法连接本机官方传输；请稍后重试".into())
    }
}
