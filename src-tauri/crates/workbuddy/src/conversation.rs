//! Bounded, memory-only session and turn identities. Message contents are hashed,
//! never retained. Explicit client session IDs take precedence over inference.
use axum::http::HeaderMap;
use serde_json::Value;
use std::{
    collections::{hash_map::RandomState, HashMap},
    hash::{BuildHasher, Hasher},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub struct Context {
    pub conversation: String,
    pub connection: String,
    pub request: String,
    pub root: String,
    pub trace: String,
    pub span: String,
    pub parent: String,
    pub sampled: &'static str,
    pub inferred: bool,
}
struct Session {
    conversation: String,
    connection: String,
    turns: HashMap<u64, String>,
    seen: Instant,
}
#[derive(Default)]
pub struct Sessions {
    hash: RandomState,
    entries: HashMap<u64, Session>,
}
fn valid_uuid(value: &str) -> Option<String> {
    Uuid::parse_str(value)
        .ok()
        .filter(|v| !v.is_nil())
        .map(|v| v.to_string())
}
pub fn client_session(headers: &HeaderMap, body: &Value) -> Option<String> {
    for name in ["x-conversation-id", "x-session-id"] {
        if let Some(id) = headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(valid_uuid)
        {
            return Some(id);
        }
    }
    let user = body["metadata"]["user_id"].as_str()?;
    if let Ok(parsed) = serde_json::from_str::<Value>(user) {
        if let Some(id) = parsed["session_id"].as_str().and_then(valid_uuid) {
            return Some(id);
        }
    }
    user.rsplit_once("_session_")
        .and_then(|(_, id)| valid_uuid(id))
}
fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value.bytes().all(|b| b.is_ascii_hexdigit())
        && value.bytes().any(|b| b != b'0')
}
impl Sessions {
    fn digest(&self, values: &[&[u8]]) -> u64 {
        let mut h = self.hash.build_hasher();
        for value in values {
            h.write_usize(value.len());
            h.write(value);
        }
        h.finish()
    }
    pub fn prepare(&mut self, headers: &HeaderMap, body: &mut Value) -> Context {
        let now = Instant::now();
        self.entries
            .retain(|_, s| now.duration_since(s.seen) < Duration::from_secs(3600));
        let explicit = client_session(headers, body);
        let messages = body["messages"].as_array().cloned().unwrap_or_default();
        // Strip only our own transport metadata from the digest. A model switch or
        // tool-result round must not change the originating human turn identity.
        let clean = messages
            .iter()
            .map(|m| {
                let mut m = m.clone();
                if let Some(o) = m.as_object_mut() {
                    for k in [
                        "agent",
                        "conversationRequestId",
                        "startsNewUserRequest",
                        "messageId",
                    ] {
                        o.remove(k);
                    }
                }
                m
            })
            .collect::<Vec<_>>();
        let first_user = clean.iter().find(|m| m["role"] == "user");
        let seed = serde_json::to_vec(&first_user).unwrap_or_default();
        let system = serde_json::to_vec(
            &clean
                .iter()
                .filter(|m| m["role"] == "system")
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        let key = if let Some(id) = &explicit {
            self.digest(&[b"explicit", id.as_bytes()])
        } else {
            self.digest(&[
                b"inferred",
                &seed,
                &system,
                headers.get("user-agent").map_or(&[], |v| v.as_bytes()),
            ])
        };
        if !self.entries.contains_key(&key) && self.entries.len() >= 1024 {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, s)| s.seen)
                .map(|(k, _)| *k)
            {
                self.entries.remove(&oldest);
            }
        }
        // Each genuine user message begins a turn; OpenAI tool results use role=tool.
        let mut turns = Vec::new();
        let mut history = self.hash.build_hasher();
        for (i, m) in clean.iter().enumerate() {
            let bytes = serde_json::to_vec(m).unwrap_or_default();
            history.write_usize(bytes.len());
            history.write(&bytes);
            if m["role"] == "user" && messages[i]["startsNewUserRequest"] != false {
                turns.push((i, history.finish()));
            }
        }
        let session = self.entries.entry(key).or_insert_with(|| Session {
            conversation: explicit
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            connection: Uuid::now_v7().to_string(),
            turns: HashMap::new(),
            seen: now,
        });
        session.seen = now;
        let recent: std::collections::HashSet<_> =
            turns.iter().rev().take(2048).map(|(_, key)| *key).collect();
        session.turns.retain(|key, _| recent.contains(key));
        for (_, key) in turns.iter().rev().take(2048) {
            session
                .turns
                .entry(*key)
                .or_insert_with(|| Uuid::now_v7().simple().to_string());
        }
        let root = turns
            .last()
            .map(|(_, k)| session.turns[k].clone())
            .unwrap_or_else(|| Uuid::now_v7().simple().to_string());
        let mut current = None;
        let mut turn_iter = turns.iter().peekable();
        if let Some(array) = body["messages"].as_array_mut() {
            for (i, message) in array.iter_mut().enumerate() {
                if turn_iter.peek().is_some_and(|(index, _)| *index == i) {
                    let (_, key) = turn_iter.next().unwrap();
                    current = session.turns.get(key).cloned();
                }
                let role = message["role"].as_str().unwrap_or("").to_owned();
                if ["user", "assistant", "tool"].contains(&role.as_str()) {
                    if let Some(o) = message.as_object_mut() {
                        o.entry("agent").or_insert(Value::String("cli".into()));
                        if let Some(id) = &current {
                            o.insert("conversationRequestId".into(), Value::String(id.clone()));
                        }
                        if role == "user" {
                            o.entry("startsNewUserRequest").or_insert(Value::Bool(true));
                        }
                    }
                }
            }
        }
        let incoming = headers
            .get("traceparent")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                let p = v.split('-').collect::<Vec<_>>();
                (p.len() == 4
                    && p[0] == "00"
                    && hex(p[1], 32)
                    && hex(p[2], 16)
                    && p[3].len() == 2
                    && p[3].bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| {
                    (
                        p[1].to_ascii_lowercase(),
                        p[2].to_ascii_lowercase(),
                        if u8::from_str_radix(p[3], 16).unwrap_or(0) & 1 == 1 {
                            "1"
                        } else {
                            "0"
                        },
                    )
                })
            });
        let (trace, parent, sampled) = incoming.unwrap_or_else(|| {
            (
                Uuid::new_v4().simple().to_string(),
                Uuid::new_v4().simple().to_string()[..16].into(),
                "1",
            )
        });
        Context {
            conversation: session.conversation.clone(),
            connection: session.connection.clone(),
            request: Uuid::now_v7().simple().to_string(),
            root,
            trace,
            parent,
            sampled,
            span: Uuid::new_v4().simple().to_string()[..16].into(),
            inferred: explicit.is_none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn tool_rounds_share_turn_but_new_user_turn_advances_root() {
        let mut sessions = Sessions::default();
        let mut h = HeaderMap::new();
        h.insert("x-session-id", Uuid::new_v4().to_string().parse().unwrap());
        let mut a =
            json!({"model":"deepseek-v4.1-flash","messages":[{"role":"user","content":"hello"}]});
        let one = sessions.prepare(&h, &mut a);
        a["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","tool_calls":[{"id":"t"}]}),
            json!({"role":"tool","tool_call_id":"t","content":"result"}),
        ]);
        let two = sessions.prepare(&h, &mut a);
        assert_eq!(one.root, two.root);
        assert_eq!(one.connection, two.connection);
        assert_eq!(one.conversation, two.conversation);
        assert_ne!(one.request, two.request);
        assert_eq!(a["messages"][1]["conversationRequestId"], one.root);
        a["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"role":"user","content":"next"}));
        let three = sessions.prepare(&h, &mut a);
        assert_ne!(one.root, three.root);
        assert_eq!(a["model"], "deepseek-v4.1-flash");
        assert_eq!(a["messages"][0]["content"], "hello");
    }
    #[test]
    fn claude_session_and_parent_span_are_respected_without_auth_passthrough() {
        let mut sessions = Sessions::default();
        let mut h = HeaderMap::new();
        h.insert(
            "traceparent",
            "00-12345678901234567890123456789012-1234567890123456-00"
                .parse()
                .unwrap(),
        );
        let id = Uuid::new_v4().to_string();
        let mut b = json!({"metadata":{"user_id":json!({"session_id":id}).to_string()},"messages":[{"role":"user","content":"test"}]});
        let c = sessions.prepare(&h, &mut b);
        assert_eq!(c.conversation, id);
        assert!(!c.inferred);
        assert_eq!(c.parent, "1234567890123456");
        assert_eq!(c.sampled, "0");
        assert_ne!(c.span, c.parent);
        assert_eq!(sessions.entries.len(), 1);
    }
}
