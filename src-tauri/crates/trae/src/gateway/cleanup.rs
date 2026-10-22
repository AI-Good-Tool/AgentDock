//! Own only sessions returned by this gateway's create request. Never list or
//! delete historical sessions. Ownership survives restart in an encrypted queue.
use super::{http, remote_base, remote_headers};
use crate::store::Store;
use agentdock_core::{applog, diagnostics::RequestDetails};
use fs2::FileExt;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

const QUEUE: &str = "agentdockRemoteCleanup";

pub struct Cleanup {
    store: Arc<Store>,
    retrying: tokio::sync::Mutex<()>,
}

pub(super) struct Session {
    owner: Arc<Cleanup>,
    key: String,
    lease: Option<std::fs::File>,
    url: String,
    account: Value,
    sid: String,
    pub(super) message: Option<String>,
    pub(super) finished: bool,
    done: bool,
    background: bool,
    pub(super) persistence_error: bool,
}

fn endpoint(base: &str, sid: &str) -> Result<String, String> {
    if sid.is_empty() || sid.len() > 1024 || matches!(sid, "." | "..") {
        return Err("invalid_session_id".into());
    }
    let mut url = reqwest::Url::parse(base).map_err(|_| "invalid_cleanup_endpoint")?;
    url.path_segments_mut()
        .map_err(|_| "invalid_cleanup_endpoint")?
        .push("chat_sessions")
        .push(sid);
    Ok(url.to_string())
}

impl Cleanup {
    pub fn new(store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self {
            store,
            retrying: tokio::sync::Mutex::new(()),
        })
    }

    /// Must be called inside the detached create task, before returning its ID.
    pub(super) fn own(
        self: &Arc<Self>,
        account: Value,
        base: &str,
        sid: &str,
        message: Option<String>,
    ) -> Result<Session, String> {
        let url = endpoint(base, sid)?;
        let key = uuid::Uuid::new_v4().to_string();
        let lease = self
            .store
            .cleanup_lock(&key)
            .ok()
            .filter(|lock| lock.try_lock_exclusive().is_ok());
        let persisted = lease.is_some()
            && self
                .store
                .update_cleanup(|data| {
                    if !data[QUEUE].is_array() {
                        data[QUEUE] = json!([]);
                    }
                    data[QUEUE].as_array_mut().unwrap().push(json!({
                        "key":key, "accountId":account["id"], "sessionId":sid,
                        "messageId":message, "finished":false, "status":"pending",
                        "createdAt":chrono::Utc::now().to_rfc3339(),
                    }));
                    Ok(())
                })
                .is_ok();
        Ok(Session {
            owner: self.clone(),
            key,
            lease,
            url,
            account,
            sid: sid.into(),
            message,
            finished: false,
            done: false,
            background: false,
            persistence_error: !persisted,
        })
    }

    /// Reconcile only explicitly owned records, skipping in-flight sessions.
    pub async fn retry(&self, host_override: Option<&str>) {
        let Ok(_retry) = self.retrying.try_lock() else {
            return;
        };
        let Ok(snapshot) = self.store.cleanup_snapshot() else {
            return;
        };
        let rows = snapshot[QUEUE].as_array().cloned().unwrap_or_default();
        for row in rows {
            let (Some(key), Some(id), Some(sid)) = (
                row["key"].as_str(),
                row["accountId"].as_str(),
                row["sessionId"].as_str(),
            ) else {
                continue;
            };
            if !["pending", "failed"]
                .iter()
                .any(|status| row["status"] == *status)
            {
                continue;
            }
            let Ok(lease) = self.store.cleanup_lock(key) else {
                continue;
            };
            if lease.try_lock_exclusive().is_err() {
                continue;
            }
            // A stale snapshot can race successful deletion and lock-file
            // removal. Recheck ownership after acquiring the session lease.
            let Ok(latest) = self.store.cleanup_snapshot() else {
                continue;
            };
            if !latest[QUEUE]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|current| current["key"] == key))
            {
                drop(lease);
                self.remove_lock(key);
                continue;
            }
            let result = match self.store.account(id) {
                Ok(account) => match remote_base(&account, host_override)
                    .and_then(|base| endpoint(&base, sid))
                {
                    Ok(url) => {
                        if row["finished"] != true {
                            stop(&account, &url, sid, row["messageId"].as_str()).await;
                        }
                        delete(&account, &url).await
                    }
                    Err(_) => Err("cleanup_endpoint_unavailable".into()),
                },
                Err(_) => Err("cleanup_account_unavailable".into()),
            };
            let saved = self.record(key, &result, row["finished"] == true);
            drop(lease);
            if result.is_ok() && saved {
                self.remove_lock(key);
            }
        }
    }

    fn remove_lock(&self, key: &str) {
        if uuid::Uuid::parse_str(key).is_ok() {
            let _ = std::fs::remove_file(
                self.store
                    .dir
                    .join("remote-cleanup-locks")
                    .join(format!("{key}.lock")),
            );
        }
    }

    fn record(&self, key: &str, result: &Result<(), String>, finished: bool) -> bool {
        let persisted = self
            .store
            .update_cleanup(|data| {
                if let Some(rows) = data[QUEUE].as_array_mut() {
                    if result.is_ok() {
                        rows.retain(|row| row["key"] != key);
                    } else if let Some(row) = rows.iter_mut().find(|row| row["key"] == key) {
                        row["status"] = json!("failed");
                        row["finished"] = json!(finished);
                        row["error"] = json!(result.as_ref().err());
                        row["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
                    }
                }
                Ok(())
            })
            .is_ok();
        if result.is_err() || !persisted {
            applog::warn("trae.cleanup", || {
                (
                    "Trae 云端会话清理或状态保存未完成".into(),
                    json!({"cloudCleanupError":result.as_ref().err(),"cleanupStateSaved":persisted}),
                )
            });
        }
        persisted
    }
}

impl Session {
    pub(super) async fn stop(&mut self) -> bool {
        let confirmed = stop(&self.account, &self.url, &self.sid, self.message.as_deref()).await;
        self.finished = confirmed;
        confirmed
    }

    async fn run(&mut self) -> Result<(), String> {
        if !self.finished {
            self.stop().await;
        }
        let result = delete(&self.account, &self.url).await;
        let saved = self.owner.record(&self.key, &result, self.finished);
        self.done = true;
        self.lease.take();
        if result.is_ok() && saved {
            self.owner.remove_lock(&self.key);
        }
        result
    }

    pub(super) async fn cleanup(&mut self, details: &mut RequestDetails) {
        let result = self.run().await;
        details.upstream_summary["cloudSessionCleanup"] =
            json!(if result.is_ok() { "deleted" } else { "failed" });
        details.upstream_summary["cloudCleanupError"] = json!(result.err());
        details.upstream_summary["cloudCleanupAt"] = json!(chrono::Utc::now().to_rfc3339());
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        if self.background {
            return;
        }
        // Independent task: cancellation of the original HTTP future must not
        // cancel cleanup. A process exit leaves the durable pending record.
        let mut task = Self {
            owner: self.owner.clone(),
            key: self.key.clone(),
            lease: self.lease.take(),
            url: self.url.clone(),
            account: self.account.clone(),
            sid: self.sid.clone(),
            message: self.message.clone(),
            finished: self.finished,
            done: false,
            background: true,
            persistence_error: self.persistence_error,
        };
        self.done = true;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                task.run().await.ok();
            });
        } else {
            task.done = true;
        }
    }
}

async fn stop(account: &Value, url: &str, sid: &str, mid: Option<&str>) -> bool {
    let Some(mid) = mid else { return false };
    let mut req = http()
        .post(format!("{url}/stop"))
        .timeout(Duration::from_secs(5))
        .json(&json!({"chat_session_id":sid,"user_message_id":mid}));
    for (key, value) in remote_headers(account) {
        req = req.header(key, value);
    }
    match req.send().await {
        Ok(r) if r.status().is_success() => r
            .json::<Value>()
            .await
            .is_ok_and(|data| data["code"].is_null() || data["code"] == 0 || data["code"] == "0"),
        _ => false,
    }
}

async fn delete(account: &Value, url: &str) -> Result<(), String> {
    let mut error = "cleanup_network_error".to_owned();
    for _ in 0..2 {
        let mut req = http().delete(url).timeout(Duration::from_secs(5));
        for (key, value) in remote_headers(account) {
            req = req.header(key, value);
        }
        match req.send().await {
            Ok(r) if matches!(r.status().as_u16(), 204 | 404) => return Ok(()),
            Ok(r) if !r.status().is_success() => {
                let status = r.status().as_u16();
                error = format!("cleanup_http_{status}");
                if matches!(status, 401 | 403) {
                    break;
                }
            }
            Ok(r) => match r.json::<Value>().await {
                Ok(data) if data["code"] == 0 => return Ok(()),
                Ok(_) => error = "cleanup_business_error".into(),
                Err(_) => error = "cleanup_network_error".into(),
            },
            Err(_) => error = "cleanup_network_error".into(),
        }
    }
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::Path,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{delete as route_delete, post},
        Json, Router,
    };
    use std::sync::{
        atomic::{AtomicU16, AtomicUsize, Ordering},
        Mutex,
    };

    struct Fixture {
        dir: std::path::PathBuf,
        store: Arc<Store>,
        owner: Arc<Cleanup>,
        account: Value,
        host: String,
        base: String,
        deletes: Arc<Mutex<Vec<String>>>,
        stops: Arc<AtomicUsize>,
        status: Arc<AtomicU16>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.task.abort();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
    impl Fixture {
        async fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("agentdock-cleanup-{}", uuid::Uuid::new_v4()));
            let store = Store::open(&dir).unwrap();
            let account = store.upsert(json!({"variant":"cn","user":{"userId":"test","token":"PRIVATE_TOKEN","expiredAt":"2099-01-01T00:00:00Z","account":{}}})).unwrap();
            let owner = Cleanup::new(store.clone());
            let deletes = Arc::new(Mutex::new(Vec::new()));
            let status = Arc::new(AtomicU16::new(204));
            let stops = Arc::new(AtomicUsize::new(0));
            let (d, s, t) = (deletes.clone(), status.clone(), stops.clone());
            let app = Router::new()
                .route(
                    "/api/remote/v1/chat_sessions/{sid}",
                    route_delete(move |Path(sid): Path<String>, headers: HeaderMap| {
                        let (d, s) = (d.clone(), s.clone());
                        async move {
                            assert_eq!(headers["authorization"], "Cloud-IDE-JWT PRIVATE_TOKEN");
                            d.lock().unwrap().push(sid.clone());
                            if sid == "slow-retry" {
                                tokio::time::sleep(Duration::from_millis(200)).await;
                            }
                            if s.load(Ordering::SeqCst) == 200 {
                                Json(json!({"code":123,"message":"PRIVATE_SERVER_TEXT"}))
                                    .into_response()
                            } else {
                                StatusCode::from_u16(s.load(Ordering::SeqCst))
                                    .unwrap()
                                    .into_response()
                            }
                        }
                    }),
                )
                .route(
                    "/api/remote/v1/chat_sessions/{sid}/stop",
                    post(move || {
                        let t = t.clone();
                        async move {
                            t.fetch_add(1, Ordering::SeqCst);
                            Json(json!({"code":0}))
                        }
                    }),
                );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let host = format!("http://{}", listener.local_addr().unwrap());
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            Self {
                dir,
                store,
                owner,
                account,
                base: format!("{host}/api/remote/v1"),
                host,
                deletes,
                stops,
                status,
                task,
            }
        }
        fn own(&self, sid: &str) -> Session {
            self.owner
                .own(self.account.clone(), &self.base, sid, Some("m1".into()))
                .unwrap()
        }
        async fn wait_empty(&self) {
            tokio::time::timeout(Duration::from_secs(2), async {
                while self.store.cleanup_snapshot().unwrap()[QUEUE]
                    .as_array()
                    .is_some_and(|rows| !rows.is_empty())
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn completed_sessions_are_deleted_and_cleanup_never_lists_other_sessions() {
        let f = Fixture::new().await;
        let mut session = f.own("own/id");
        assert!(session.url.ends_with("/chat_sessions/own%2Fid"));
        assert_eq!(
            f.store.cleanup_snapshot().unwrap()[QUEUE][0]["status"],
            "pending"
        );
        session.finished = true;
        let mut details = RequestDetails {
            upstream_summary: json!({"model":"test"}),
            ..Default::default()
        };
        session.cleanup(&mut details).await;
        assert_eq!(details.upstream_summary["cloudSessionCleanup"], "deleted");
        assert_eq!(details.upstream_summary["model"], "test");
        assert_eq!(*f.deletes.lock().unwrap(), vec!["own/id"]);
        assert_eq!(f.stops.load(Ordering::SeqCst), 0);
        assert!(f.store.cleanup_snapshot().unwrap()[QUEUE]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn cleanup_status_codes_and_errors_are_bounded_and_do_not_expose_server_text() {
        for (status, attempts, deleted) in [
            (404, 1, true),
            (401, 1, false),
            (403, 1, false),
            (500, 2, false),
            (200, 2, false),
        ] {
            let f = Fixture::new().await;
            f.status.store(status, Ordering::SeqCst);
            let mut session = f.own("new");
            session.finished = true;
            let mut details = RequestDetails::default();
            details.upstream_summary = json!({});
            session.cleanup(&mut details).await;
            assert_eq!(f.deletes.lock().unwrap().len(), attempts);
            assert_eq!(
                details.upstream_summary["cloudSessionCleanup"],
                if deleted { "deleted" } else { "failed" }
            );
            assert!(!serde_json::to_string(&details).unwrap().contains("PRIVATE"));
            if !deleted {
                assert_eq!(
                    f.store.cleanup_snapshot().unwrap()[QUEUE][0]["status"],
                    "failed"
                );
            }
        }
    }

    #[tokio::test]
    async fn cancellation_stops_then_deletes_and_durable_retry_skips_active_and_legacy() {
        let f = Fixture::new().await;
        f.store
            .update(|data| {
                data["requests"] = json!([{"upstreamSession":"legacy"}]);
                Ok(())
            })
            .unwrap();
        let session = f.own("cancelled");
        f.owner.retry(Some(&f.host)).await;
        assert!(f.deletes.lock().unwrap().is_empty());
        drop(session);
        f.wait_empty().await;
        assert_eq!(*f.deletes.lock().unwrap(), vec!["cancelled"]);
        assert_eq!(f.stops.load(Ordering::SeqCst), 1);

        f.status.store(500, Ordering::SeqCst);
        let mut session = f.own("retry");
        let mut details = RequestDetails::default();
        details.upstream_summary = json!({});
        session.cleanup(&mut details).await;
        f.status.store(204, Ordering::SeqCst);
        let restarted = Cleanup::new(Store::open(&f.dir).unwrap());
        restarted.retry(Some(&f.host)).await;
        let disk = Store::open(&f.dir).unwrap();
        assert!(disk.cleanup_snapshot().unwrap()[QUEUE]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(disk.snapshot()["requests"][0]["upstreamSession"], "legacy");
        assert_eq!(
            *f.deletes.lock().unwrap(),
            vec!["cancelled", "retry", "retry", "retry"]
        );
    }

    #[tokio::test]
    async fn independent_instances_preserve_ownership_and_skip_each_others_active_sessions() {
        let f = Fixture::new().await;
        // Open the second store before ownership is recorded: its account
        // snapshot is stale, while cleanup must always read the latest disk state.
        let other_store = Store::open(&f.dir).unwrap();
        let other = Cleanup::new(other_store.clone());
        let mut first = f.own("private-owned-first");
        let mut second = other
            .own(f.account.clone(), &f.base, "second", None)
            .unwrap();
        other_store.event("checkin", "independent account-vault write");
        assert_eq!(
            f.store.cleanup_snapshot().unwrap()[QUEUE]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        other.retry(Some(&f.host)).await;
        f.owner.retry(Some(&f.host)).await;
        assert!(f.deletes.lock().unwrap().is_empty());
        let bytes = std::fs::read(f.dir.join("remote-cleanup.enc")).unwrap();
        assert!(!bytes
            .windows(b"private-owned-first".len())
            .any(|w| w == b"private-owned-first"));

        first.finished = true;
        second.finished = true;
        f.status.store(500, Ordering::SeqCst);
        first.run().await.unwrap_err();
        f.status.store(204, Ordering::SeqCst);
        second.run().await.unwrap();
        assert_eq!(
            other_store.cleanup_snapshot().unwrap()[QUEUE]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        other.retry(Some(&f.host)).await;
        assert!(f.store.cleanup_snapshot().unwrap()[QUEUE]
            .as_array()
            .unwrap()
            .is_empty());
        // Successful records release and retire their per-session lock files.
        assert_eq!(
            std::fs::read_dir(f.dir.join("remote-cleanup-locks"))
                .unwrap()
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn cancelled_retry_releases_its_lease_for_a_later_pass() {
        let f = Fixture::new().await;
        f.status.store(500, Ordering::SeqCst);
        let mut session = f.own("slow-retry");
        session.finished = true;
        session.run().await.unwrap_err();
        f.status.store(204, Ordering::SeqCst);
        let owner = f.owner.clone();
        let host = f.host.clone();
        let task = tokio::spawn(async move { owner.retry(Some(&host)).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while f.deletes.lock().unwrap().len() < 3 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        f.owner.retry(Some(&f.host)).await;
        assert!(f.store.cleanup_snapshot().unwrap()[QUEUE]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(f.deletes.lock().unwrap().len(), 4);
    }
}
