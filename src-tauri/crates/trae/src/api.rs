//! Official Trae account APIs: usage snapshot, daily check-in and
//! device-signed token refresh. Callers persist the returned field updates.
use crate::client::{host_for, profile};
use serde_json::{json, Value};
use std::time::Duration;

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("http client")
}

/// POST JSON to an official host; business errors become concise messages.
pub async fn request(
    account: &Value,
    route: &str,
    body: Value,
    category: &str,
    extra: &[(&str, String)],
) -> Result<Value, String> {
    let token = account["user"]["token"].as_str().unwrap_or("");
    let mut req = http()
        .post(format!("{}{route}", host_for(account, category)?))
        .header("Content-Type", "application/json")
        .json(&body);
    req = if category == "account" {
        req.header("x-cloudide-token", token)
    } else {
        req.header("Authorization", format!("Cloud-IDE-JWT {token}"))
    };
    for (k, v) in extra {
        req = req.header(*k, v);
    }
    let r = req.send().await.map_err(|_| "官方接口连接失败".to_owned())?;
    let status = r.status().as_u16();
    let text = r.text().await.map_err(|_| "官方接口响应读取失败")?;
    let data: Value = serde_json::from_str(&text)
        .map_err(|_| format!("HTTP {status}：返回内容不是 JSON"))?;
    if !(200..300).contains(&status) {
        return Err(format!(
            "HTTP {status}：{}",
            data["message"]
                .as_str()
                .or_else(|| data["ResponseMetadata"]["Error"]["Code"].as_str())
                .unwrap_or("服务拒绝请求")
        ));
    }
    let code = &data["code"];
    if (!code.is_null() && code != 0 && code != "")
        || data["ResponseMetadata"]["Error"].is_object()
    {
        return Err(format!(
            "接口错误：{}",
            if code.is_null() || code == 0 {
                data["ResponseMetadata"]["Error"]["Code"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_owned()
            } else {
                code.to_string().trim_matches('"').to_owned()
            }
        ));
    }
    Ok(data)
}

fn device(account: &Value) -> (&'static str, String) {
    ("x-device-id", account["deviceId"].as_str().unwrap_or("0").to_owned())
}

/// Pay status, usage and (CN) check-in status, each independently tolerated.
pub async fn refresh_snapshot(account: &Value) -> Value {
    let cn = account["variant"] == "cn";
    let version = if cn { "v2" } else { "v1" };
    let mut tasks = vec![
        (
            "pay",
            format!("/trae/api/{version}/pay/ide_user_pay_status"),
            json!({"req_source":1}),
        ),
        (
            "usage",
            format!("/trae/api/{version}/pay/ide_user_ent_usage"),
            json!({"require_usage":true,"req_source":1}),
        ),
    ];
    if cn {
        tasks.push((
            "checkin",
            "/trae/api/v2/ug/checkin_credits/status".into(),
            json!({"req_source":1}),
        ));
    }
    let extra = [device(account)];
    let results = futures_util::future::join_all(
        tasks
            .iter()
            .map(|(_, route, body)| request(account, route, body.clone(), "ug", &extra)),
    )
    .await;
    let time = chrono::Utc::now().to_rfc3339();
    let mut snapshot = json!({"time":time,"source":"official-api","errors":{}});
    for ((key, _, _), result) in tasks.iter().zip(results) {
        match result {
            Ok(v) => snapshot[*key] = v,
            Err(e) => snapshot["errors"][*key] = json!(e),
        }
    }
    snapshot
}

pub fn checkin_headers(account: &Value) -> Vec<(&'static str, String)> {
    vec![
        device(account),
        ("x-device-type", "macOS".into()),
        ("x-device-brand", std::env::consts::ARCH.into()),
    ]
}

/// Status first; claim only when open and not yet claimed; confirm afterwards.
/// Returns (result, new checkin field).
pub async fn checkin(account: &Value) -> Result<(Value, Value), String> {
    if account["variant"] != "cn" {
        return Err("国际版客户端未提供此签到接口".into());
    }
    let headers = checkin_headers(account);
    let status = request(
        account,
        "/trae/api/v2/ug/checkin_credits/status",
        json!({"req_source":1}),
        "ug",
        &headers,
    )
    .await?;
    checkin_with(account, status, |route| {
        let headers = headers.clone();
        async move { request(account, route, json!({"req_source":1}), "ug", &headers).await }
    })
    .await
}

pub async fn checkin_with<'a, F, Fut>(
    _account: &'a Value,
    status: Value,
    send: F,
) -> Result<(Value, Value), String>
where
    F: Fn(&'static str) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    if !status["enable"].is_boolean() || !status["checked_in"].is_boolean() {
        return Err("签到状态格式变化，已停止领取".into());
    }
    if status["enable"] != true {
        return Err("此账号当前未开放签到".into());
    }
    let now = chrono::Utc::now().to_rfc3339();
    if status["checked_in"] == true {
        let mut field = status.clone();
        field["time"] = json!(now);
        let mut result = status;
        result["already"] = json!(true);
        return Ok((result, field));
    }
    let result = send("/trae/api/v2/ug/checkin_credits/claim").await?;
    // Re-read status instead of assuming a successful transport means a successful claim.
    let mut field = send("/trae/api/v2/ug/checkin_credits/status").await?;
    field["time"] = json!(chrono::Utc::now().to_rfc3339());
    if field["checked_in"] != true {
        return Err("领取请求已完成，但服务端尚未确认签到".into());
    }
    Ok((result, field))
}

/// Device-signed refresh. Returns the updated `user` object.
pub async fn refresh_token(account: &Value) -> Result<Value, String> {
    let private = account["deviceKeys"]["privateKeyPEM"].as_str();
    let refresh = account["user"]["refreshToken"].as_str();
    let (Some(private), Some(refresh)) = (private, refresh.filter(|s| !s.is_empty())) else {
        return Err("缺少设备密钥或 refreshToken，请重新采集客户端账号".into());
    };
    let p = profile(account["variant"].as_str().unwrap_or(""))?;
    let client_id = p.product["iCubeApp"]["authConfig"]["TRAE"]["stable"]
        .as_str()
        .unwrap_or("ono9krqynydwx5")
        .to_owned();
    let route = "/trae/api/v3/oauth/ExchangeToken";
    let timestamp = chrono::Utc::now().timestamp();
    let nonce = hex::encode(rand::random::<[u8; 16]>());
    let canonical = [
        "POST",
        route,
        &client_id,
        refresh,
        &timestamp.to_string(),
        &nonce,
    ]
    .join("\n");
    let signature = crate::keys::sign(private, canonical.as_bytes())?;
    let version = p.product["appVersion"].clone();
    let data = request(
        account,
        route,
        json!({
            "ClientID": client_id,
            "ClientSecret": "",
            "RefreshToken": refresh,
            "DeviceInfo": {
                "DeviceID": account["deviceId"],
                "MachineID": account["machineId"],
                "PlatformCode": "IDE_PC",
                "DeviceType": "PC",
                "DeviceName": hostname(),
                "ClientVersion": version,
                "DevicePublicKey": account["deviceKeys"]["publicKeyPEM"],
            },
            "DeviceProof": {"Signature": signature, "Timestamp": timestamp, "Nonce": nonce},
            "IDEVersion": version,
        }),
        "account",
        &[],
    )
    .await?;
    let r = &data["Result"];
    let (Some(token), Some(refresh)) = (r["Token"].as_str(), r["RefreshToken"].as_str()) else {
        return Err("刷新接口未返回有效凭证".into());
    };
    let mut user = account["user"].clone();
    user["token"] = json!(token);
    user["refreshToken"] = json!(refresh);
    user["expiredAt"] = json!(ms_to_iso(&r["TokenExpireAt"]));
    user["refreshExpiredAt"] = json!(ms_to_iso(&r["RefreshExpireAt"]));
    user["tokenReleaseAt"] = json!(chrono::Utc::now().to_rfc3339());
    Ok(user)
}

pub fn ms_to_iso(v: &Value) -> String {
    let ms = v
        .as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn hostname() -> String {
    std::process::Command::new("/bin/hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[tokio::test]
    async fn checkin_queries_then_claims_then_confirms_and_never_repeats() {
        let a = json!({"variant":"cn"});
        let calls = Mutex::new(vec![]);
        let (result, field) = checkin_with(&a, json!({"enable":true,"checked_in":false}), |route| {
            calls.lock().unwrap().push(route);
            async move {
                Ok(if route.ends_with("claim") {
                    json!({})
                } else {
                    json!({"enable":true,"checked_in":true,"credits":100})
                })
            }
        })
        .await
        .unwrap();
        assert!(result.is_object());
        assert_eq!(field["checked_in"], true);
        assert!(field["time"].is_string());
        assert_eq!(
            calls.lock().unwrap().iter().map(|r| r.rsplit('/').next().unwrap()).collect::<Vec<_>>(),
            vec!["claim", "status"]
        );
        let (already, _) = checkin_with(&a, json!({"enable":true,"checked_in":true}), |_| async {
            panic!("must not claim")
        })
        .await
        .unwrap();
        assert_eq!(already["already"], true);
        assert!(checkin_with(&a, json!({"enable":true}), |_| async { Ok(json!({})) })
            .await
            .is_err());
        assert!(checkin(&json!({"variant":"intl"})).await.is_err());
    }
    #[test]
    fn timestamps_are_iso_milliseconds() {
        assert_eq!(ms_to_iso(&json!(2000)), "1970-01-01T00:00:02.000Z");
    }
}
