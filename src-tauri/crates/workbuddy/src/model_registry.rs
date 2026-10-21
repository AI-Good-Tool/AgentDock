//! Region catalogs and local public names. Never equate different real model IDs.
use crate::pool::Store;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub models: Vec<Value>,
    pub active: Vec<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Policy {
    pub alias: String,
    /// None means all discovered regions; an explicit set only narrows permissions.
    pub regions: Option<Vec<String>>,
}
pub fn catalogs(store: &Store) -> HashMap<String, Catalog> {
    if !store.model_catalogs.is_empty() {
        return store.model_catalogs.clone();
    }
    let models = store.models.clone().unwrap_or_else(|| {
        serde_json::from_str(include_str!("../evidence/models.remote.json")).unwrap()
    });
    let active = store.active_models.clone().unwrap_or_else(|| {
        serde_json::from_str::<Value>(include_str!("../evidence/routing.remote.json"))
            .unwrap()["modelIds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    });
    HashMap::from([(
        store.catalog_variant.clone().unwrap_or_else(|| "cn".into()),
        Catalog { models, active },
    )])
}
fn config_in(
    store: &Store,
    catalogs: &HashMap<String, Catalog>,
    id: &str,
    region: &str,
) -> Option<Value> {
    if store
        .model_policies
        .get(id)
        .and_then(|p| p.regions.as_ref())
        .is_some_and(|r| !r.iter().any(|r| r == region))
    {
        return None;
    }
    let c = catalogs.get(region)?;
    if !c.active.iter().any(|r| r == id) {
        return None;
    }
    c.models.iter().find(|m| m["id"] == id).cloned()
}
pub fn config(store: &Store, id: &str, region: &str) -> Option<Value> {
    if !store.model_catalogs.is_empty() {
        return config_in(store, &store.model_catalogs, id, region);
    }
    config_in(store, &catalogs(store), id, region)
}
/// Exact region configuration, ignoring legacy local aliases/region policies
/// (those are migrated into the unified routing table).
pub fn region_config(store: &Store, id: &str, region: &str) -> Option<Value> {
    let catalogs = catalogs(store);
    let c = catalogs.get(region)?;
    if !c.active.iter().any(|r| r == id) {
        return None;
    }
    c.models.iter().find(|m| m["id"] == id).cloned()
}
pub fn upstream_models(store: &Store) -> Vec<agentdock_core::provider::UpstreamModel> {
    let catalogs = catalogs(store);
    let mut out = vec![];
    for variant in ["cn", "ai"] {
        let Some(c) = catalogs.get(variant) else {
            continue;
        };
        for model in &c.models {
            let Some(id) = model["id"].as_str() else {
                continue;
            };
            if !c.active.iter().any(|a| a == id)
                || crate::protocol::validate(
                    &json!({"model":id,"messages":[{"role":"user","content":"metadata"}]}),
                    std::slice::from_ref(model),
                )
                .is_err()
            {
                continue;
            }
            let reasoning = &model["reasoning"];
            out.push(agentdock_core::provider::UpstreamModel {
                provider: Some(agentdock_core::provider::ProviderKind::Workbuddy),
                region: crate::pool::unified_region(variant).into(),
                id: id.into(),
                name: model["name"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or(id).into(),
                supports_tools: model["supportsToolCall"].as_bool(),
                supports_images: model["supportsImages"].as_bool(),
                efforts: reasoning["supportedEfforts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                default_effort: reasoning["defaultEffort"]
                    .as_str()
                    .or_else(|| reasoning["effort"].as_str())
                    .map(str::to_owned),
                context_window: model["maxInputTokens"].as_u64(),
                max_output: model["maxOutputTokens"].as_u64(),
                config: model.clone(),
            });
        }
    }
    out
}
pub fn request_supported(body: &Value, config: &Value) -> bool {
    if crate::protocol::validate(body, std::slice::from_ref(config)).is_err() {
        return false;
    }
    if let (Some(requested), Some(limit)) = (
        body["max_tokens"].as_u64(),
        config["maxOutputTokens"].as_u64(),
    ) {
        if requested > limit {
            return false;
        }
    }
    if config["supportsToolCall"] == false
        && body["tools"].as_array().is_some_and(|a| !a.is_empty())
    {
        return false;
    }
    if config["supportsImages"] == false
        && body["messages"].as_array().into_iter().flatten().any(|m| {
            m["content"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|p| p["type"] == "image_url")
        })
    {
        return false;
    }
    true
}
pub fn models(store: &Store) -> Vec<Value> {
    let mut rows = BTreeMap::<String, Value>::new();
    let catalogs = catalogs(store);
    // Stable precedence for display only; execution always uses its region's exact config.
    for region in ["cn", "ai"] {
        if let Some(c) = catalogs.get(region) {
            for model in &c.models {
                let Some(id) = model["id"].as_str() else {
                    continue;
                };
                let valid = crate::protocol::validate(
                    &json!({"model":id,"messages":[{"role":"user","content":"metadata"}]}),
                    std::slice::from_ref(model),
                )
                .is_ok();
                let row = rows.entry(id.into()).or_insert_with(|| model.clone());
                if valid
                    && crate::protocol::validate(
                        &json!({"model":id,"messages":[{"role":"user","content":"metadata"}]}),
                        std::slice::from_ref(row),
                    )
                    .is_err()
                {
                    *row = model.clone();
                }
            }
        }
    }
    rows.into_iter()
        .map(|(id, mut row)| {
            let p = store.model_policies.get(&id);
            let alias = p
                .map(|p| p.alias.as_str())
                .filter(|a| !a.is_empty())
                .unwrap_or(&id);
            let compatible: Vec<_> = ["cn", "ai"]
                .into_iter()
                .filter_map(|r| config_in(store, &catalogs, &id, r))
                .filter(|m| {
                    crate::protocol::validate(
                        &json!({"model":id,"messages":[{"role":"user","content":"metadata"}]}),
                        std::slice::from_ref(m),
                    )
                    .is_ok()
                })
                .collect();
            for flag in ["supportsImages", "supportsToolCall", "supportsReasoning"] {
                if compatible.iter().any(|m| m[flag] == true) {
                    row[flag] = json!(true);
                } else if compatible.iter().any(|m| m[flag] == false) {
                    row[flag] = json!(false);
                }
            }
            // Advertise a conservative input window; output budgets are filtered per request.
            if let Some(limit) = compatible
                .iter()
                .filter_map(|m| m["maxInputTokens"].as_u64())
                .min()
            {
                row["maxInputTokens"] = json!(limit);
            }
            if let Some(limit) = compatible
                .iter()
                .filter_map(|m| m["maxOutputTokens"].as_u64())
                .max()
            {
                row["maxOutputTokens"] = json!(limit);
            }
            row["alias"] = json!(alias);
            row["routingRegions"] = json!(p.and_then(|p| p.regions.as_ref()));
            row["regions"] = json!(["cn", "ai"]
                .into_iter()
                .filter(|r| config_in(store, &catalogs, &id, r).is_some())
                .collect::<Vec<_>>());
            row["catalogRegions"] = json!(["cn", "ai"]
                .into_iter()
                .filter(|r| catalogs
                    .get(*r)
                    .is_some_and(|c| c.models.iter().any(|m| m["id"] == id)))
                .collect::<Vec<_>>());
            row
        })
        .collect()
}
pub fn active(store: &Store) -> Vec<String> {
    models(store)
        .into_iter()
        .filter(|m| m["regions"].as_array().is_some_and(|r| !r.is_empty()))
        .filter_map(|m| m["id"].as_str().map(str::to_owned))
        .collect()
}
pub fn resolve(store: &Store, public: &str) -> Option<(String, String)> {
    let rows = models(store);
    let row = rows
        .iter()
        .find(|m| m["alias"] == public)
        .or_else(|| rows.iter().find(|m| m["id"] == public))?;
    Some((row["id"].as_str()?.into(), row["alias"].as_str()?.into()))
}
pub fn validate_policy(store: &Store, id: &str, policy: &Policy) -> Result<(), String> {
    let rows = models(store);
    if !rows.iter().any(|m| m["id"] == id) {
        return Err("模型不在已发现目录中".into());
    }
    let alias = if policy.alias.is_empty() {
        id
    } else {
        policy.alias.as_str()
    };
    if alias.len() > 128
        || alias.is_empty()
        || !alias
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-:/".contains(&b))
    {
        return Err("别名需为 1–128 个字母、数字或 . _ - : / 字符；留空恢复真实模型 ID".into());
    }
    if rows
        .iter()
        .any(|m| m["id"] != id && (m["id"] == alias || m["alias"] == alias))
    {
        return Err("别名与其他模型的真实 ID 或别名冲突".into());
    }
    if let Some(regions) = &policy.regions {
        if regions.iter().any(|r| !["cn", "ai"].contains(&r.as_str())) {
            return Err("无效的地区".into());
        }
    }
    Ok(())
}

/// Incremental SSE metadata rewriting; text/tool arguments are never string-replaced.
#[derive(Default)]
pub struct ModelSse {
    pending: Vec<u8>,
}
impl ModelSse {
    pub fn feed(&mut self, bytes: &[u8], name: &str) -> Result<Vec<u8>, String> {
        let mut out = vec![];
        for &b in bytes {
            self.pending.push(b);
            if self.pending.len() > crate::MAX_REQUEST_BODY_BYTES {
                return Err("上游 SSE 事件超过安全上限".into());
            }
            if self.pending.ends_with(b"\n\n") || self.pending.ends_with(b"\r\n\r\n") {
                out.extend(self.line(name));
            }
        }
        Ok(out)
    }
    fn line(&mut self, name: &str) -> Vec<u8> {
        let line = std::mem::take(&mut self.pending);
        if let Ok(text) = std::str::from_utf8(&line) {
            let data = text
                .lines()
                .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
                .collect::<Vec<_>>()
                .join("\n");
            if let Ok(mut value) = serde_json::from_str::<Value>(&data) {
                if value.get("model").is_some() {
                    value["model"] = json!(name);
                    let mut lines = vec![];
                    let mut inserted = false;
                    for part in text.lines().filter(|l| !l.is_empty()) {
                        if part.starts_with("data:") {
                            if !inserted {
                                lines.push(format!("data: {value}"));
                                inserted = true;
                            }
                        } else {
                            lines.push(part.to_owned());
                        }
                    }
                    return (lines.join("\n") + "\n\n").into_bytes();
                }
            }
        }
        line
    }
    pub fn finish(&mut self, name: &str) -> Vec<u8> {
        self.line(name)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Store {
        let mut s = Store::default();
        s.model_catalogs.insert(
            "cn".into(),
            Catalog {
                models: vec![json!({"id":"shared"}), json!({"id":"cn-only"})],
                active: vec!["shared".into(), "cn-only".into()],
            },
        );
        s.model_catalogs.insert(
            "ai".into(),
            Catalog {
                models: vec![json!({"id":"shared"}), json!({"id":"ai-only"})],
                active: vec!["shared".into(), "ai-only".into()],
            },
        );
        s
    }
    #[test]
    fn union_permissions_aliases_and_collisions() {
        let mut s = store();
        assert_eq!(models(&s).len(), 3);
        assert!(config(&s, "shared", "ai").is_some());
        assert!(config(&s, "cn-only", "ai").is_none());
        let p = Policy {
            alias: "my-model".into(),
            regions: Some(vec!["ai".into()]),
        };
        validate_policy(&s, "shared", &p).unwrap();
        s.model_policies.insert("shared".into(), p);
        assert_eq!(resolve(&s, "my-model").unwrap().0, "shared");
        assert!(config(&s, "shared", "cn").is_none());
        assert!(validate_policy(
            &s,
            "cn-only",
            &Policy {
                alias: "my-model".into(),
                regions: None
            }
        )
        .is_err());
        assert!(validate_policy(
            &s,
            "cn-only",
            &Policy {
                alias: "ai-only".into(),
                regions: None
            }
        )
        .is_err());
    }
    #[test]
    fn multiline_sse_keeps_event_metadata_and_only_changes_model() {
        let mut p = ModelSse::default();
        let src=b"event: chunk\nid: 7\ndata: {\ndata: \"model\":\"real\",\ndata: \"text\":\"real\"}\n\n";
        let out = String::from_utf8(p.feed(src, "public").unwrap()).unwrap();
        assert!(out.contains("event: chunk"));
        assert!(out.contains("id: 7"));
        assert!(out.contains("\"model\":\"public\""));
        assert!(out.contains("\"text\":\"real\""));
    }
    #[test]
    fn sse_alias_never_rewrites_assistant_or_tool_payloads() {
        let mut parser = ModelSse::default();
        let source=b"data: {\"model\":\"real\",\"choices\":[{\"delta\":{\"content\":\"real\"}}]}\r\n\ndata: [DONE]\n\n";
        let mut out = vec![];
        for b in source {
            out.extend(parser.feed(&[*b], "alias").unwrap());
        }
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\"model\":\"alias\""));
        assert!(s.contains("\"content\":\"real\""));
        assert!(s.contains("[DONE]"));
    }
}
