//! End-to-end gateway tests over local fixture upstreams (no real accounts,
//! no production traffic): cross-provider routing, scopes, aliases, protocol
//! conversion, authentication and failure isolation.
use crate::{app::Hub, settings::Settings};
use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{atomic::{AtomicBool, AtomicUsize, Ordering}, Arc},
};

const KEY: &str = "fixture-local-key-0123456789";

fn provider_input(model: &str) -> agentdock_core::provider::SendInput {
    agentdock_core::provider::SendInput {
        body: chat(model), headers: Default::default(), request_id: "fixture-stale-review".into(),
        session_id: None, effort_input: Value::Null, client_profile: None,
    }
}

#[tokio::test]
async fn stale_qoder_http_and_stream_401_cannot_disable_replaced_credentials() {
    use agentdock_core::provider::{credential_revision, ProviderKind};
    use futures_util::StreamExt;
    for model in ["q-auth-http", "q-auth-stream", "q-auth-late"] {
        let f = fixture().await;
        let input = provider_input(model);
        let mut details = agentdock_core::diagnostics::RequestDetails::incoming("openai-chat", "/v1/chat/completions", &input.body);
        let revision = match f.hub.qoder.send("cn:u1", json!({"id":model}), input, &mut details, std::time::Instant::now()).await {
            Err(failure) => {
                assert_eq!(failure.effect, agentdock_core::provider::AccountEffect::AuthFailed);
                failure.credential_revision
            }
            Ok(mut response) => loop {
                match response.events.next().await.expect("fixture must reject") {
                    Ok(_) => continue,
                    Err(failure) => {
                        assert_eq!(failure.status, Some(401));
                        break failure.credential_revision;
                    }
                }
            },
        };
        assert_eq!(revision, Some(credential_revision("never-real")));
        // Relogin finishes after the old request used its credential snapshot.
        f.hub.qoder.fixture_accounts.lock().unwrap().as_mut().unwrap()[0].token = "fresh-login-fixture".into();
        f.hub.disable_account_auth_observed(ProviderKind::Qoder, "cn:u1", revision);
        assert_eq!(f.hub.account_runtime()["qoder/cn:u1"]["authFailed"], false, "{model}");
        assert_eq!(f.hub.account_runtime()["qoder/cn:u1"]["enabled"], true);
        assert!(f.hub.settings().auth_disabled_accounts.is_empty());
        assert!(f.hub.qoder.config().disabled_accounts.is_empty());
        // A rejection of the current credential still suspends it durably.
        f.hub.disable_account_auth_observed(ProviderKind::Qoder, "cn:u1", Some(credential_revision("fresh-login-fixture")));
        assert_eq!(f.hub.account_runtime()["qoder/cn:u1"]["enabled"], false);
        assert!(crate::app::read_settings(&f.hub.dir.join("settings.json")).auth_disabled_accounts.contains("qoder/cn:u1"));
    }
}

#[tokio::test]
async fn stale_trae_catalog_session_and_event_401_cannot_disable_replaced_credentials() {
    use agentdock_core::provider::{credential_revision, ProviderKind};
    for mode in [401, 402, 404, 405, 406] {
        let f = fixture().await;
        f.trae_plan_mode.store(mode, Ordering::SeqCst);
        let id = f.hub.trae.pool_accounts()[0].id.clone();
        let input = provider_input("shared-real");
        let mut details = agentdock_core::diagnostics::RequestDetails::incoming("openai-chat", "/v1/chat/completions", &input.body);
        let failure = match f.hub.trae.send(&id, Some("shared-real".into()), input, &mut details, std::time::Instant::now()).await {
            Ok(_) => panic!("fixture must reject"),
            Err(failure) => failure,
        };
        assert_eq!(failure.effect, agentdock_core::provider::AccountEffect::AuthFailed);
        assert_eq!(failure.credential_revision, Some(credential_revision("never-real")));
        f.hub.trae.store.patch_account(&id, |a| a["user"]["token"] = json!("fresh-login-fixture")).unwrap();
        f.hub.disable_account_auth_observed(ProviderKind::Trae, &id, failure.credential_revision);
        let key = format!("trae/{id}");
        assert_eq!(f.hub.account_runtime()[&key]["authFailed"], false, "{mode}");
        assert_eq!(f.hub.account_runtime()[&key]["enabled"], true);
        assert!(f.hub.settings().auth_disabled_accounts.is_empty());
        assert_ne!(f.hub.trae.store.account(&id).unwrap()["enabled"], false);
        f.hub.disable_account_auth_observed(ProviderKind::Trae, &id, Some(credential_revision("fresh-login-fixture")));
        assert_eq!(f.hub.account_runtime()[&key]["enabled"], false);
        assert!(crate::app::read_settings(&f.hub.dir.join("settings.json")).auth_disabled_accounts.contains(&key));
    }
}

#[tokio::test]
async fn stale_expiry_or_credential_failure_does_not_disable_repaired_workbuddy_login() {
    use agentdock_core::provider::{credential_revision, ProviderKind};
    let f = fixture().await;
    f.hub.wb.update_store(|store| {
        store.accounts.push(serde_json::from_value(json!({"id":"late","name":"fixture","source":"test","sourceId":"fixture-only","variant":"cn","enabled":true,"expiresAt":1})).unwrap());
        Ok(())
    }).unwrap();
    // The scheduler saw an expired snapshot, then a login updated the metadata.
    f.hub.wb.update_store(|store| { store.accounts.iter_mut().find(|a| a.id == "late").unwrap().expires_at = Some(chrono::Utc::now().timestamp_millis()+86400000); Ok(()) }).unwrap();
    f.hub.disable_expired_account(ProviderKind::Workbuddy,"late");
    assert_eq!(f.hub.account_runtime()["workbuddy/late"]["enabled"],true);
    // No matching current credential can confirm the retired request's failure.
    f.hub.disable_account_auth_observed(ProviderKind::Workbuddy,"late",Some(credential_revision("retired-fixture-token")));
    assert_eq!(f.hub.account_runtime()["workbuddy/late"]["authFailed"],false);
    assert!(f.hub.settings().auth_disabled_accounts.is_empty());
}

struct Fixture {
    hub: crate::app::Shared,
    url: String,
    qoder_calls: Arc<AtomicUsize>,
    trae_calls: Arc<AtomicUsize>,
    trae_queries: Arc<std::sync::Mutex<Vec<Value>>>,
    trae_uploads: Arc<AtomicUsize>,
    trae_upload_fail: Arc<AtomicBool>,
    trae_plan_mode: Arc<AtomicUsize>,
    trae_stops: Arc<AtomicUsize>,
    trae_deletes: Arc<AtomicUsize>,
    trae_cleanup_status: Arc<AtomicUsize>,
    trae_events: Arc<AtomicUsize>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    (url, tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }))
}

async fn fixture() -> Fixture {
    let qoder_calls = Arc::new(AtomicUsize::new(0));
    let qc = qoder_calls.clone();
    let queued = Arc::new(AtomicUsize::new(0));
    let qoder_upstream = Router::new().route(
        "/model/v1/chat/completions",
        post(move |Json(body): Json<Value>| {
            let qc = qc.clone();
            let queued = queued.clone();
            async move {
                qc.fetch_add(1, Ordering::SeqCst);
                let sse = [("retry-after", "0"), ("content-type", "text/event-stream")];
                if body["model"] == "q-auth-http" {
                    return (axum::http::StatusCode::UNAUTHORIZED, sse, json!({"code":"token_expired"}).to_string());
                }
                if body["model"] == "q-auth-stream" || body["model"] == "q-auth-late" {
                    let prefix = if body["model"] == "q-auth-late" {
                        format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"content":"partial"}}]}))
                    } else { String::new() };
                    let frame = json!({"statusCodeValue":401,"body":"token expired"});
                    return (axum::http::StatusCode::OK, sse, format!("{prefix}data: {frame}\n\n"));
                }
                // Qoder reports model permission and queueing inside an HTTP 200 stream.
                if body["model"] == "q-gate" {
                    let frame = json!({"statusCodeValue":403,"body":"model not allowed"});
                    return (axum::http::StatusCode::OK, sse, format!("data: {frame}\n\n"));
                }
                if body["model"] == "q-queue" && queued.fetch_add(1, Ordering::SeqCst) == 0 {
                    let notice = json!({"message": json!({"code":"10605","isQueued":true,"retryAfterSeconds":1}).to_string()});
                    let frame = json!({"statusCodeValue":403,"body":notice.to_string()});
                    return (axum::http::StatusCode::OK, sse, format!("data: {frame}\n\n"));
                }
                if body["model"] == "q-flaky" {
                    return (
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        [("retry-after", "30"), ("content-type", "application/json")],
                        json!({"code":"rate_limited"}).to_string(),
                    );
                }
                let mut chunks = vec![];
                if body["tools"].as_array().is_some_and(|t| !t.is_empty())
                    && !body["messages"].as_array().unwrap().iter().any(|m| m["role"] == "tool")
                {
                    let tool = &body["tools"][0]["function"];
                    chunks.push(json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":tool["name"],"arguments":"{}"}}]},"finish_reason":"tool_calls"}]}));
                } else {
                    let effort = body["reasoning_effort"].as_str().map(|e| format!(" effort={e}")).unwrap_or_default();
                    chunks.push(json!({"choices":[{"index":0,"delta":{"content":format!("OK from {}{effort}", body["model"].as_str().unwrap())}}]}));
                    chunks.push(json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}));
                }
                chunks.push(json!({"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}}));
                let wire = chunks.iter().map(|c| format!("data: {c}\n\n")).collect::<String>() + "data: [DONE]\n\n";
                (axum::http::StatusCode::OK, [("retry-after", "0"), ("content-type", "text/event-stream")], wire)
            }
        }),
    );
    let trae_calls = Arc::new(AtomicUsize::new(0));
    let tc = trae_calls.clone();
    let trae_queries: Arc<std::sync::Mutex<Vec<Value>>> = Arc::default();
    let queries = trae_queries.clone();
    let trae_uploads = Arc::new(AtomicUsize::new(0));
    let uploads = trae_uploads.clone();
    let trae_upload_fail = Arc::new(AtomicBool::new(false));
    let upload_fail = trae_upload_fail.clone();
    let trae_plan_mode = Arc::new(AtomicUsize::new(0));
    let plan_mode = trae_plan_mode.clone();
    let trae_stops = Arc::new(AtomicUsize::new(0));
    let stops = trae_stops.clone();
    let trae_deletes = Arc::new(AtomicUsize::new(0));
    let deletes = trae_deletes.clone();
    let trae_cleanup_status = Arc::new(AtomicUsize::new(204));
    let cleanup_status = trae_cleanup_status.clone();
    let create_mode = trae_plan_mode.clone();
    let catalog_mode = trae_plan_mode.clone();
    let trae_events = Arc::new(AtomicUsize::new(0));
    let events_started = trae_events.clone();
    // Trae Remote Agent chain: models → chat_sessions → events (one turn per request).
    let sessions: Arc<std::sync::Mutex<HashMap<String, (String, String)>>> = Arc::default();
    let created = sessions.clone();
    let delete_sessions = sessions.clone();
    let message_sessions = sessions.clone();
    let trae_upstream = Router::new()
        .route(
            "/api/ide/v1/get_resource_upload_url",
            post(|headers: axum::http::HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers["authorization"], "Cloud-IDE-JWT never-real");
                assert_eq!(body["biz_type"], "image");
                assert_eq!(body["scale_param"], json!({"width":16,"height":16}));
                let host = headers["host"].to_str().unwrap();
                Json(json!({"code":0,"data":{
                    "upload_hosts":[format!("http://{host}")],
                    "store_infos":[{"store_uri":"fixture-image","override_resource_id":"remote-image-1","auth":"scoped-upload-token"}],
                    "session_key":"upload-session",
                }}))
            }),
        )
        .route(
            "/fixture-image",
            axum::routing::put(move |headers: axum::http::HeaderMap, bytes: axum::body::Bytes| {
                let (uploads, upload_fail) = (uploads.clone(), upload_fail.clone());
                async move {
                    assert_eq!(headers["authorization"], "scoped-upload-token");
                    assert_eq!(headers["content-type"], "image/png");
                    assert_eq!(headers["content-crc32"], "3187ef75");
                    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
                    uploads.fetch_add(1, Ordering::SeqCst);
                    if upload_fail.load(Ordering::SeqCst) {
                        axum::http::StatusCode::BAD_GATEWAY
                    } else {
                        axum::http::StatusCode::OK
                    }
                }
            }),
        )
        .route(
            "/api/ide/v1/commit_resource_upload_result",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body, json!({"oids":["fixture-image"],"success_oids":["fixture-image"],"session_key":"upload-session","biz_type":"image"}));
                Json(json!({"code":0}))
            }),
        )
        .route(
            "/api/remote/v1/models",
            axum::routing::get(move || {
                let catalog_mode = catalog_mode.clone();
                async move {
                use axum::response::IntoResponse;
                let status = catalog_mode.load(Ordering::SeqCst) as u16;
                if [401, 403, 429, 500].contains(&status) {
                    return (axum::http::StatusCode::from_u16(status).unwrap(), Json(json!({"code":status}))).into_response();
                }
                Json(json!({"code":0,"data":{"list":[
                    {"function":"solo_work_remote","models":[{"name":"work-only"}]},
                    {"function":"solo_agent_remote","models":[
                        {"name":"shared-real","display_name":"Shared","is_default":true,"config_source":1},
                        {"name":"flaky-real","display_name":"Flaky"}]}]}})).into_response()
                }
            }),
        )
        .route(
            "/api/remote/v1/chat_sessions",
            post(move |Json(body): Json<Value>| {
                let (tc, created, queries, create_mode) = (tc.clone(), created.clone(), queries.clone(), create_mode.clone());
                async move {
                    tc.fetch_add(1, Ordering::SeqCst);
                    use axum::response::IntoResponse;
                    if create_mode.load(Ordering::SeqCst) == 402 {
                        return (axum::http::StatusCode::UNAUTHORIZED, [("content-type", "text/html")], "<html>Unauthorized</html>").into_response();
                    }
                    if create_mode.load(Ordering::SeqCst) == 404 {
                        return Json(json!({"code":"TOKEN_EXPIRED"})).into_response();
                    }
                    let m = &body["initial_message"];
                    assert_eq!(m["custom_model"]["name"], m["model_name"], "the full catalog entry is sent");
                    let query: Value = serde_json::from_str(m["query"].as_str().unwrap()).unwrap();
                    queries.lock().unwrap().push(query.clone());
                    let sid = uuid::Uuid::new_v4().simple().to_string();
                    created.lock().unwrap().insert(
                        sid.clone(),
                        (m["model_name"].as_str().unwrap().to_owned(), query[0]["data"]["content"].as_str().unwrap().to_owned()),
                    );
                    if create_mode.load(Ordering::SeqCst) == 5 {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                    if create_mode.load(Ordering::SeqCst) == 4 {
                        return Json(json!({"code":0,"data":{"chat_session_id":sid}})).into_response();
                    }
                    Json(json!({"code":0,"data":{"chat_session_id":sid,"message_id":"m1"}})).into_response()
                }
            }),
        )
        .route(
            "/api/remote/v1/chat_sessions/{sid}/events",
            axum::routing::get(move |axum::extract::Path(sid): axum::extract::Path<String>, headers: axum::http::HeaderMap| {
                let sessions = sessions.clone();
                let plan_mode = plan_mode.clone();
                let events_started = events_started.clone();
                async move {
                    let connection = events_started.fetch_add(1,Ordering::SeqCst);
                    use axum::response::IntoResponse;
                    // Mode 9: the stream loses every content event of a long
                    // turn; only the message list holds the finished reply.
                    if plan_mode.load(Ordering::SeqCst) == 9 {
                        let wire = if connection == 0 {
                            "event: status_changed\ndata: {\"new_status\":3}\n\nevent: metadata\ndata: {}\n\n"
                        } else {
                            "event: status_changed\ndata: {\"new_status\":5}\n\n"
                        };
                        return ([("content-type", "text/event-stream")], wire).into_response();
                    }
                    if plan_mode.load(Ordering::SeqCst) == 405 {
                        return axum::http::StatusCode::UNAUTHORIZED.into_response();
                    }
                    if plan_mode.load(Ordering::SeqCst) == 406 {
                        return ([("content-type", "text/event-stream")], "event: error\ndata: {\"code\":\"TOKEN_EXPIRED\"}\n\n").into_response();
                    }
                    if plan_mode.load(Ordering::SeqCst) == 6 {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                    let (model, query) = sessions.lock().unwrap().get(&sid).cloned().unwrap();
                    let reply = match query.split_once("\nTools: ") {
                        Some((_, tools)) if !query.contains("[Client tool result]") => {
                            let tools: Value = serde_json::from_str(tools).unwrap();
                            format!("<client_tool_call>{}</client_tool_call>", json!({"name": tools[0]["name"], "input": {}}))
                        }
                        _ => format!("你好 from {model}"),
                    };
                    let mut events = vec![
                        ("model_config", json!({"config_name": model})),
                        ("plan_item", json!({"id":"p1","thought":"thinking"})),
                    ];
                    let mode = plan_mode.load(Ordering::SeqCst);
                    if mode == 2 {
                        events.push(("plan_item", json!({"id":"p3","tool_call_info":{"name":"read_file","params":{"path":"PRIVATE_TOOL_ARGUMENT"}}})));
                    }
                    if mode == 1 || mode == 3 {
                        events.push(("plan_item", json!({"id":"p2","tool_call_info":{"id":"finish-1","name":"finish","params":{}}})));
                    }
                    if mode != 3 {
                        events.push(("plan_item", if mode == 1 {
                            json!({"id":"p2","tool_call_info":{"id":"finish-1","params":{"summary":reply}}})
                        } else {
                            json!({"id":"p2","tool_call_info":{"name":"finish","params":{"summary":reply}}})
                        }));
                    }
                    events.push(("token_usage", json!({"prompt_tokens":3,"completion_tokens":2})));
                    // Mode 8 ends via the session status only, like the official terminal rule.
                    events.push(if mode == 8 { ("status_changed", json!({"new_status":5})) } else { ("done", json!({"status":"completed"})) });
                    // Mode 7 closes after two events; only a correct Last-Event-ID resumes it.
                    let resume = headers.get("last-event-id").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<usize>().ok());
                    let (skip, take) = match (mode, resume) {
                        (7, None) => (0, 2),
                        (7, Some(n)) => (n, usize::MAX),
                        _ => (0, usize::MAX),
                    };
                    let wire = events
                    .iter()
                    .enumerate()
                    .skip(skip)
                    .take(take)
                    .map(|(i, (e, d))| if mode == 7 { format!("id: {}\nevent: {e}\ndata: {d}\n\n", i + 1) } else { format!("event: {e}\ndata: {d}\n\n") })
                    .collect::<String>();
                    ([("content-type", "text/event-stream")], wire).into_response()
                }
            }),
        )
        .route("/api/remote/v1/chat_sessions/{sid}", axum::routing::delete(move |axum::extract::Path(sid):axum::extract::Path<String>| {
            let (deletes, status, sessions) = (deletes.clone(),cleanup_status.clone(),delete_sessions.clone());
            async move {
                assert!(sessions.lock().unwrap().contains_key(&sid), "only delete a session created by this fixture request");
                deletes.fetch_add(1,Ordering::SeqCst);
                let code = status.load(Ordering::SeqCst) as u16;
                if code == 204 { sessions.lock().unwrap().remove(&sid); }
                axum::http::StatusCode::from_u16(code).unwrap()
            }
        }).get(|| async { Json(json!({"code":0,"data":{"status":5}})) }))
        .route("/api/remote/v1/chat_sessions/{sid}/messages", axum::routing::get(move |axum::extract::Path(sid): axum::extract::Path<String>| {
            let sessions = message_sessions.clone();
            async move {
                let (model, _) = sessions.lock().unwrap().get(&sid).cloned().unwrap();
                let content = json!({"messages":[
                    {"type":"plan_item","plan_item":{"id":"p1","thought":"thinking"}},
                    {"type":"plan_item","plan_item":{"id":"p2","tool_call_info":{"name":"finish","params":{"summary":format!("recovered from {model}")}}}},
                ]});
                Json(json!({"code":0,"data":{"items":[
                    {"role":"user","status":"in_progress"},
                    {"role":"assistant","status":"completed","content":content.to_string(),
                     "user_message_context":json!({"model_info":{"config_name":model}}).to_string(),
                     "token_usage":json!({"prompt_tokens":7,"completion_tokens":4}).to_string()},
                ]}}))
            }
        }))
        .route("/api/remote/v1/chat_sessions/{sid}/stop", post(move || {
            let stops = stops.clone();
            async move {
                stops.fetch_add(1, Ordering::SeqCst);
                Json(json!({"code":0}))
            }
        }));
    let (qoder_url, q_task) = serve(qoder_upstream).await;
    let (trae_url, t_task) = serve(trae_upstream).await;
    let dir = std::env::temp_dir().join(format!("agentdock-e2e-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let qoder = qoder::gateway::Provider::fixture(
        vec![
            qoder::qcore::Account { edition: "cn".into(), uid: "u1".into(), name: "Qoder CN".into(), token: "never-real".into(), ..Default::default() },
            qoder::qcore::Account { edition: "intl".into(), uid: "u2".into(), name: "Qoder Intl".into(), token: "never-real".into(), ..Default::default() },
        ],
        qoder::gateway::Config {
            transport: "modelServer".into(),
            cn_endpoint: qoder_url.clone(),
            intl_endpoint: qoder_url,
            disabled_accounts: vec![],
        },
        HashMap::from([
            ("cn".into(), vec![json!({"id":"q-shared","name":"Shared"}), json!({"id":"q-only","name":"QoderOnly"}), json!({"id":"q-flaky","name":"Flaky"}), json!({"id":"q-gate","name":"Gate"}), json!({"id":"q-queue","name":"Queued"}), json!({"id":"q-auth-http","name":"AuthHttp"}), json!({"id":"q-auth-stream","name":"AuthStream"}), json!({"id":"q-auth-late","name":"AuthLate"}), json!({"id":"q-think","name":"Think","reasoning":{"supportedEfforts":["low","medium","xhigh","none"],"defaultEffort":"medium"}})]),
            ("intl".into(), vec![json!({"id":"q-shared-i","name":"Shared"}), json!({"id":"q-gate-i","name":"Gate"})]),
        ]),
    );
    let trae = trae::Provider::open(&dir.join("trae")).unwrap();
    trae.store
        .upsert(json!({"variant":"cn","user":{"userId":"t1","token":"never-real","expiredAt":"2099-01-01T00:00:00Z","account":{"username":"Trae CN","storeRegion":"CN"}},
            "models":[{"id":"cfg-shared","model":"shared-real","name":"Shared"},{"id":"cfg-flaky","model":"flaky-real","name":"Flaky"}]}))
        .unwrap();
    *trae.test_host.lock().unwrap() = Some(trae_url);
    let hub = Hub::assemble(&dir, Settings::default(), KEY, qoder, trae);
    let (url, g_task) = serve(crate::server::router(hub.clone())).await;
    Fixture { hub, url, qoder_calls, trae_calls, trae_queries, trae_uploads, trae_upload_fail, trae_plan_mode, trae_stops, trae_deletes, trae_cleanup_status, trae_events, tasks: vec![q_task, t_task, g_task] }
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}
async fn post_json(f: &Fixture, path: &str, body: Value) -> reqwest::Response {
    client().post(format!("{}{path}", f.url)).bearer_auth(KEY).json(&body).send().await.unwrap()
}
fn chat(model: &str) -> Value {
    json!({"model":model,"messages":[{"role":"user","content":"PRIVATE PROMPT"}]})
}
async fn wait_logs(f: &Fixture, n: usize) -> Vec<crate::logs::RequestLog> {
    for _ in 0..100 {
        let logs = f.hub.logs.list();
        if logs.len() >= n {
            return logs;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    f.hub.logs.list()
}

#[tokio::test]
async fn identical_models_rotate_across_every_provider_and_region() {
    let f = fixture().await;
    let mut seen = vec![];
    for i in 0..6 {
        let r = post_json(&f, "/v1/chat/completions", chat("Shared")).await;
        assert_eq!(r.status(), 200);
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["model"], "Shared", "alias is returned, never the real id");
        let logs = wait_logs(&f, i + 1).await;
        let l = &logs[0];
        assert_eq!(l.model, "Shared");
        assert_eq!(l.outcome, "completed");
        seen.push(format!("{}/{}/{}", l.provider, l.region, l.real_model));
    }
    assert_eq!(
        seen,
        ["qoder/cn/q-shared", "qoder/intl/q-shared-i", "trae/cn/shared-real", "qoder/cn/q-shared", "qoder/intl/q-shared-i", "trae/cn/shared-real"]
    );
    // Prompts never reach the log.
    assert!(!serde_json::to_string(&f.hub.logs.list()).unwrap().contains("PRIVATE PROMPT"));
    // Usage is recorded for statistics.
    let stats = crate::logs::statistics(&f.hub.logs.usage_rows(), &json!({}), 480);
    assert_eq!(stats["summary"]["records"], 6);
    assert_eq!(stats["providers"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn exclusive_models_and_scoped_paths_never_widen_the_pool() {
    let f = fixture().await;
    for _ in 0..3 {
        let v: Value = post_json(&f, "/v1/chat/completions", chat("QoderOnly")).await.json().await.unwrap();
        assert!(v["choices"][0]["message"]["content"].as_str().unwrap().contains("q-only"));
    }
    assert_eq!(f.trae_calls.load(Ordering::SeqCst), 0);
    // Region scope: only international accounts.
    for _ in 0..2 {
        let v: Value = post_json(&f, "/intl/v1/chat/completions", chat("Shared")).await.json().await.unwrap();
        assert!(v["choices"][0]["message"]["content"].as_str().unwrap().contains("q-shared-i"));
    }
    // Provider scope: Trae only; a model Trae lacks is unavailable there.
    let v: Value = post_json(&f, "/trae/v1/chat/completions", chat("Shared")).await.json().await.unwrap();
    assert!(v["choices"][0]["message"]["content"].as_str().unwrap().contains("你好 from shared-real"));
    let r = post_json(&f, "/trae/v1/chat/completions", chat("QoderOnly")).await;
    assert_eq!(r.status(), 400);
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], "model_not_available");
    // Scoped model lists.
    let list: Value = client().get(format!("{}/trae/cn/v1/models", f.url)).bearer_auth(KEY).send().await.unwrap().json().await.unwrap();
    let ids: Vec<_> = list["data"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_owned()).collect();
    assert!(ids.contains(&"Shared".into()) && !ids.contains(&"QoderOnly".into()), "{ids:?}");
    // A pinned account outside the model's pool is rejected without upstream traffic.
    let before = f.qoder_calls.load(Ordering::SeqCst) + f.trae_calls.load(Ordering::SeqCst);
    let r = client().post(format!("{}/v1/chat/completions", f.url)).bearer_auth(KEY).header("x-account-id", "qoder/intl:u2").json(&chat("QoderOnly")).send().await.unwrap();
    assert_eq!(r.status(), 503);
    assert_eq!(before, f.qoder_calls.load(Ordering::SeqCst) + f.trae_calls.load(Ordering::SeqCst));
}

#[tokio::test]
async fn aliases_apply_to_requests_lists_logs_and_all_protocols() {
    let f = fixture().await;
    let routes = f.hub.routes(&f.hub.settings());
    let shared = crate::catalog::resolve(&routes, "Shared").unwrap().route_id.clone();
    f.hub
        .save_route(&shared, crate::settings::RouteOverride { alias: "my-shared".into(), ..Default::default() })
        .unwrap();
    let list: Value = client().get(format!("{}/v1/models", f.url)).header("x-api-key", KEY).send().await.unwrap().json().await.unwrap();
    let ids: Vec<_> = list["data"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_owned()).collect();
    assert!(ids.contains(&"my-shared".to_owned()) && !ids.contains(&"Shared".to_owned()));
    for stream in [false, true] {
        for (path, body) in [
            ("/qoder/v1/messages", json!({"model":"my-shared","max_tokens":32,"messages":[{"role":"user","content":"hi"}],"stream":stream})),
            ("/qoder/v1/responses", json!({"model":"my-shared","input":"hi","stream":stream})),
            ("/trae/v1/responses", json!({"model":"my-shared","input":"hi","stream":stream})),
            ("/trae/v1/messages", json!({"model":"my-shared","max_tokens":32,"messages":[{"role":"user","content":"hi"}],"stream":stream})),
            ("/v1/chat/completions", json!({"model":"my-shared","messages":[{"role":"user","content":"hi"}],"stream":stream})),
        ] {
            let r = post_json(&f, path, body).await;
            assert_eq!(r.status(), 200, "{path} stream={stream}");
            let text = r.text().await.unwrap();
            assert!(text.contains("my-shared"), "{path}: {text}");
            assert!(!text.contains("q-shared") || text.contains("OK from"), "real ids only appear in generated text");
            if stream {
                assert!(
                    text.contains(if path.ends_with("messages") { "message_stop" } else if path.ends_with("responses") { "response.completed" } else { "[DONE]" }),
                    "{path}: {text}"
                );
            }
        }
    }
    let logs = wait_logs(&f, 10).await;
    assert!(logs.iter().all(|l| l.model == "my-shared" && l.outcome == "completed"), "{logs:?}");
    // Real model ids still resolve when unambiguous.
    assert_eq!(post_json(&f, "/v1/chat/completions", chat("q-shared-i")).await.status(), 200);
}

#[tokio::test]
async fn clearing_checkin_history_keeps_account_status_and_other_platforms() {
    let f = fixture().await;
    let record = workbuddy::checkin::Record { at: 1, account_id: "a".into(), submit: true, status: Some(json!({"today_checked_in":true})), error: None, trigger: Some("manual".into()) };
    // "legacy" predates per-account state: its status exists only in the history.
    let legacy = [
        workbuddy::checkin::Record { at: 2, account_id: "legacy".into(), status: Some(json!({"today_checked_in":true})), ..record.clone() },
        workbuddy::checkin::Record { at: 3, account_id: "legacy".into(), status: None, error: Some("offline".into()), ..record.clone() },
    ];
    f.hub.wb.update_store(|s| {
        s.checkin_states.entry("a".into()).or_default().observe(&record);
        s.checkin_records.push(record.clone());
        s.checkin_records.extend(legacy.iter().cloned());
        Ok(())
    }).unwrap();
    f.hub.trae.store.update(|d| { d["checkinHistory"] = json!([{"accountId":"t","status":"success"}]); Ok(()) }).unwrap();
    assert!(f.hub.clear_checkin_history(Some("unknown")).is_err());
    // Qoder's claim log lives in the user's Qoder data directory, so tests clear only the others.
    f.hub.clear_checkin_history(Some("workbuddy")).unwrap();
    assert!(f.hub.wb.store.lock().unwrap().checkin_records.is_empty());
    assert!(f.hub.wb.store.lock().unwrap().checkin_states["a"].status.is_some());
    let kept = f.hub.wb.store.lock().unwrap().checkin_states["legacy"].clone();
    assert_eq!((kept.updated_at, kept.error_at, kept.status.unwrap()["today_checked_in"].clone()), (Some(2), Some(3), json!(true)));
    assert_eq!(f.hub.trae.store.snapshot()["checkinHistory"].as_array().unwrap().len(), 1);
    f.hub.clear_checkin_history(Some("trae")).unwrap();
    assert!(f.hub.trae.store.snapshot()["checkinHistory"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn lan_access_is_off_by_default_and_changes_only_while_stopped() {
    let f = fixture().await;
    assert!(!f.hub.settings().allow_lan);
    let (stop, _rx) = tokio::sync::oneshot::channel();
    *f.hub.server.lock().unwrap() = Some(crate::app::ServerHandle { port: 1, lan: false, stop });
    let err = f.hub.update_settings(|s| { s.allow_lan = true; Ok(()) }).unwrap_err();
    assert!(err.contains("局域网"), "{err}");
    assert!(!f.hub.settings().allow_lan);
    f.hub.server.lock().unwrap().take();
    assert!(f.hub.update_settings(|s| { s.allow_lan = true; Ok(()) }).unwrap().allow_lan);
}

#[tokio::test]
async fn each_api_key_uses_its_own_scope_and_strategy_and_is_recorded() {
    use crate::api_key::KeyFields;
    use crate::settings::Strategy;
    let f = fixture().await;
    let scope = std::collections::BTreeMap::from([(agentdock_core::provider::ProviderKind::Trae, vec!["cn".to_owned()])]);
    let ci = f.hub.keys.create(None, KeyFields { alias: "ci".into(), enabled: true, strategy: Some(Strategy::Ordered), scope: Some(scope) }).unwrap();
    let send = |key: String, model: &'static str| {
        let url = format!("{}/v1/chat/completions", f.url);
        async move { client().post(url).bearer_auth(key).json(&chat(model)).send().await.unwrap() }
    };
    for _ in 0..3 {
        let r = send(ci.key.clone(), "Shared").await;
        assert_eq!(r.status(), 200);
    }
    // Models outside the key's scope are neither listed nor served.
    let models: Value = client().get(format!("{}/v1/models", f.url)).header("x-api-key", &ci.key).send().await.unwrap().json().await.unwrap();
    let listed: Vec<&str> = models["data"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
    assert!(listed.contains(&"Shared") && !listed.contains(&"QoderOnly"), "{listed:?}");
    assert_eq!(send(ci.key.clone(), "QoderOnly").await.status(), 400);
    // The default key keeps the global scope.
    assert_eq!(send(KEY.into(), "QoderOnly").await.status(), 200);
    let logs = wait_logs(&f, 5).await;
    let by_ci: Vec<_> = logs.iter().filter(|l| l.key_id == ci.id).collect();
    assert_eq!(by_ci.len(), 4);
    for l in by_ci.iter().filter(|l| l.status == 200) {
        assert_eq!((l.provider.as_str(), l.region.as_str(), l.strategy.as_str()), ("trae", "cn", "Ordered"));
        assert_eq!((l.key_alias.as_str(), l.key_hint.as_str()), ("ci", crate::api_key::abbreviate(&ci.key).as_str()));
    }
    let default = logs.iter().find(|l| l.key_id == "default").unwrap();
    assert_eq!((default.key_alias.as_str(), default.strategy.as_str()), ("", "RoundRobin"));
    assert!(!serde_json::to_string(&logs).unwrap().contains(&ci.key), "records never hold the key itself");
    let stats = crate::logs::statistics(&f.hub.logs.usage_rows(), &json!({"keyId": ci.id}), 0);
    assert_eq!(stats["summary"]["records"], 4);
    assert_eq!(stats["apiKeys"][0]["label"], "ci");
    // A global pin outside the key's scope does not lock the key out.
    let qoder = f.hub.pool_accounts().into_iter().find(|a| a.provider == agentdock_core::provider::ProviderKind::Qoder).unwrap().key();
    f.hub.update_settings(|s| {
        s.pinned.insert("all".into(), qoder.clone());
        Ok(())
    }).unwrap();
    assert_eq!(send(ci.key.clone(), "Shared").await.status(), 200);
    assert_eq!(send(KEY.into(), "QoderOnly").await.status(), 200);
    let logs = wait_logs(&f, 7).await;
    assert_eq!((logs[0].provider.as_str(), logs[1].provider.as_str()), ("qoder", "trae"), "{logs:?}");
    // A disabled key is refused before anything else.
    f.hub.keys.update(&ci.id, KeyFields { alias: "ci".into(), enabled: false, ..KeyFields::default() }).unwrap();
    let r = send(ci.key.clone(), "Shared").await;
    assert_eq!(r.status(), 401);
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], "api_key_disabled");
}

#[tokio::test]
async fn merged_model_shares_the_destination_pool_and_keeps_its_name() {
    let f = fixture().await;
    let routes = f.hub.routes(&f.hub.settings());
    let id = |name: &str| crate::catalog::resolve(&routes, name).unwrap().route_id.clone();
    let (only, shared) = (id("QoderOnly"), id("Shared"));
    f.hub.link_route(&only, Some(shared.clone())).unwrap();
    let models: Value = client().get(format!("{}/v1/models", f.url)).bearer_auth(KEY).send().await.unwrap().json().await.unwrap();
    let listed: Vec<&str> = models["data"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
    assert!(listed.contains(&"Shared") && !listed.contains(&"QoderOnly"), "{listed:?}");
    let r = post_json(&f, "/v1/chat/completions", chat("QoderOnly")).await;
    assert_eq!(r.status(), 200);
    assert_eq!(wait_logs(&f, 1).await[0].model, "Shared");
    // An account offering both targets falls back to the merged one.
    f.hub.runtime.lock().unwrap().model_cooldown.insert(("qoder/cn:u1".into(), "q-shared".into()), i64::MAX);
    let r = post_json(&f, "/qoder/cn/v1/chat/completions", chat("Shared")).await;
    assert_eq!(r.status(), 200);
    assert_eq!(wait_logs(&f, 2).await[0].real_model, "q-only");
    // Undoing the merge removes the override entirely.
    f.hub.link_route(&only, None).unwrap();
    assert!(!f.hub.settings().routes.contains_key(&only));
    assert_eq!(crate::catalog::resolve(&f.hub.routes(&f.hub.settings()), "QoderOnly").unwrap().route_id, only);
}

#[tokio::test]
async fn tools_roundtrip_on_every_provider_including_the_trae_text_bridge() {
    let f = fixture().await;
    let body = json!({"model":"Shared","max_tokens":64,"tools":[{"name":"read","input_schema":{"type":"object","properties":{}}}],"messages":[{"role":"user","content":"use a tool"}]});
    let v: Value = post_json(&f, "/qoder/v1/messages", body.clone()).await.json().await.unwrap();
    assert_eq!(v["stop_reason"], "tool_use");
    assert_eq!(v["content"][0]["name"], "read");
    // Trae: the Remote reply's <client_tool_call> becomes a real tool_use block.
    let v: Value = post_json(&f, "/trae/v1/messages", body.clone()).await.json().await.unwrap();
    assert_eq!(v["stop_reason"], "tool_use", "{v}");
    assert_eq!(v["content"][0]["type"], "tool_use");
    assert_eq!(v["content"][0]["name"], "read");
    assert_eq!(f.trae_calls.load(Ordering::SeqCst), 1);
    // The tool result goes back as context and the turn ends with text.
    let id = v["content"][0]["id"].as_str().unwrap().to_owned();
    let mut follow = body.clone();
    follow["messages"] = json!([
        {"role":"user","content":"use a tool"},
        {"role":"assistant","content":[{"type":"tool_use","id":id,"name":"read","input":{}}]},
        {"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":"file body"}]},
    ]);
    let v: Value = post_json(&f, "/trae/v1/messages", follow).await.json().await.unwrap();
    assert_eq!(v["stop_reason"], "end_turn", "{v}");
    assert!(v["content"][0]["text"].as_str().unwrap().contains("你好 from shared-real"));
    // Invalid image bytes must not create a Remote session.
    let image = json!({"model":"Shared","max_tokens":16,"messages":[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}]}]});
    let before = f.trae_calls.load(Ordering::SeqCst);
    let response = post_json(&f, "/trae/v1/messages", image).await;
    assert_eq!(response.status(), 400);
    assert_eq!(f.trae_calls.load(Ordering::SeqCst), before);
}

const IMAGE_DATA: &str = "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAF0lEQVR4nGP4z8BAEiJN9aiGUQ1DSgMAkPn/Afnh+ngAAAAASUVORK5CYII=";

#[tokio::test]
async fn trae_events_stream_closed_early_resumes_from_last_event_id() {
    let f = fixture().await;
    f.trae_plan_mode.store(7, Ordering::SeqCst);
    let r = post_json(&f, "/trae/cn/v1/chat/completions", chat("Shared")).await;
    assert_eq!(r.status(), 200);
    let value: Value = r.json().await.unwrap();
    assert!(value["choices"][0]["message"]["content"].as_str().unwrap().contains("你好 from"), "{value}");
    assert_eq!(f.trae_events.load(Ordering::SeqCst), 2, "one resumed connection");
    let log = &wait_logs(&f, 1).await[0];
    let summary = &log.details["upstreamSummary"];
    assert_eq!(summary["eventReconnects"], 1);
    assert_eq!(summary["events"], json!(["model_config", "plan_item", "reconnect:eof", "plan_item", "token_usage", "done:completed"]));
    assert_eq!(log.usage["total_tokens"], 5);

    // The session reaching Finished ends the turn even without `done`.
    let f = fixture().await;
    f.trae_plan_mode.store(8, Ordering::SeqCst);
    let r = post_json(&f, "/trae/cn/v1/chat/completions", chat("Shared")).await;
    assert_eq!(r.status(), 200);
    assert_eq!(f.trae_events.load(Ordering::SeqCst), 1);
    let log = &wait_logs(&f, 1).await[0];
    assert!(log.details["upstreamSummary"].get("eventReconnects").is_none());
}

#[tokio::test]
async fn trae_turn_whose_events_were_lost_is_read_back_from_the_message_list() {
    let f = fixture().await;
    f.trae_plan_mode.store(9, Ordering::SeqCst);
    let r = post_json(&f, "/trae/cn/v1/chat/completions", chat("Shared")).await;
    assert_eq!(r.status(), 200);
    let value: Value = r.json().await.unwrap();
    assert_eq!(value["choices"][0]["message"]["content"], "recovered from shared-real", "{value}");
    let log = &wait_logs(&f, 1).await[0];
    let summary = &log.details["upstreamSummary"];
    assert_eq!(summary["events"], json!(["status_changed:3", "metadata", "reconnect:eof", "status_changed:5", "recovered:messages"]));
    assert_eq!(summary["finalMessage"], json!({"status":"completed"}));
    assert_eq!(log.usage["total_tokens"], 11);
    assert_eq!(summary["cloudSessionCleanup"], "deleted");
}

#[tokio::test]
async fn conversation_over_the_trae_message_cap_moves_to_other_providers_without_a_session() {
    let f = fixture().await;
    let body = json!({"model":"Shared","messages":[{"role":"user","content":"x".repeat(1_100_000)}]});
    for _ in 0..3 {
        let r = post_json(&f, "/v1/chat/completions", body.clone()).await;
        assert_eq!(r.status(), 200);
    }
    assert_eq!(f.trae_calls.load(Ordering::SeqCst), 0, "no Trae session is created");
    let logs = wait_logs(&f, 4).await;
    let refused: Vec<_> = logs.iter().filter(|l| l.provider == "trae").collect();
    assert!(!refused.is_empty(), "rotation reached Trae at least once");
    for log in &refused {
        assert_eq!(log.outcome, "retried");
        assert_eq!(log.upstream_code.as_deref(), Some("context_length_exceeded"));
        // One refusal excludes every Trae account for that request.
        assert_eq!(refused.iter().filter(|l| l.request_group == log.request_group).count(), 1);
        assert!(logs.iter().any(|l| l.request_group == log.request_group && l.provider == "qoder" && l.outcome == "completed"));
    }

    // Trae alone has no alternative: the client sees the context error.
    let r = post_json(&f, "/trae/cn/v1/chat/completions", body).await;
    assert_eq!(r.status(), 400);
    let error: Value = r.json().await.unwrap();
    assert_eq!(error["error"]["code"], "context_length_exceeded", "{error}");
}

#[tokio::test]
async fn delayed_trae_finish_parameters_work_with_large_namespaced_tool_requests() {
    let f = fixture().await;
    f.trae_plan_mode.store(1, Ordering::SeqCst);
    let tools: Vec<Value> = (0..15).map(|index| {
        let names = if index < 12 { & ["read_file", "run_command", "finish", "list_files"][..] }
            else { & ["read_file", "run_command", "finish"][..] };
        json!({"type":"namespace","name":format!("ns_{index}"),"tools":names.iter().map(|name| {
            json!({"type":"function","name":name,"parameters":{"type":"object","properties":{}}})
        }).collect::<Vec<_>>()})
    }).collect();
    let body = json!({"model":"Shared","input":[{"role":"user","content":"long external conversation ".repeat(4000)}],"tools":tools});
    let response = post_json(&f, "/trae/v1/responses", body).await;
    assert_eq!(response.status(), 200);
    let value: Value = response.json().await.unwrap();
    assert_eq!(value["output"][0]["type"], "function_call", "{value}");
    assert_eq!(value["output"][0]["name"], "read_file");
    assert_eq!(value["output"][0]["namespace"], "ns_0");
    let logs = wait_logs(&f, 1).await;
    assert_eq!(logs[0].details["upstreamSummary"]["clientTools"], 57);
    assert_eq!(logs[0].outcome, "completed");
    assert_eq!(logs[0].details["upstreamSummary"]["cloudSessionCleanup"], "deleted");
    assert_eq!(f.trae_deletes.load(Ordering::SeqCst), 1);
    assert_eq!(f.trae_stops.load(Ordering::SeqCst), 0);

    // Mixed-provider rotation still uses the original account order, including
    // under concurrent requests with different request-local tool catalogs.
    let results = futures_util::future::join_all((0..6).map(|index| {
        let f = &f;
        async move {
            let name = format!("caller_tool_{index}");
            let response = post_json(f, "/v1/responses", json!({"model":"Shared","input":"use the external tool",
                "tools":[{"type":"function","name":name,"parameters":{"type":"object","properties":{}}}]})).await;
            assert_eq!(response.status(), 200);
            let value: Value = response.json().await.unwrap();
            assert_eq!(value["output"][0]["name"], name, "{value}");
        }
    })).await;
    assert_eq!(results.len(), 6);
    assert_eq!(f.trae_calls.load(Ordering::SeqCst), 3);
    assert_eq!(f.qoder_calls.load(Ordering::SeqCst), 4);
    assert_eq!(f.hub.runtime.lock().unwrap().in_flight.values().sum::<usize>(), 0);

    let response = post_json(&f, "/trae/v1/responses", json!({"model":"Shared","input":"hello","stream":true})).await;
    assert_eq!(response.status(), 200);
    let stream = response.text().await.unwrap();
    assert!(stream.contains("response.completed"), "{stream}");
}

#[tokio::test]
async fn actual_remote_tools_still_stop_and_incomplete_finish_never_becomes_success() {
    let f = fixture().await;
    f.trae_plan_mode.store(2, Ordering::SeqCst);
    let response = post_json(&f, "/trae/v1/responses", json!({"model":"Shared","input":"hello"})).await;
    assert_eq!(response.status(), 502);
    let logs = wait_logs(&f, 1).await;
    assert_eq!(logs[0].outcome, "remote_tool_execution");
    assert_eq!(logs[0].details["upstreamSummary"]["cloudSessionCleanup"], "deleted");
    assert_eq!(logs[0].details["upstreamSummary"]["blockedRemoteTool"], "read_file");
    assert_eq!(logs[0].details["upstreamSummary"]["remoteStopConfirmed"], true);
    assert!(!logs[0].details.to_string().contains("PRIVATE_TOOL_ARGUMENT"));
    assert_eq!(f.trae_stops.load(Ordering::SeqCst), 1);
    assert_eq!(f.qoder_calls.load(Ordering::SeqCst), 0);

    f.trae_plan_mode.store(3, Ordering::SeqCst);
    let response = post_json(&f, "/trae/v1/responses", json!({"model":"Shared","input":"hello"})).await;
    assert_eq!(response.status(), 502);
    let error: Value = response.json().await.unwrap();
    assert!(error.to_string().contains("Trae finish 未返回完整 summary"), "{error}");
    let logs = wait_logs(&f, 2).await;
    assert_eq!(logs[0].outcome, "invalid_tool_output");
    assert_eq!(logs[0].details["upstreamSummary"]["cloudSessionCleanup"], "deleted");
}

#[tokio::test]
async fn trae_cleanup_failure_preserves_answer_and_retries_owned_records_only() {
    let f = fixture().await;
    f.trae_cleanup_status.store(500, Ordering::SeqCst);
    let response = post_json(
        &f,
        "/trae/v1/responses",
        json!({"model":"Shared","input":"hello"}),
    )
    .await;
    assert_eq!(response.status(), 200);
    let value: Value = response.json().await.unwrap();
    assert!(value.to_string().contains("你好 from shared-real"));
    let logs = wait_logs(&f, 1).await;
    assert_eq!(logs[0].outcome, "completed");
    assert_eq!(
        logs[0].details["upstreamSummary"]["cloudSessionCleanup"],
        "failed"
    );
    assert_eq!(f.trae_deletes.load(Ordering::SeqCst), 2);
    f.trae_cleanup_status.store(204, Ordering::SeqCst);
    f.hub.trae.retry_remote_cleanup().await;
    assert_eq!(f.trae_deletes.load(Ordering::SeqCst), 3);
    assert!(f.hub.trae.store.cleanup_snapshot().unwrap()["agentdockRemoteCleanup"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn trae_missing_message_id_and_cancelled_create_still_delete_the_created_session() {
    let f = fixture().await;
    f.trae_plan_mode.store(4, Ordering::SeqCst);
    let response = post_json(
        &f,
        "/trae/v1/responses",
        json!({"model":"Shared","input":"hello"}),
    )
    .await;
    assert_eq!(response.status(), 502);
    let logs = wait_logs(&f, 1).await;
    assert_eq!(
        logs[0].details["upstreamSummary"]["cloudSessionCleanup"],
        "deleted"
    );
    assert_eq!(f.trae_deletes.load(Ordering::SeqCst), 1);

    for mode in [5, 6] {
        let before_create = f.trae_calls.load(Ordering::SeqCst);
        let before_events = f.trae_events.load(Ordering::SeqCst);
        let before_delete = f.trae_deletes.load(Ordering::SeqCst);
        f.trae_plan_mode.store(mode, Ordering::SeqCst);
        let hub = f.hub.clone();
        let account = hub.trae.store.accounts()[0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let send = tokio::spawn(async move {
            let input = agentdock_core::provider::SendInput {
                body: json!({"model":"shared-real","messages":[{"role":"user","content":"hello"}]}),
                headers: Default::default(),
                request_id: "cancel-create".into(),
                session_id: None,
                effort_input: Value::Null,
                client_profile: None,
            };
            let mut details = agentdock_core::diagnostics::RequestDetails::default();
            hub.trae
                .send(
                    &account,
                    Some("shared-real".into()),
                    input,
                    &mut details,
                    std::time::Instant::now(),
                )
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while if mode == 5 {
                f.trae_calls.load(Ordering::SeqCst) == before_create
            } else {
                f.trae_events.load(Ordering::SeqCst) == before_events
            } {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        send.abort();
        assert!(send.await.is_err());
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while f.trae_deletes.load(Ordering::SeqCst) == before_delete
                || !f.hub.trae.store.cleanup_snapshot().unwrap()["agentdockRemoteCleanup"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(f.trae_stops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn trae_images_cross_all_protocols_and_upload_failures_never_drop_images() {
    let f = fixture().await;
    let url = format!("data:image/png;base64,{IMAGE_DATA}");
    let requests = [
        ("/trae/v1/messages", json!({"model":"Shared","max_tokens":32,"messages":[
            {"role":"user","content":[{"type":"text","text":"Describe this"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":IMAGE_DATA}}]},
        ]})),
        ("/trae/v1/chat/completions", json!({"model":"Shared","messages":[
            {"role":"user","content":[{"type":"text","text":"Describe this"},{"type":"image_url","image_url":{"url":url}}]},
            {"role":"assistant","content":"previous reply"},
            {"role":"user","content":"continue"},
        ]})),
        ("/trae/v1/responses", json!({"model":"Shared","input":[
            {"role":"user","content":[{"type":"input_text","text":"Describe this"},{"type":"input_image","image_url":url}]},
        ]})),
        ("/trae/v1/messages", json!({"model":"Shared","max_tokens":32,"messages":[
            {"role":"assistant","content":[{"type":"tool_use","id":"image-tool","name":"read","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"image-tool","content":[
                {"type":"text","text":"Describe this"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":IMAGE_DATA}},
            ]}]},
        ]})),
    ];
    for (index, (path, body)) in requests.iter().enumerate() {
        let response = post_json(&f, path, body.clone()).await;
        assert_eq!(response.status(), 200, "{path}: {}", response.text().await.unwrap());
        assert_eq!(f.trae_uploads.load(Ordering::SeqCst), index + 1);
        let queries = f.trae_queries.lock().unwrap();
        let query = queries.last().unwrap();
        assert_eq!(query[2], json!({"type":"image","data":{"image_id":"remote-image-1"}}));
        assert!(query[0]["data"]["content"].as_str().unwrap().contains("[Attached image 1]"));
        assert!(!query.to_string().contains("base64"));
    }
    // Failed uploads fail explicitly; never send an image-free substitute prompt.
    f.trae_upload_fail.store(true, Ordering::SeqCst);
    let before = f.trae_calls.load(Ordering::SeqCst);
    let response = post_json(&f, requests[0].0, requests[0].1.clone()).await;
    assert_eq!(response.status(), 502);
    let error: Value = response.json().await.unwrap();
    assert!(error["error"]["message"].as_str().unwrap().contains("Trae 图片上传失败"), "{error}");
    assert_eq!(f.trae_calls.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn only_the_newest_images_go_upstream_when_limited() {
    let f = fixture().await;
    let image = |_: &str| json!({"type":"image_url","image_url":{"url":format!("data:image/png;base64,{IMAGE_DATA}")}});
    let body = json!({"model":"Shared","messages":[
        {"role":"user","content":[{"type":"text","text":"first"},image("a")]},
        {"role":"assistant","content":"seen a"},
        {"role":"user","content":[{"type":"text","text":"second"},image("b")]},
        {"role":"assistant","content":"seen b"},
        {"role":"user","content":[{"type":"text","text":"now"},image("c"),image("d")]},
    ]});
    f.hub.update_settings(|s| {
        s.recent_images = Some(1);
        Ok(())
    }).unwrap();
    assert_eq!(post_json(&f, "/trae/v1/chat/completions", body.clone()).await.status(), 200);
    // This turn's two images stay even above the limit; both older ones go.
    assert_eq!(f.trae_uploads.load(Ordering::SeqCst), 2);
    let query = f.trae_queries.lock().unwrap().last().unwrap().to_string();
    assert_eq!(query.matches("An earlier image was omitted").count(), 2, "{query}");
    let log = &wait_logs(&f, 1).await[0];
    assert_eq!(log.details["requestSummary"]["omittedImages"], 2);
    assert!(log.details["requestSummary"]["omittedImageBytes"].as_u64().unwrap() > 0);
    // Unlimited (the default) sends every image.
    f.hub.update_settings(|s| {
        s.recent_images = None;
        Ok(())
    }).unwrap();
    assert_eq!(post_json(&f, "/trae/v1/chat/completions", body).await.status(), 200);
    assert_eq!(f.trae_uploads.load(Ordering::SeqCst), 6);
    assert!(f.hub.update_settings(|s| {
        s.recent_images = Some(0);
        Ok(())
    }).is_err());
}

#[tokio::test]
async fn rate_limited_accounts_cool_down_and_traffic_moves_to_other_providers() {
    let f = fixture().await;
    // The Qoder 429 happens before any output, so the request fails over to Trae.
    let r = post_json(&f, "/v1/chat/completions", chat("Flaky")).await;
    assert_eq!(r.status(), 200);
    let key = "qoder/cn:u1";
    assert!(f.hub.runtime.lock().unwrap().cooldown.contains_key(key));
    let before = f.qoder_calls.load(Ordering::SeqCst);
    for _ in 0..3 {
        assert_eq!(post_json(&f, "/v1/chat/completions", chat("Flaky")).await.status(), 200);
    }
    assert_eq!(before, f.qoder_calls.load(Ordering::SeqCst), "cooled-down account is skipped");
    // Shared still works on the remaining accounts.
    for _ in 0..2 {
        assert_eq!(post_json(&f, "/v1/chat/completions", chat("Shared")).await.status(), 200);
    }
    let logs = wait_logs(&f, 7).await;
    let retried = logs.iter().find(|l| l.status == 429 && l.outcome == "retried" && l.provider == "qoder").unwrap();
    assert!(logs.iter().any(|l| l.request_group == retried.request_group && l.attempt == 2 && l.provider == "trae" && l.outcome == "completed"));
    assert_eq!(f.hub.runtime.lock().unwrap().in_flight.values().sum::<usize>(), 0);
    // Disabling a provider removes it from the pool.
    f.hub.update_settings(|s| { s.providers.get_mut(&agentdock_core::provider::ProviderKind::Trae).unwrap().enabled = false; Ok(()) }).unwrap();
    let r = post_json(&f, "/v1/chat/completions", chat("Flaky")).await;
    assert_eq!(r.status(), 503, "only the cooled-down Qoder account remains");
    assert!(r.json::<Value>().await.unwrap()["error"]["message"].as_str().unwrap().contains("账号冷却中"));
}

#[tokio::test]
async fn in_stream_qoder_rejections_fail_over_before_output_and_cool_down_only_the_model() {
    let f = fixture().await;
    for stream in [false, true] {
        let mut body = chat("Gate");
        body["stream"] = json!(stream);
        let r = post_json(&f, "/qoder/v1/chat/completions", body).await;
        assert_eq!(r.status(), 200);
        let text = r.text().await.unwrap();
        assert!(text.contains("OK from q-gate-i") && !text.contains("not allowed"), "{text}");
    }
    let logs = wait_logs(&f, 3).await;
    let retried: Vec<_> = logs.iter().filter(|l| l.outcome == "retried").collect();
    assert_eq!(retried.len(), 1, "the second request skips the cooled model: {logs:?}");
    assert_eq!((retried[0].status, retried[0].real_model.as_str(), retried[0].upstream_code.as_deref()), (403, "q-gate", Some("403")));
    {
        let rt = f.hub.runtime.lock().unwrap();
        assert!(rt.model_cooldown.contains_key(&("qoder/cn:u1".to_owned(), "q-gate".to_owned())));
        assert!(!rt.cooldown.contains_key("qoder/cn:u1"), "the account keeps serving its other models");
    }
    assert_eq!(post_json(&f, "/qoder/cn/v1/chat/completions", chat("Shared")).await.status(), 200);
    // With no alternative account left, the model is rejected with its real status.
    let r = post_json(&f, "/qoder/cn/v1/messages", json!({"model":"Gate","max_tokens":8,"messages":[{"role":"user","content":"hi"}]})).await;
    assert_eq!(r.status(), 503, "cooled model on the only account is not retried");
    assert!(r.text().await.unwrap().contains("该模型冷却中"));
    f.hub.runtime.lock().unwrap().model_cooldown.clear();
    let r = post_json(&f, "/qoder/cn/v1/messages", json!({"model":"Gate","max_tokens":8,"messages":[{"role":"user","content":"hi"}]})).await;
    assert_eq!(r.status(), 403);
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["type"], "permission_error");
}

#[tokio::test]
async fn claude_code_and_codex_effort_sliders_map_to_levels_the_model_offers() {
    let f = fixture().await;
    let messages = |effort: &str, thinking: &str| json!({"model":"Think","max_tokens":16,"output_config":{"effort":effort},"thinking":{"type":thinking},"messages":[{"role":"user","content":"hi"}]});
    let cases = [
        ("/v1/messages", messages("low", "enabled"), "low"),
        ("/v1/messages", messages("high", "enabled"), "xhigh"),
        ("/v1/messages", messages("max", "enabled"), "xhigh"),
        ("/v1/messages", messages("high", "disabled"), "none"),
        ("/v1/responses", json!({"model":"Think","input":"hi","reasoning":{"effort":"minimal"}}), "low"),
        ("/v1/responses", json!({"model":"Think","input":"hi","reasoning":{"effort":"high"}}), "xhigh"),
        ("/v1/chat/completions", json!({"model":"Think","reasoning_effort":"high","messages":[{"role":"user","content":"hi"}]}), "xhigh"),
    ];
    for (path, body, used) in &cases {
        let r = post_json(&f, path, body.clone()).await;
        assert_eq!(r.status(), 200, "{path} {body}");
        let text = r.text().await.unwrap();
        assert!(text.contains(&format!("OK from q-think effort={used}")), "{path} {body}: {text}");
    }
    let logs = wait_logs(&f, cases.len()).await;
    assert!(logs.iter().all(|l| l.outcome == "completed"));
    assert!(logs.iter().any(|l| l.details["requestSummary"]["appliedEffort"] == "xhigh"));
    // The model list carries Codex catalog levels so the client shows a slider.
    let list: Value = client().get(format!("{}/v1/models", f.url)).bearer_auth(KEY).send().await.unwrap().json().await.unwrap();
    let think = list["data"].as_array().unwrap().iter().find(|m| m["id"] == "Think").unwrap();
    let levels: Vec<&Value> = think["supported_reasoning_levels"].as_array().unwrap().iter().map(|l| &l["effort"]).collect();
    assert_eq!(levels, [&json!("none"), &json!("low"), &json!("medium"), &json!("xhigh")]);
    assert_eq!(think["default_reasoning_level"], "medium");
}

#[tokio::test]
async fn codex_default_hosted_web_search_does_not_block_the_turn() {
    let f = fixture().await;
    let body = json!({"model":"Think","stream":true,"input":"hi","reasoning":{"effort":"high","summary":"auto"},
        "include":["reasoning.encrypted_content"],"store":false,"parallel_tool_calls":true,"tool_choice":"auto",
        "tools":[{"type":"function","name":"exec_command","parameters":{"type":"object"}},
            {"type":"namespace","name":"mcp__node_repl","tools":[{"type":"function","name":"js","parameters":{"type":"object"}}]},
            {"type":"web_search","external_web_access":true}]});
    let r = post_json(&f, "/v1/responses", body).await;
    assert_eq!(r.status(), 200);
    let text = r.text().await.unwrap();
    assert!(text.contains("response.completed"), "{text}");
    let log = &wait_logs(&f, 1).await[0];
    assert_eq!(log.outcome, "completed");
    assert_eq!(log.details["requestSummary"]["skippedHostedTools"], json!(["web_search"]));
}

#[tokio::test]
async fn queue_notice_waits_and_retries_the_same_account_when_no_other_exists() {
    let f = fixture().await;
    let r = post_json(&f, "/v1/chat/completions", chat("Queued")).await;
    assert_eq!(r.status(), 200);
    assert!(r.text().await.unwrap().contains("OK from q-queue"));
    let logs = wait_logs(&f, 2).await;
    let queued = logs.iter().find(|l| l.outcome == "retried").unwrap();
    assert!(queued.message.contains("排队中") && queued.upstream_code.as_deref() == Some("10605"), "{queued:?}");
    assert!(logs.iter().any(|l| l.attempt == 2 && l.account_id == queued.account_id && l.outcome == "completed"));
    let rt = f.hub.runtime.lock().unwrap();
    assert!(rt.model_cooldown.is_empty() && !rt.cooldown.contains_key("qoder/cn:u1"), "queueing is not a rejection");
}

#[tokio::test]
async fn workbuddy_expired_or_relogin_required_is_disabled_only_when_its_route_is_used() {
    for needs_relogin in [false, true] {
        let f = fixture().await;
        f.hub.wb.store.lock().unwrap().accounts.push(serde_json::from_value(json!({
            "id":"wb-fixture", "name":"WB fixture", "source":"test", "variant":"cn", "enabled":true,
            "expiresAt": if needs_relogin { i64::MAX } else { 1 },
        })).unwrap());
        if needs_relogin {
            f.hub.wb.runtime.lock().unwrap().presentations.insert("wb-fixture".into(), json!({"needsRelogin":true}));
        }
        f.hub.wb.update_models(vec![json!({"id":"wb-fixture-model"})], vec!["wb-fixture-model".into()], "cn".into()).unwrap();
        assert_eq!(post_json(&f, "/qoder/cn/v1/chat/completions", chat("Shared")).await.status(), 200);
        assert!(f.hub.wb.pool_accounts()[0].enabled, "unrelated scoped requests do not change other providers");
        assert_eq!(post_json(&f, "/workbuddy/cn/v1/chat/completions", chat("wb-fixture-model")).await.status(), 503);
        assert!(!f.hub.wb.pool_accounts()[0].enabled);
        let saved = workbuddy::pool::AppState::new(f.hub.dir.join("workbuddy/store.json")).unwrap();
        assert!(!saved.pool_accounts()[0].enabled);
    }
}

#[tokio::test]
async fn disabled_account_turns_proxy_off_and_stays_out_until_enabled() {
    use agentdock_core::provider::ProviderKind;
    let f = fixture().await;
    f.hub.wb.update_store(|store| {
        store.accounts.push(workbuddy::pool::Account { id:"off-wb".into(),name:"Off".into(),source:"test".into(),source_id:"off".into(),variant:"cn".into(),enabled:true,checkin_enabled:true,note:String::new(),display_field:String::new(),expires_at:None });
        Ok(())
    }).unwrap();
    assert!(f.hub.set_account_disabled(ProviderKind::Workbuddy, "missing", true).is_err());
    f.hub.set_account_disabled(ProviderKind::Workbuddy, "off-wb", true).unwrap();
    let rt = &f.hub.account_runtime()["workbuddy/off-wb"];
    assert_eq!((rt["disabled"].clone(), rt["enabled"].clone()), (json!(true), json!(false)));
    assert!(!f.hub.wb.store.lock().unwrap().accounts.iter().find(|a| a.id == "off-wb").unwrap().enabled);
    assert!(f.hub.disabled_ids(ProviderKind::Workbuddy).contains("off-wb"));
    assert!(f.hub.disabled_ids(ProviderKind::Qoder).is_empty());
    let err = f.hub.set_account_enabled(ProviderKind::Workbuddy, "off-wb", true).unwrap_err();
    assert!(err.contains("已禁用"), "{err}");
    // Even a native switch turned back on elsewhere is not scheduled.
    f.hub.wb.set_enabled("off-wb", true).unwrap();
    assert!(!f.hub.pool_accounts().iter().find(|a| a.id == "off-wb").unwrap().enabled);
    f.hub.enforce_disabled_accounts();
    assert!(!f.hub.wb.store.lock().unwrap().accounts.iter().find(|a| a.id == "off-wb").unwrap().enabled);
    assert!(crate::app::read_settings(&f.hub.dir.join("settings.json")).disabled_accounts.contains("workbuddy/off-wb"));
    f.hub.set_account_disabled(ProviderKind::Workbuddy, "off-wb", false).unwrap();
    assert_eq!(f.hub.account_runtime()["workbuddy/off-wb"]["disabled"], false);
    assert_eq!(f.hub.account_runtime()["workbuddy/off-wb"]["enabled"], false, "enabling leaves the proxy off");
    f.hub.set_account_enabled(ProviderKind::Workbuddy, "off-wb", true).unwrap();
    assert_eq!(f.hub.account_runtime()["workbuddy/off-wb"]["enabled"], true);
}

#[tokio::test]
async fn stale_credit_auth_failure_disables_proxy_but_network_failure_does_not() {
    use agentdock_core::provider::ProviderKind;
    let f = fixture().await;
    f.hub.wb.update_store(|store| {
        store.accounts.push(workbuddy::pool::Account { id:"fixture-wb".into(),name:"Fixture".into(),source:"test".into(),source_id:"fixture".into(),variant:"cn".into(),enabled:true,checkin_enabled:false,note:String::new(),display_field:String::new(),expires_at:None });
        Ok(())
    }).unwrap();
    f.hub.wb.cache_credit("fixture-wb",json!({"ok":true,"totalRemaining":42,"updatedAt":123})).unwrap();
    f.hub.wb.cache_credit("fixture-wb",json!({"ok":false,"error":"connection timeout","authFailed":false})).unwrap();
    assert_eq!(f.hub.account_runtime()["workbuddy/fixture-wb"]["enabled"],true);
    f.hub.wb.cache_credit("fixture-wb",json!({"ok":false,"error":"Unauthorized","authFailed":true})).unwrap();
    let snapshot = f.hub.wb.snapshot();
    assert_eq!(snapshot["accounts"][0]["credit"]["totalRemaining"],42);
    assert_eq!(snapshot["accounts"][0]["credit"]["stale"],true);
    assert_eq!(snapshot["accounts"][0]["credit"]["authFailed"],true);
    assert_eq!(f.hub.account_runtime()["workbuddy/fixture-wb"]["enabled"],false);
    let saved = crate::app::read_settings(&f.hub.dir.join("settings.json"));
    assert!(saved.auth_disabled_accounts.contains("workbuddy/fixture-wb"));
    let reopened = Hub::assemble(&f.hub.dir,saved,KEY,f.hub.qoder.clone(),f.hub.trae.clone());
    assert_eq!(reopened.account_runtime()["workbuddy/fixture-wb"]["authFailed"],true);
    reopened.set_account_enabled(ProviderKind::Workbuddy,"fixture-wb",true).unwrap();
    assert_eq!(reopened.wb.snapshot()["accounts"][0]["credit"]["authFailed"],false);
    assert_eq!(reopened.account_runtime()["workbuddy/fixture-wb"]["enabled"],true);
    // Old versions persisted only the normalized HTML title, without authFailed.
    reopened.wb.cache_credit("fixture-wb",json!({"ok":false,"error":"服务端返回 HTML 错误页：401 Authorization Required"})).unwrap();
    reopened.wb.update_store(|store| { store.resources.get_mut("fixture-wb").unwrap().as_object_mut().unwrap().remove("authFailed"); Ok(()) }).unwrap();
    let migrated = Hub::assemble(&f.hub.dir,reopened.settings(),KEY,f.hub.qoder.clone(),f.hub.trae.clone());
    assert_eq!(migrated.account_runtime()["workbuddy/fixture-wb"]["authFailed"],true);
    assert_eq!(migrated.account_runtime()["workbuddy/fixture-wb"]["enabled"],false);
}

#[tokio::test]
async fn workbuddy_reauthorization_and_credit_verification_clear_stale_auth_without_enabling_proxy() {
    let f = fixture().await;
    let account = |id: &str| workbuddy::pool::Account {
        id:id.into(), name:id.into(), source:"test".into(), source_id:id.into(), variant:"cn".into(),
        enabled:true, checkin_enabled:true, note:"preserved note".into(), display_field:"note".into(), expires_at:None,
    };
    f.hub.wb.update_store(|store| { store.accounts.extend([account("relogin"),account("other")]); Ok(()) }).unwrap();
    for id in ["relogin","other"] {
        f.hub.wb.cache_credit(id,json!({"ok":true,"totalRemaining":42,"updatedAt":123})).unwrap();
        f.hub.wb.cache_credit(id,json!({"ok":false,"authFailed":true,"error":"401"})).unwrap();
    }
    let future = chrono::Utc::now().timestamp_millis() + 86_400_000;
    // The OAuth adapter has imported the fresh metadata before restoring login.
    f.hub.wb.update_store(|store| { store.accounts[0].expires_at = Some(future); Ok(()) }).unwrap();
    f.hub.wb.runtime.lock().unwrap().presentations.insert("relogin".into(),json!({"needsRelogin":true}));
    f.hub.restore_workbuddy_login("relogin").unwrap();
    let rt = f.hub.account_runtime();
    assert_eq!(rt["workbuddy/relogin"]["authFailed"],false);
    assert_eq!(rt["workbuddy/relogin"]["enabled"],false);
    assert_eq!(rt["workbuddy/relogin"]["expiresAt"],future);
    assert_eq!(rt["workbuddy/other"]["authFailed"],true);
    let snapshot = f.hub.wb.snapshot();
    let a = &snapshot["accounts"][0];
    assert_eq!(a["credit"]["authFailed"],false);
    assert_eq!(a["credit"]["totalRemaining"],42);
    assert!(a["credit"].get("error").is_none());
    assert_eq!(a["presentation"]["needsRelogin"],false);
    assert_eq!(a["checkinEnabled"],true);
    assert_eq!(a["note"],"preserved note");
    assert!(f.hub.pool_accounts().iter().find(|a| a.id == "relogin").unwrap().blocked.is_none());
    drop(f.hub.wb.acquire("credits",Some("relogin")).unwrap());
    // Failed refreshes do not prove login recovery, and do not clear other accounts.
    f.hub.wb.cache_credit("other",json!({"ok":false,"error":"network timeout"})).unwrap();
    assert_eq!(f.hub.account_runtime()["workbuddy/other"]["authFailed"],true);
    // A manually requested authenticated credit query can verify an external re-login.
    drop(f.hub.wb.acquire("credits",Some("other")).unwrap());
    f.hub.wb.cache_credit("other",json!({"ok":true,"totalRemaining":55,"updatedAt":456})).unwrap();
    assert_eq!(f.hub.account_runtime()["workbuddy/other"]["authFailed"],false);
    assert_eq!(f.hub.account_runtime()["workbuddy/other"]["enabled"],false);
    let restarted = Hub::assemble(&f.hub.dir,crate::app::read_settings(&f.hub.dir.join("settings.json")),KEY,f.hub.qoder.clone(),f.hub.trae.clone());
    for id in ["relogin","other"] {
        assert_eq!(restarted.account_runtime()[format!("workbuddy/{id}")]["authFailed"],false);
        assert_eq!(restarted.account_runtime()[format!("workbuddy/{id}")]["enabled"],false);
    }
    assert!(restarted.settings().auth_disabled_accounts.is_empty());
    restarted.set_account_enabled(agentdock_core::provider::ProviderKind::Workbuddy,"relogin",true).unwrap();
    restarted.wb.cache_credit("relogin",json!({"ok":true,"totalRemaining":66})).unwrap();
    assert_eq!(restarted.account_runtime()["workbuddy/relogin"]["enabled"],true);
}

#[tokio::test]
async fn qoder_and_trae_recovery_clear_auth_failure_keep_proxy_off_and_reject_stale_signals() {
    use agentdock_core::provider::{credential_revision, ProviderKind};
    let f = fixture().await;
    let id = f.hub.trae.pool_accounts()[0].id.clone();
    f.hub.disable_account_auth(ProviderKind::Trae, &id);
    let old = f.hub.trae.store.account(&id).unwrap();
    // Reimporting identical credentials cannot prove recovery.
    f.hub.trae.import("cn", old.clone()).unwrap();
    assert_eq!(f.hub.account_runtime()[format!("trae/{id}")]["authFailed"], true);
    let mut fresh = old;
    fresh["user"]["token"] = json!("fresh-trae-fixture");
    fresh["user"]["expiredAt"] = json!("2099-01-01T00:00:00Z");
    f.hub.trae.import("cn", fresh).unwrap();
    assert_eq!(f.hub.account_runtime()[format!("trae/{id}")]["authFailed"], false);
    assert_eq!(f.hub.account_runtime()[format!("trae/{id}")]["enabled"], false);
    f.hub.disable_account_auth_observed(ProviderKind::Trae, &id, Some(credential_revision("never-real")));
    assert_eq!(f.hub.account_runtime()[format!("trae/{id}")]["authFailed"], false);
    f.hub.disable_account_auth(ProviderKind::Trae, &id);
    f.hub.restore_login_observed(ProviderKind::Trae, &id, Some(credential_revision("never-real"))).unwrap();
    assert_eq!(f.hub.account_runtime()[format!("trae/{id}")]["authFailed"], true);
    // A verified same-token response can recover too (the refresh API need not rotate).
    f.hub.trae.store.login_restored(&f.hub.trae.store.account(&id).unwrap());

    f.hub.disable_account_auth(ProviderKind::Qoder, "cn:u1");
    let before = f.hub.credential_revisions(ProviderKind::Qoder);
    f.hub.restore_changed_logins(ProviderKind::Qoder, &before).unwrap();
    assert_eq!(f.hub.account_runtime()["qoder/cn:u1"]["authFailed"], true);
    {
        let mut accounts = f.hub.qoder.fixture_accounts.lock().unwrap();
        let a = &mut accounts.as_mut().unwrap()[0];
        a.token = "fresh-qoder-fixture".into();
        a.expiresAt = "2000-01-01T00:00:00Z".into();
    }
    f.hub.restore_changed_logins(ProviderKind::Qoder, &before).unwrap();
    assert_eq!(f.hub.account_runtime()["qoder/cn:u1"]["authFailed"], true, "an expired replacement is not recovery");
    f.hub.qoder.fixture_accounts.lock().unwrap().as_mut().unwrap()[0].expiresAt = "2099-01-01T00:00:00Z".into();
    f.hub.restore_changed_logins(ProviderKind::Qoder, &before).unwrap();
    assert_eq!(f.hub.account_runtime()["qoder/cn:u1"]["authFailed"], false);
    assert_eq!(f.hub.account_runtime()["qoder/cn:u1"]["enabled"], false);
    f.hub.disable_account_auth_observed(ProviderKind::Qoder, "cn:u1", Some(credential_revision("never-real")));
    assert_eq!(f.hub.account_runtime()["qoder/cn:u1"]["authFailed"], false);

    let restarted = Hub::assemble(&f.hub.dir, crate::app::read_settings(&f.hub.dir.join("settings.json")), KEY, f.hub.qoder.clone(), f.hub.trae.clone());
    for key in ["qoder/cn:u1".to_owned(), format!("trae/{id}")] {
        assert_eq!(restarted.account_runtime()[&key]["authFailed"], false);
        assert_eq!(restarted.account_runtime()[&key]["enabled"], false);
    }
}

#[tokio::test]
async fn trae_events_restore_in_runtime_logs_and_stay_cleared_after_restart() {
    let f = fixture().await;
    f.hub.trae.store.event("refresh", "fixture refresh complete");
    f.hub.trae.store.event("error", "fixture check-in failed");
    let reopened = trae::Provider::open(&f.hub.dir.join("trae")).unwrap();
    let restarted = Hub::assemble(&f.hub.dir, f.hub.settings(), KEY, f.hub.qoder.clone(), reopened);
    let entries = restarted.applog.query(&json!({"source":"trae"}));
    assert_eq!(entries["entries"].as_array().unwrap().len(), 2);
    assert_eq!(entries["entries"][0]["message"], "[error] fixture check-in failed");
    assert_eq!(restarted.applog.query(&json!({"level":"warn"}))["entries"].as_array().unwrap().len(), 1);
    restarted.trae.store.clear_events_then(|| restarted.applog.clear()).unwrap();
    assert_eq!(restarted.applog.query(&json!({}))["total"], 0);
    let reopened = trae::Provider::open(&f.hub.dir.join("trae")).unwrap();
    let cleared = Hub::assemble(&f.hub.dir, f.hub.settings(), KEY, f.hub.qoder.clone(), reopened);
    assert_eq!(cleared.applog.query(&json!({}))["total"], 0);
}

#[tokio::test]
async fn mixed_batch_export_roundtrips_without_mutating_fixture_credentials() {
    use crate::account_json::{self, AccountRef};
    let f = fixture().await;
    let trae_id = f.hub.trae.pool_accounts()[0].id.clone();
    let doc = account_json::export_batch(&f.hub,&[
        AccountRef{provider:"qoder".into(),id:"cn:u1".into()},
        AccountRef{provider:"trae".into(),id:trae_id.clone()},
        AccountRef{provider:"trae".into(),id:trae_id.clone()},
    ]).unwrap();
    assert_eq!(doc["accounts"].as_array().unwrap().len(),2);
    assert_eq!(doc["accounts"][0]["account"]["token"],"never-real");
    assert_eq!(doc["accounts"][1]["account"]["user"]["token"],"never-real");
    let single_provider_batch = json!({"format":account_json::BATCH_FORMAT,"accounts":[doc["accounts"][1]]});
    let result = account_json::import(&f.hub,"auto",None,&single_provider_batch).unwrap();
    assert_eq!(result["count"],1);
    assert_eq!(f.hub.trae.pool_accounts()[0].id,trae_id);
    let before = f.hub.trae.store.accounts();
    let invalid = json!([doc["accounts"][1],{"format":account_json::FORMAT,"provider":"qoder","account":{"uid":"missing-edition"}}]);
    assert!(account_json::import(&f.hub,"auto",None,&invalid).is_err());
    assert_eq!(f.hub.trae.store.accounts(),before);
}

#[tokio::test]
async fn automatic_disable_persistence_error_preserves_original_failure_and_runtime_exclusion() {
    let f = fixture().await;
    std::fs::create_dir(f.hub.dir.join("settings.json")).unwrap();
    let response = post_json(&f, "/qoder/cn/v1/chat/completions", chat("AuthHttp")).await;
    assert_eq!(response.status(), 401);
    assert_eq!(f.hub.account_runtime()["qoder/cn:u1"]["enabled"], false);
    assert_eq!(post_json(&f, "/qoder/cn/v1/chat/completions", chat("Shared")).await.status(), 503);
    assert_eq!(f.qoder_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn expired_accounts_are_disabled_before_upstream_and_stay_off_after_restart() {
    use agentdock_core::provider::ProviderKind;
    let f = fixture().await;
    let id = f.hub.trae.pool_accounts()[0].id.clone();
    f.hub.trae.store.patch_account(&id, |a| a["user"]["expiredAt"] = json!("2000-01-01T00:00:00Z")).unwrap();
    let response = post_json(&f, "/trae/cn/v1/chat/completions", chat("Shared")).await;
    assert_eq!(response.status(), 503);
    assert_eq!(f.trae_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.hub.account_runtime()[format!("trae/{id}")]["enabled"], false);
    assert_eq!(f.hub.account_runtime()[format!("trae/{id}")]["authFailed"], true);
    let saved = crate::app::read_settings(&f.hub.dir.join("settings.json"));
    assert!(saved.auth_disabled_accounts.contains(&format!("trae/{id}")));
    let reopened = trae::Provider::open(&f.hub.dir.join("trae")).unwrap();
    let restarted = Hub::assemble(&f.hub.dir, saved, KEY, f.hub.qoder.clone(), reopened);
    assert_eq!(restarted.account_runtime()[format!("trae/{id}")]["enabled"], false);
    assert_eq!(restarted.account_runtime()[format!("trae/{id}")]["authFailed"], true);
    // Updating login alone does not unexpectedly re-enable proxying.
    restarted.trae.store.patch_account(&id, |a| a["user"]["expiredAt"] = json!("2099-01-01T00:00:00Z")).unwrap();
    assert!(!restarted.pool_accounts().iter().find(|a| a.id == id).unwrap().enabled);
    restarted.set_account_enabled(ProviderKind::Trae, &id, true).unwrap();
    assert_eq!(restarted.account_runtime()[format!("trae/{id}")]["authFailed"], false);
    assert_eq!(restarted.account_runtime()[format!("trae/{id}")]["enabled"], true);
    assert!(crate::app::read_settings(&f.hub.dir.join("settings.json")).auth_disabled_accounts.is_empty());
}

#[tokio::test]
async fn http_and_in_stream_auth_failures_disable_only_the_failing_account() {
    for model in ["AuthHttp", "AuthStream", "AuthLate"] {
        for stream in [false, true] {
            let f = fixture().await;
            let mut body = chat(model);
            body["stream"] = json!(stream);
            let response = post_json(&f, "/qoder/cn/v1/chat/completions", body).await;
            let _ = response.text().await.unwrap();
            wait_logs(&f, 1).await;
            let rt = f.hub.account_runtime();
            assert_eq!(rt["qoder/cn:u1"]["enabled"], false, "{model}/{stream}");
            assert_eq!(rt["qoder/cn:u1"]["authFailed"], true);
            assert_eq!(rt["qoder/intl:u2"]["enabled"], true);
            assert!(f.hub.qoder.config().disabled_accounts.contains(&"cn:u1".to_owned()));
            assert!(crate::app::read_settings(&f.hub.dir.join("settings.json")).auth_disabled_accounts.contains("qoder/cn:u1"));
            let calls = f.qoder_calls.load(Ordering::SeqCst);
            assert_eq!(post_json(&f, "/qoder/cn/v1/chat/completions", chat("Shared")).await.status(), 503);
            assert_eq!(f.qoder_calls.load(Ordering::SeqCst), calls);
        }
    }
}

#[tokio::test]
async fn trae_auth_failure_disables_proxy_but_permission_quota_and_server_errors_do_not() {
    for status in [401, 403, 429, 500] {
        let f = fixture().await;
        f.trae_plan_mode.store(status, Ordering::SeqCst);
        assert_eq!(post_json(&f, "/trae/cn/v1/chat/completions", chat("Shared")).await.status(), 502);
        let id = f.hub.trae.pool_accounts()[0].id.clone();
        let rt = f.hub.account_runtime();
        assert_eq!(rt[format!("trae/{id}")]["enabled"], status != 401);
        assert_eq!(rt[format!("trae/{id}")]["authFailed"], status == 401);
        assert_eq!(f.trae_calls.load(Ordering::SeqCst), 0, "catalog rejection must not create a session");
    }
}

#[tokio::test]
async fn html_auth_failure_during_trae_session_creation_disables_without_creating_a_session() {
    for mode in [402, 404] {
    let f = fixture().await;
    f.trae_plan_mode.store(mode, Ordering::SeqCst);
    assert_eq!(post_json(&f, "/trae/cn/v1/chat/completions", chat("Shared")).await.status(), 502);
    let id = f.hub.trae.pool_accounts()[0].id.clone();
    assert_eq!(f.hub.account_runtime()[format!("trae/{id}")]["enabled"], false);
    let logs = wait_logs(&f, 1).await;
    assert_eq!(logs[0].details["upstreamStatus"], if mode == 402 { 401 } else { 200 });
    assert_eq!(f.trae_calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.trae_events.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn concurrent_account_suspensions_keep_all_native_and_durable_switches_in_step() {
    use agentdock_core::provider::ProviderKind;
    let f = fixture().await;
    let barrier = Arc::new(std::sync::Barrier::new(2));
    std::thread::scope(|scope| {
        for id in ["cn:u1", "intl:u2"] {
            let (hub, barrier) = (f.hub.clone(), barrier.clone());
            scope.spawn(move || {
                barrier.wait();
                hub.disable_account_auth(ProviderKind::Qoder, id);
            });
        }
    });
    let saved = crate::app::read_settings(&f.hub.dir.join("settings.json"));
    for id in ["cn:u1", "intl:u2"] {
        assert!(saved.auth_disabled_accounts.contains(&format!("qoder/{id}")));
        assert!(f.hub.qoder.config().disabled_accounts.contains(&id.to_owned()));
        assert_eq!(f.hub.account_runtime()[format!("qoder/{id}")]["enabled"], false);
    }
    f.hub.set_account_enabled(ProviderKind::Qoder, "cn:u1", true).unwrap();
    assert_eq!(f.hub.account_runtime()["qoder/cn:u1"]["enabled"], true);
    assert_eq!(f.hub.account_runtime()["qoder/intl:u2"]["enabled"], false);
    assert_eq!(f.hub.qoder.config().disabled_accounts, ["intl:u2"]);
}

#[tokio::test]
async fn authentication_origin_estimates_limits_and_unknown_models() {
    let f = fixture().await;
    let url = format!("{}/v1/models", f.url);
    assert_eq!(client().get(&url).send().await.unwrap().status(), 401);
    assert_eq!(client().get(&url).bearer_auth("wrong").send().await.unwrap().status(), 401);
    assert_eq!(client().get(&url).header("x-api-key", KEY).send().await.unwrap().status(), 200);
    assert_eq!(client().get(&url).bearer_auth(KEY).header("origin", "https://evil.example").send().await.unwrap().status(), 403);
    assert_eq!(client().get(&url).bearer_auth(KEY).header("origin", "null").send().await.unwrap().status(), 200);
    assert_eq!(client().get(format!("{}/health", f.url)).send().await.unwrap().status(), 200);
    let r = post_json(&f, "/v1/messages/count_tokens", json!({"model":"Shared","messages":[{"role":"user","content":"hello"}]})).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["x-token-count-method"], "estimate");
    let r = post_json(&f, "/v1/messages", json!({"model":"nope","max_tokens":8,"messages":[{"role":"user","content":"x"}]})).await;
    assert_eq!(r.status(), 400);
    assert_eq!(r.json::<Value>().await.unwrap()["type"], "error");
    let big = reqwest::Body::from(vec![b' '; agentdock_core::MAX_REQUEST_BODY_BYTES + 1]);
    let r = client().post(format!("{}/v1/responses", f.url)).bearer_auth(KEY).header("content-type", "application/json").body(big).send().await.unwrap();
    assert_eq!(r.status(), 413);
    // Unauthenticated requests are rejected without reading a never-ending body.
    let pending = reqwest::Body::wrap_stream(futures_util::stream::pending::<Result<Vec<u8>, std::io::Error>>());
    let r = client().post(format!("{}/v1/responses", f.url)).body(pending).timeout(std::time::Duration::from_secs(2)).send().await.unwrap();
    assert_eq!(r.status(), 401);
    let detail: Value = client().get(format!("{}/v1/models/Shared", f.url)).bearer_auth(KEY).send().await.unwrap().json().await.unwrap();
    assert_eq!(detail["type"], "model");
    assert_eq!(detail["upstreams"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn credit_first_prefers_richer_accounts_across_providers() {
    let f = fixture().await;
    f.hub.qoder.cache_credits("intl:u2", json!({"ok":true,"usage":{"userQuota":{"remaining":900,"total":1000},"addOnQuota":{"remaining":0}},"updatedAt":1}));
    f.hub.qoder.cache_credits("cn:u1", json!({"ok":true,"usage":{"userQuota":{"remaining":10,"total":1000},"addOnQuota":{"remaining":0}},"updatedAt":1}));
    f.hub.update_settings(|s| { s.strategy = crate::settings::Strategy::CreditFirst; Ok(()) }).unwrap();
    for i in 0..3 {
        assert_eq!(post_json(&f, "/v1/chat/completions", chat("Shared")).await.status(), 200);
        assert_eq!(wait_logs(&f, i + 1).await[0].account_id, "intl:u2");
    }
}

#[tokio::test]
async fn account_batch_delete_reports_partial_failures_and_preserves_other_accounts() {
    use crate::account_json::AccountRef;
    let f = fixture().await;
    let trae_id = f.hub.trae.pool_accounts()[0].id.clone();
    let account = || AccountRef { provider:"trae".into(), id:trae_id.clone() };
    assert!(f.hub.delete_accounts(&[account()], false).is_err());
    assert!(f.hub.delete_accounts(&[], true).is_err());
    assert!(f.hub.delete_accounts(&[account(), AccountRef { provider:"unknown".into(), id:"bad".into() }], true).is_err());
    assert!(f.hub.trae.store.account(&trae_id).is_ok());
    let result = f.hub.delete_accounts(&[account(), account(), AccountRef { provider:"workbuddy".into(),id:"missing".into() }],true).unwrap();
    assert_eq!(result["count"],1);
    assert_eq!(result["failed"],1);
    assert_eq!(result["results"].as_array().unwrap().len(),2);
    assert!(f.hub.trae.store.account(&trae_id).is_err());
    assert!(!f.hub.pool_accounts().iter().any(|a| a.provider == agentdock_core::provider::ProviderKind::Trae && a.id == trae_id));
    assert!(!f.hub.qoder.accounts().is_empty());
}

#[tokio::test]
async fn late_auth_callbacks_do_not_recreate_deleted_account_suspensions() {
    use agentdock_core::provider::ProviderKind;
    let f = fixture().await;
    f.hub.wb.update_store(|store| {
        store.accounts.push(serde_json::from_value(json!({"id":"deleted-fixture","name":"fixture","source":"test","sourceId":"fixture-only","variant":"cn","enabled":true})).unwrap());
        Ok(())
    }).unwrap();
    f.hub.delete_accounts(&[crate::account_json::AccountRef {provider:"workbuddy".into(),id:"deleted-fixture".into()}], true).unwrap();
    f.hub.disable_account_auth(ProviderKind::Workbuddy, "deleted-fixture");
    f.hub.restore_login_observed(ProviderKind::Workbuddy, "deleted-fixture", None).unwrap();
    assert!(!f.hub.settings().auth_disabled_accounts.contains("workbuddy/deleted-fixture"));
    assert!(!f.hub.runtime.lock().unwrap().auth_failed.contains("workbuddy/deleted-fixture"));
}
