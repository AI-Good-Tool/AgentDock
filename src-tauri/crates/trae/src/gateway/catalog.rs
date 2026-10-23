//! Retry only the read-only catalog lookup, before any Remote turn is created.
use super::{http, remote_base, remote_headers, valid_model_name, AGENT, MODELS_QUERY};
use agentdock_core::{
    diagnostics::RequestDetails,
    provider::{AccountEffect, SendFailure},
};
use serde_json::{json, Value};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);
const ATTEMPTS: u32 = 2;

fn failure(details: &mut RequestDetails, code: &str, message: &str) -> SendFailure {
    details.local_error("upstream_catalog", message);
    let mut failure = SendFailure::local(502, code, message.to_owned());
    if details.upstream_status == Some(401) {
        failure.effect = AccountEffect::AuthFailed;
    } else if matches!(details.upstream_status, Some(403 | 429)) {
        failure.effect = AccountEffect::Cooldown(60);
    }
    failure
}

fn transport_failure(details: &mut RequestDetails, error: &reqwest::Error) -> SendFailure {
    // Never format reqwest errors: they may contain URLs or credential data.
    if error.is_timeout() {
        failure(details, "upstream_timeout", "官方 Remote 模型目录请求超时")
    } else {
        failure(
            details,
            "upstream_unreachable",
            "官方 Remote 模型目录连接或传输失败",
        )
    }
}

pub(super) async fn fetch(
    account: &Value,
    host_override: Option<&str>,
    details: &mut RequestDetails,
) -> Result<Vec<Value>, SendFailure> {
    fetch_with_timeout(account, host_override, details, TIMEOUT).await
}

async fn fetch_with_timeout(
    account: &Value,
    host_override: Option<&str>,
    details: &mut RequestDetails,
    timeout: Duration,
) -> Result<Vec<Value>, SendFailure> {
    let base = remote_base(account, host_override)
        .map_err(|message| failure(details, "client_unavailable", &message))?;
    let url = format!("{base}{MODELS_QUERY}");
    let client = http();
    for attempt in 1..=ATTEMPTS {
        details.upstream_url = Some(url.clone());
        details.upstream_status = None;
        details.upstream_request_id = None;
        details.content_type = None;
        details.upstream_summary = json!({
            "transport": "trae-remote-agent", "phase": "model_catalog",
            "catalogAttempts": attempt, "catalogTimeoutMs": timeout.as_millis(),
        });
        let mut request = client.get(&url).timeout(timeout);
        for (key, value) in remote_headers(account) {
            request = request.header(key, value);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                if attempt < ATTEMPTS && (error.is_timeout() || error.is_connect()) {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                return Err(transport_failure(details, &error));
            }
        };
        let status = response.status().as_u16();
        details.upstream_status = Some(status);
        details.upstream_request_id = ["x-request-id", "x-tt-logid"]
            .into_iter()
            .find_map(|name| response.headers().get(name)?.to_str().ok())
            .map(|value| agentdock_core::diagnostics::bounded(value, 256));
        details.content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(|value| agentdock_core::diagnostics::bounded(value, 128));
        if !(200..300).contains(&status) {
            if attempt < ATTEMPTS && matches!(status, 502 | 503 | 504) {
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
            return Err(failure(
                details,
                "upstream_http_error",
                &format!("官方 Remote 模型目录查询失败（HTTP {status}）"),
            ));
        }
        let body: Value = match response.json().await {
            Ok(body) => body,
            Err(error) if error.is_timeout() || error.is_body() => {
                if attempt < ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                return Err(transport_failure(details, &error));
            }
            Err(_) => {
                return Err(failure(
                    details,
                    "upstream_protocol_error",
                    "官方 Remote 模型目录响应不是 JSON",
                ))
            }
        };
        if body["code"].as_i64().is_some_and(|code| code != 0)
            || agentdock_core::provider::login_rejected(200, &body)
        {
            let mut error = failure(
                details,
                "upstream_protocol_error",
                "官方 Remote 模型目录返回业务错误",
            );
            error.error = json!({"code":body["code"]});
            if agentdock_core::provider::login_rejected(200, &body) {
                error.effect = AccountEffect::AuthFailed;
            }
            details.error["code"] = body["code"].clone();
            return Err(error);
        }
        let list = body["data"]["list"].as_array().ok_or_else(|| {
            failure(
                details,
                "upstream_protocol_error",
                "官方 Remote 模型目录格式变化",
            )
        })?;
        return Ok(list
            .iter()
            .filter(|group| group["function"] == AGENT)
            .flat_map(|group| group["models"].as_array().cloned().unwrap_or_default())
            .filter(|model| model["name"].as_str().is_some_and(valid_model_name))
            .collect());
    }
    unreachable!("each catalog attempt returns or retries")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, response::IntoResponse, routing::get, Json, Router};
    use std::sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    };

    async fn fixture(
        status: StatusCode,
        slow: bool,
    ) -> (String, Arc<AtomicU32>, tokio::task::JoinHandle<()>) {
        let calls = Arc::new(AtomicU32::new(0));
        let count = calls.clone();
        let app = Router::new().route("/api/remote/v1/models", get(move || {
            let count = count.clone();
            async move {
                let attempt = count.fetch_add(1, Ordering::SeqCst);
                if slow { tokio::time::sleep(Duration::from_millis(100)).await; }
                if attempt == 0 || status == StatusCode::UNAUTHORIZED {
                    return status.into_response();
                }
                Json(json!({"code":0,"data":{"list":[{"function":AGENT,"models":[{"name":"test-model"}]}]}})).into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (host, calls, task)
    }

    #[tokio::test]
    async fn transient_catalog_failure_retries_get_once() {
        let (host, calls, task) = fixture(StatusCode::SERVICE_UNAVAILABLE, false).await;
        let mut details = RequestDetails::default();
        let result = fetch(&json!({"variant":"cn"}), Some(&host), &mut details)
            .await
            .unwrap();
        assert_eq!(result[0]["name"], "test-model");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(details.upstream_summary["catalogAttempts"], 2);
        assert!(details.error_stage.is_none());
        task.abort();
    }

    #[tokio::test]
    async fn auth_rejection_is_not_retried_or_confused_with_transport() {
        let (host, calls, task) = fixture(StatusCode::UNAUTHORIZED, false).await;
        let mut details = RequestDetails::default();
        let error = fetch(
            &json!({"variant":"cn","user":{"token":"SECRET"}}),
            Some(&host),
            &mut details,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(error.code, "upstream_http_error");
        assert_eq!(error.effect, AccountEffect::AuthFailed);
        assert_eq!(details.upstream_status, Some(401));
        assert_eq!(details.error_stage.as_deref(), Some("upstream_catalog"));
        assert!(!serde_json::to_string(&details).unwrap().contains("SECRET"));
        task.abort();
    }

    #[tokio::test]
    async fn timeouts_preserve_catalog_endpoint_and_classification() {
        let (host, calls, task) = fixture(StatusCode::SERVICE_UNAVAILABLE, true).await;
        let mut details = RequestDetails::default();
        let error = fetch_with_timeout(
            &json!({"variant":"cn"}),
            Some(&host),
            &mut details,
            Duration::from_millis(30),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.code, "upstream_timeout");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(details.upstream_status, None);
        assert!(details
            .upstream_url
            .as_deref()
            .unwrap()
            .contains("/api/remote/v1/models?"));
        assert_eq!(details.upstream_summary["catalogAttempts"], 2);
        assert_eq!(details.error_stage.as_deref(), Some("upstream_catalog"));
        task.abort();
    }
}
