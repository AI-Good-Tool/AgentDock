//! Client-provided conversation identity. Explicit headers win over Claude
//! `metadata.user_id`; nothing is inferred from message contents here.
use serde_json::Value;

fn valid_uuid(value: &str) -> Option<String> {
    uuid::Uuid::parse_str(value)
        .ok()
        .filter(|v| !v.is_nil())
        .map(|v| v.to_string())
}

/// A UUID session supplied by the client, if any.
pub fn client_session(headers: &http::HeaderMap, body: &Value) -> Option<String> {
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

/// Any bounded opaque session label (non-UUID values allowed), for upstreams
/// that only need a stable string.
pub fn session_label(headers: &http::HeaderMap) -> Option<String> {
    headers
        .get("x-session-id")
        .or_else(|| headers.get("x-conversation-id"))
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn header_then_claude_metadata() {
        let id = "8f2b1c3e-1111-4222-8333-944455556666";
        let mut h = http::HeaderMap::new();
        assert_eq!(
            client_session(
                &h,
                &json!({"metadata":{"user_id":format!("user_x_account__session_{id}")}})
            )
            .as_deref(),
            Some(id)
        );
        assert_eq!(
            client_session(
                &h,
                &json!({"metadata":{"user_id":json!({"session_id":id}).to_string()}})
            )
            .as_deref(),
            Some(id)
        );
        h.insert("x-session-id", "not-a-uuid".parse().unwrap());
        assert!(client_session(&h, &json!({})).is_none());
        assert_eq!(session_label(&h).as_deref(), Some("not-a-uuid"));
        h.insert("x-conversation-id", id.parse().unwrap());
        assert_eq!(client_session(&h, &json!({})).as_deref(), Some(id));
    }
}
