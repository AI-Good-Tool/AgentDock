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
                    chunks.push(json!({"choices":[{"index":0,"delta":{"content":format!("OK from {}", body["model"].as_str().unwrap())}}]}));
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
    // Trae Remote Agent chain: models → chat_sessions → events (one turn per request).
    let sessions: Arc<std::sync::Mutex<HashMap<String, (String, String)>>> = Arc::default();
    let created = sessions.clone();
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
            axum::routing::get(|| async {
                Json(json!({"code":0,"data":{"list":[
                    {"function":"solo_work_remote","models":[{"name":"work-only"}]},
                    {"function":"solo_agent_remote","models":[
                        {"name":"shared-real","display_name":"Shared","is_default":true,"config_source":1},
                        {"name":"flaky-real","display_name":"Flaky"}]}]}}))
            }),
        )
        .route(
            "/api/remote/v1/chat_sessions",
            post(move |Json(body): Json<Value>| {
                let (tc, created, queries) = (tc.clone(), created.clone(), queries.clone());
                async move {
                    tc.fetch_add(1, Ordering::SeqCst);
                    let m = &body["initial_message"];
                    assert_eq!(m["custom_model"]["name"], m["model_name"], "the full catalog entry is sent");
                    let query: Value = serde_json::from_str(m["query"].as_str().unwrap()).unwrap();
                    queries.lock().unwrap().push(query.clone());
                    let sid = uuid::Uuid::new_v4().simple().to_string();
                    created.lock().unwrap().insert(
                        sid.clone(),
                        (m["model_name"].as_str().unwrap().to_owned(), query[0]["data"]["content"].as_str().unwrap().to_owned()),
                    );
                    Json(json!({"code":0,"data":{"chat_session_id":sid,"message_id":"m1"}}))
                }
            }),
        )
        .route(
            "/api/remote/v1/chat_sessions/{sid}/events",
            axum::routing::get(move |axum::extract::Path(sid): axum::extract::Path<String>| {
                let sessions = sessions.clone();
                let plan_mode = plan_mode.clone();
                async move {
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
                    events.extend([
                        ("token_usage", json!({"prompt_tokens":3,"completion_tokens":2})),
                        ("done", json!({"status":"completed"})),
                    ]);
                    let wire = events
                    .iter()
                    .map(|(e, d)| format!("event: {e}\ndata: {d}\n\n"))
                    .collect::<String>();
                    ([("content-type", "text/event-stream")], wire)
                }
            }),
        )
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
            ("cn".into(), vec![json!({"id":"q-shared","name":"Shared"}), json!({"id":"q-only","name":"QoderOnly"}), json!({"id":"q-flaky","name":"Flaky"}), json!({"id":"q-gate","name":"Gate"}), json!({"id":"q-queue","name":"Queued"})]),
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
    Fixture { hub, url, qoder_calls, trae_calls, trae_queries, trae_uploads, trae_upload_fail, trae_plan_mode, trae_stops, tasks: vec![q_task, t_task, g_task] }
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
