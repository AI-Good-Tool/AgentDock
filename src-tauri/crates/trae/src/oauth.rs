//! Official browser login (PKCE + device key) with a one-shot loopback callback.
use crate::{api::request, client, keys, store::Store};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64U, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

pub struct Login {
    store: Arc<Store>,
    records: Arc<Mutex<HashMap<String, Value>>>,
    pending: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>>,
}

#[derive(Clone)]
struct Flow {
    variant: String,
    verifier: String,
    device_id: String,
    machine_id: String,
    public: String,
    private: String,
    client_id: String,
    version: Value,
    port: u16,
    store: Arc<Store>,
    records: Arc<Mutex<HashMap<String, Value>>>,
    handled: Arc<Mutex<bool>>,
    done: Arc<tokio::sync::Notify>,
}

impl Login {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            records: Arc::default(),
            pending: Arc::default(),
        }
    }
    pub fn records(&self) -> Vec<Value> {
        self.records.lock().unwrap().values().cloned().collect()
    }
    pub fn close(&self) {
        for (_, stop) in self.pending.lock().unwrap().drain() {
            let _ = stop.send(());
        }
    }
    /// Open the official authorization page; the callback saves the account.
    pub async fn start(&self, variant: &str) -> Result<Value, String> {
        if self.pending.lock().unwrap().contains_key(variant) {
            return Err("此版本已有登录窗口，请完成或等待五分钟后重试".into());
        }
        let p = client::profile(variant)?;
        let verifier = B64U.encode(rand::random::<[u8; 32]>()) + &B64U.encode(rand::random::<[u8; 16]>());
        let challenge = B64U.encode(Sha256::digest(verifier.as_bytes()));
        let (public, private) = keys::generate()?;
        let local = client::read_local(variant).ok();
        let device_id = local
            .as_ref()
            .and_then(|l| l["deviceId"].as_str().map(str::to_owned))
            .unwrap_or_else(|| hex::encode(rand::random::<[u8; 16]>()));
        let machine_id = local
            .as_ref()
            .and_then(|l| l["machineId"].as_str().filter(|s| !s.is_empty()).map(str::to_owned))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let client_id = p.product["iCubeApp"]["authConfig"]["TRAE"]["stable"]
            .as_str()
            .unwrap_or("ono9krqynydwx5")
            .to_owned();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "无法启动本地登录回调")?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let flow = Flow {
            variant: variant.into(),
            verifier,
            device_id: device_id.clone(),
            machine_id: machine_id.clone(),
            public,
            private,
            client_id: client_id.clone(),
            version: p.product["appVersion"].clone(),
            port,
            store: self.store.clone(),
            records: self.records.clone(),
            handled: Arc::default(),
            done: Arc::default(),
        };
        self.records.lock().unwrap().insert(
            variant.into(),
            json!({"status":"waiting","variant":variant,"startedAt":chrono::Utc::now().to_rfc3339()}),
        );
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        self.pending.lock().unwrap().insert(variant.into(), stop_tx);
        let app = Router::new()
            .route("/authorize", get(authorize))
            .with_state(flow.clone());
        let pending = self.pending.clone();
        let records = self.records.clone();
        let key = variant.to_owned();
        let expired = key.clone();
        let done = flow.done.clone();
        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                tokio::select! {
                    _ = stop_rx => {},
                    _ = done.notified() => {},
                    _ = tokio::time::sleep(Duration::from_secs(300)) => {
                        if let Some(r) = records.lock().unwrap().get_mut(&expired) {
                            if r["status"] == "waiting" {
                                r["status"] = json!("expired");
                            }
                        }
                    },
                }
            });
            let _ = server.await;
            pending.lock().unwrap().remove(&key);
        });
        let console = p.product["bootConfig"]["consoleHost"]
            .as_str()
            .ok_or("客户端缺少登录地址")?;
        let mut url = reqwest::Url::parse(console)
            .and_then(|u| u.join("/authorization"))
            .map_err(|_| "客户端登录地址无效")?;
        for (k, v) in [
            ("login_version", "1".to_owned()),
            ("auth_from", "trae".into()),
            ("login_channel", "native_ide".into()),
            ("auth_type", "local".into()),
            ("client_id", client_id),
            ("redirect", "0".into()),
            ("login_trace_id", uuid::Uuid::new_v4().to_string()),
            (
                "auth_callback_url",
                format!("http://127.0.0.1:{port}/authorize"),
            ),
            ("machine_id", machine_id.clone()),
            ("device_id", device_id.clone()),
            ("x_device_id", device_id),
            ("x_machine_id", machine_id),
            (
                "x_app_version",
                p.product["appVersion"].as_str().unwrap_or("").into(),
            ),
            ("x_app_type", "stable".into()),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256".into()),
        ] {
            url.query_pairs_mut().append_pair(k, &v);
        }
        if url.scheme() != "https" {
            return Err("客户端登录地址必须是 HTTPS".into());
        }
        crate::platform::open_url(url.as_str())?;
        Ok(json!({"status":"waiting","expiresIn":300}))
    }
}

async fn authorize(
    State(flow): State<Flow>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if headers.get("host").and_then(|v| v.to_str().ok())
        != Some(format!("127.0.0.1:{}", flow.port).as_str())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    {
        let mut handled = flow.handled.lock().unwrap();
        if *handled {
            return StatusCode::CONFLICT.into_response();
        }
        *handled = true;
    }
    let code = query
        .get("authCodeInfo")
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v["AuthCode"].as_str().map(str::to_owned));
    let Some(code) = code else {
        *flow.handled.lock().unwrap() = false;
        return (StatusCode::BAD_REQUEST, "Missing authCodeInfo").into_response();
    };
    let result = exchange(&flow, &code).await;
    let response = match &result {
        Ok(_) => {
            flow.store.event(
                "login",
                &format!("{} 浏览器账号已添加", client::variant_name(&flow.variant)),
            );
            if let Some(r) = flow.records.lock().unwrap().get_mut(&flow.variant) {
                r["status"] = json!("success");
            }
            Html("<h2>账号已添加，可以关闭此窗口并返回 AgentDock。</h2>").into_response()
        }
        Err(e) => {
            flow.store.event(
                "error",
                &format!("{} 浏览器登录失败：{e}", client::variant_name(&flow.variant)),
            );
            if let Some(r) = flow.records.lock().unwrap().get_mut(&flow.variant) {
                r["status"] = json!("error");
                r["error"] = json!(e);
            }
            (
                StatusCode::BAD_REQUEST,
                "登录失败，请返回 AgentDock 查看诊断。",
            )
                .into_response()
        }
    };
    flow.done.notify_one();
    response
}

async fn exchange(flow: &Flow, code: &str) -> Result<(), String> {
    let mut a = json!({"variant":flow.variant,"user":{"token":"","account":{"storeRegion":if flow.variant=="cn"{"CN"}else{"SG"}}}});
    let data = request(
        &a,
        "/trae/api/v3/oauth/ExchangeToken",
        json!({
            "ClientID": flow.client_id,
            "AuthCode": code,
            "CodeVerifier": flow.verifier,
            "DeviceInfo": {
                "DeviceID": flow.device_id,
                "MachineID": flow.machine_id,
                "PlatformCode": "IDE_PC",
                "DeviceType": "PC",
                "DeviceName": crate::api::hostname(),
                "DevicePublicKey": flow.public,
                "ClientVersion": flow.version,
            },
            "IDEVersion": flow.version,
        }),
        "account",
        &[],
    )
    .await?;
    let t = &data["Result"];
    let (Some(token), Some(refresh)) = (t["Token"].as_str(), t["RefreshToken"].as_str()) else {
        return Err("未收到登录凭证".into());
    };
    a["user"]["token"] = json!(token);
    let info = request(
        &a,
        "/cloudide/api/v3/trae/GetUserInfo",
        json!({"ReqSource":"IDE","IDEVersion":flow.version}),
        "account",
        &[],
    )
    .await?;
    let info = &info["Result"];
    if info["UserID"].is_null() {
        return Err("无法取得账号资料".into());
    }
    let region = info["AIRegion"].clone();
    let user = json!({
        "token": token,
        "refreshToken": refresh,
        "expiredAt": crate::api::ms_to_iso(&t["TokenExpireAt"]),
        "refreshExpiredAt": crate::api::ms_to_iso(&t["RefreshExpireAt"]),
        "tokenReleaseAt": chrono::Utc::now().to_rfc3339(),
        "userId": match &info["UserID"] { Value::String(s) => s.clone(), v => v.to_string() },
        "host": client::host_for(&a, "account")?,
        "userRegion": {"region": region, "_aiRegion": region},
        "account": {
            "username": info["ScreenName"],
            "email": info["NonPlainTextEmail"].as_str().unwrap_or(""),
            "avatar_url": info["AvatarUrl"],
            "scope": "marscode",
            "loginScope": "trae",
            "storeRegion": if flow.variant == "cn" { json!("CN") } else { json!(info["AIRegion"].as_str().unwrap_or("SG")) },
            "userTag": "row",
        },
    });
    let saved = flow.store.upsert(json!({
        "variant": flow.variant,
        "user": user,
        "deviceId": flow.device_id,
        "machineId": flow.machine_id,
        "deviceKeys": {"publicKeyPEM": flow.public, "privateKeyPEM": flow.private},
        "capturedAt": chrono::Utc::now().to_rfc3339(),
    }))?;
    flow.store.login_restored(&saved);
    Ok(())
}
