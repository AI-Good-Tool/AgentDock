//! Unified model routing table. Identical official model names merge across
//! providers and regions into one public route; same-catalog name collisions
//! stay apart. Aliases and explicit upstream lists come from settings.
use crate::settings::{RouteOverride, Settings};
use agentdock_core::anthropic::{closest_effort, EFFORT_ORDER};
use agentdock_core::provider::{group_key, ProviderKind, UpstreamModel};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub route_id: String,
    /// Public name clients use and see in responses.
    pub alias: String,
    pub default_alias: String,
    /// Every mapped upstream (including providers/regions currently out of scope).
    pub all_targets: Vec<UpstreamModel>,
    /// Upstreams usable under the global provider scope.
    pub targets: Vec<UpstreamModel>,
    /// Explicit targets that are missing from the current catalogs.
    pub missing: Vec<String>,
    pub configured: bool,
    pub custom: bool,
    pub disabled: bool,
    /// Routes merged into this one (`RouteOverride::merge_into`).
    pub linked: Vec<LinkedRoute>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedRoute {
    pub route_id: String,
    pub alias: String,
    /// Keys of the upstreams it contributed.
    pub targets: Vec<String>,
}
impl Route {
    /// Names that select this route: its alias and those of merged routes.
    fn answers_to(&self, name: &str) -> bool {
        self.alias == name || self.linked.iter().any(|l| l.alias == name)
    }
    pub fn available(&self) -> bool {
        !self.disabled && !self.targets.is_empty()
    }
    pub fn providers(&self) -> Vec<ProviderKind> {
        let mut out: Vec<_> = self.targets.iter().map(|t| t.provider()).collect();
        out.sort();
        out.dedup();
        out
    }
    pub fn scoped(&self, scope: &Scope) -> Vec<&UpstreamModel> {
        self.targets.iter().filter(|t| scope.allows(t)).collect()
    }
    /// Levels a client may offer for this route, in Codex's model-catalog
    /// shape. Any of them is accepted: each request is mapped to the closest
    /// level of the account that serves it.
    pub fn reasoning_levels(&self, scope: &Scope) -> (Value, Value) {
        let targets = self.scoped(scope);
        let levels: Vec<&str> = EFFORT_ORDER
            .into_iter()
            .filter(|e| targets.iter().any(|t| t.efforts.iter().any(|x| x == e)))
            .collect();
        let default = targets
            .iter()
            .filter_map(|t| t.default_effort.as_deref())
            .find(|e| levels.contains(e))
            .or_else(|| closest_effort("high", &levels));
        let described = levels.iter().map(|e| json!({"effort":e,"description":effort_label(e)})).collect();
        (Value::Array(described), json!(default))
    }
    /// Capabilities shared by every in-scope upstream (never guess support).
    pub fn view(&self, scope: &Scope) -> Value {
        let targets = self.scoped(scope);
        let all = |f: &dyn Fn(&UpstreamModel) -> Option<bool>| {
            !targets.is_empty() && targets.iter().all(|t| f(t) == Some(true))
        };
        let windows: Vec<u64> = targets.iter().filter_map(|t| t.context_window).collect();
        let efforts: Vec<String> = targets
            .first()
            .map(|first| {
                first
                    .efforts
                    .iter()
                    .filter(|e| targets.iter().all(|t| t.efforts.contains(e)))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut regions: Vec<&str> = targets.iter().map(|t| t.region.as_str()).collect();
        regions.sort();
        regions.dedup();
        json!({
            "routeId": self.route_id,
            "alias": self.alias,
            "defaultAlias": self.default_alias,
            "available": !self.disabled && !targets.is_empty(),
            "disabled": self.disabled,
            "configured": self.configured,
            "custom": self.custom,
            "supportsTools": all(&|t| t.supports_tools),
            "supportsImages": all(&|t| t.supports_images),
            "contextWindow": if !targets.is_empty() && windows.len() == targets.len() { json!(windows.iter().min()) } else { Value::Null },
            "efforts": efforts,
            "providers": targets.iter().map(|t| t.provider().key()).collect::<HashSet<_>>(),
            "regions": regions,
            "targets": targets.iter().map(|t| target_view(t)).collect::<Vec<_>>(),
            "allTargets": self.all_targets.iter().map(target_view).collect::<Vec<_>>(),
            "missing": self.missing,
            "linked": self.linked,
        })
    }
}
fn effort_label(effort: &str) -> &'static str {
    match effort {
        "none" => "关闭思考",
        "minimal" => "最少推理",
        "low" => "轻量推理，响应更快",
        "medium" => "均衡推理",
        "high" => "增强推理",
        "xhigh" => "深度推理",
        "max" => "最大推理",
        _ => "极限推理",
    }
}
pub fn target_view(t: &UpstreamModel) -> Value {
    json!({"key":t.key(),"provider":t.provider().key(),"region":t.region,"id":t.id,"name":t.name,
        "supportsTools":t.supports_tools,"supportsImages":t.supports_images,"efforts":t.efforts,
        "contextWindow":t.context_window,"maxOutput":t.max_output})
}

/// Path / pin restriction for one request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scope {
    pub provider: Option<ProviderKind>,
    pub region: Option<String>,
}
impl Scope {
    pub fn allows(&self, t: &UpstreamModel) -> bool {
        self.provider.is_none_or(|p| p == t.provider())
            && self.region.as_deref().is_none_or(|r| r == t.region)
    }
    /// Settings key used for pinned accounts ("all", "cn", "trae", "trae/intl"…).
    pub fn key(&self) -> String {
        match (&self.provider, &self.region) {
            (None, None) => "all".into(),
            (None, Some(r)) => r.clone(),
            (Some(p), None) => p.key().into(),
            (Some(p), Some(r)) => format!("{}/{r}", p.key()),
        }
    }
}

fn auto_group(m: &UpstreamModel, ambiguous: bool) -> String {
    if m.id == "auto" {
        // "Let the provider choose" is not one model; never merge it across providers.
        return format!("{}:auto", m.provider().key());
    }
    if ambiguous {
        group_key(&m.id)
    } else {
        group_key(&m.name)
    }
}

/// Build routes from every provider catalog plus saved overrides.
pub fn build(models: &[UpstreamModel], settings: &Settings) -> Vec<Route> {
    let in_scope = |m: &UpstreamModel| {
        let s = settings.scope(m.provider());
        s.enabled && s.regions.iter().any(|r| *r == m.region)
    };
    let by_key: HashMap<String, &UpstreamModel> = models.iter().map(|m| (m.key(), m)).collect();
    let claimed: HashSet<String> = settings
        .routes
        .values()
        .filter_map(|o| o.targets.as_ref())
        .flatten()
        .cloned()
        .collect();
    // Names repeated inside one provider-region catalog (e.g. several "Hy3"
    // variants) are different models: group those by id instead of name.
    let mut names: HashMap<(ProviderKind, &str, String), HashSet<&str>> = HashMap::new();
    for m in models {
        names
            .entry((m.provider(), m.region.as_str(), group_key(&m.name)))
            .or_default()
            .insert(m.id.as_str());
    }
    // Value: (members, grouped by id because the display name was ambiguous).
    let mut groups: BTreeMap<String, (Vec<UpstreamModel>, bool)> = BTreeMap::new();
    for m in models {
        if claimed.contains(&m.key()) {
            continue;
        }
        let ambiguous = names[&(m.provider(), m.region.as_str(), group_key(&m.name))].len() > 1;
        let entry = groups.entry(auto_group(m, ambiguous)).or_default();
        entry.0.push(m.clone());
        entry.1 |= ambiguous && m.id != "auto";
    }
    let mut routes: Vec<Route> = vec![];
    for (id, (mut members, by_id)) in groups {
        members.sort_by(|a, b| (a.provider(), &a.region, &a.id).cmp(&(b.provider(), &b.region, &b.id)));
        let default_alias = if by_id {
            members[0].id.clone()
        } else {
            preferred_name(&members)
        };
        routes.push(route(id, default_alias, members, Some(&settings.routes), &by_key, &in_scope));
    }
    // Explicit and custom routes (also kept visible when catalogs omit their models).
    for (id, o) in &settings.routes {
        if routes.iter().any(|r| r.route_id == *id) {
            continue;
        }
        if o.targets.is_none() {
            continue; // alias-only override for a route that currently has no models
        }
        let members: Vec<UpstreamModel> = o
            .targets
            .iter()
            .flatten()
            .filter_map(|k| by_key.get(k).map(|m| (*m).clone()))
            .collect();
        let default_alias = members
            .first()
            .map(|_| preferred_name(&members))
            .unwrap_or_else(|| id.clone());
        routes.push(route(id.clone(), default_alias, members, Some(&settings.routes), &by_key, &in_scope));
    }
    // Merged routes join their destination with whatever upstreams they have
    // now. One hop only: a destination that is itself merged is ignored. A
    // disabled source stays on its own so its upstreams remain unscheduled.
    for (src, o) in &settings.routes {
        let Some(dst) = o.merge_into.as_deref() else { continue };
        if dst == src || settings.routes.get(dst).is_some_and(|d| d.merge_into.is_some())
            || !routes.iter().any(|r| r.route_id == dst) {
            continue;
        }
        let Some(at) = routes.iter().position(|r| r.route_id == *src && !r.disabled) else { continue };
        let source = routes.remove(at);
        let dest = routes.iter_mut().find(|r| r.route_id == dst).unwrap();
        let mut keys = vec![];
        for t in source.all_targets {
            keys.push(t.key());
            if !dest.all_targets.iter().any(|x| x.key() == t.key()) {
                if in_scope(&t) {
                    dest.targets.push(t.clone());
                }
                dest.all_targets.push(t);
            }
        }
        dest.missing.extend(source.missing);
        dest.linked.push(LinkedRoute { route_id: source.route_id, alias: source.alias, targets: keys });
    }
    // Disambiguate colliding default aliases; configured aliases are validated on save.
    let snapshot: Vec<(String, String)> = routes
        .iter()
        .map(|r| (r.route_id.clone(), r.alias.clone()))
        .collect();
    for r in &mut routes {
        let explicit = settings.routes.get(&r.route_id).is_some_and(|o| !o.alias.is_empty());
        if explicit {
            continue;
        }
        let clash = snapshot.iter().any(|(id, alias)| *id != r.route_id && *alias == r.alias);
        if clash {
            let suffix = match r.route_id.strip_suffix(":auto").and_then(ProviderKind::parse) {
                Some(p) => p.label().to_owned(),
                None => r.route_id.clone(),
            };
            r.alias = format!("{} [{suffix}]", r.alias);
        }
    }
    routes.sort_by(|a, b| a.alias.to_lowercase().cmp(&b.alias.to_lowercase()));
    routes
}

fn route(
    id: String,
    default_alias: String,
    members: Vec<UpstreamModel>,
    overrides: Option<&BTreeMap<String, RouteOverride>>,
    by_key: &HashMap<String, &UpstreamModel>,
    in_scope: &dyn Fn(&UpstreamModel) -> bool,
) -> Route {
    let o = overrides.and_then(|o| o.get(&id));
    let (all_targets, missing) = match o.and_then(|o| o.targets.as_ref()) {
        Some(keys) => (
            keys.iter()
                .filter_map(|k| by_key.get(k).map(|m| (*m).clone()))
                .collect::<Vec<_>>(),
            keys.iter()
                .filter(|k| !by_key.contains_key(*k))
                .cloned()
                .collect(),
        ),
        None => (members, vec![]),
    };
    let alias = o
        .map(|o| o.alias.trim().to_owned())
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| default_alias.clone());
    Route {
        targets: all_targets.iter().filter(|m| in_scope(m)).cloned().collect(),
        all_targets,
        missing,
        alias,
        default_alias,
        configured: o.is_some(),
        custom: o.is_some_and(|o| o.custom),
        disabled: o.is_some_and(|o| o.disabled),
        linked: vec![],
        route_id: id,
    }
}

/// Most common official display name; ties prefer provider order.
fn preferred_name(members: &[UpstreamModel]) -> String {
    let mut counts: Vec<(String, usize, usize)> = vec![];
    for (i, m) in members.iter().enumerate() {
        if let Some(c) = counts.iter_mut().find(|c| c.0 == m.name) {
            c.1 += 1;
        } else {
            counts.push((m.name.clone(), 1, i));
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    counts[0].0.clone()
}

/// Resolve a requested model: exact alias, then a unique real upstream id,
/// then a unique normalized alias. Ambiguity fails closed.
pub fn resolve<'a>(routes: &'a [Route], requested: &str) -> Result<&'a Route, String> {
    if requested.trim().is_empty() {
        return Err("model 必须为非空字符串".into());
    }
    let live = |r: &&Route| !r.disabled;
    if let Some(r) = routes.iter().filter(live).find(|r| r.answers_to(requested)) {
        return Ok(r);
    }
    let by_id: Vec<&Route> = routes
        .iter()
        .filter(live)
        .filter(|r| r.all_targets.iter().any(|t| t.id == requested))
        .collect();
    match by_id.len() {
        1 => return Ok(by_id[0]),
        n if n > 1 => {
            return Err(format!(
                "模型 ID「{requested}」对应多个路由（{}），请改用别名",
                by_id.iter().map(|r| r.alias.as_str()).collect::<Vec<_>>().join("、")
            ))
        }
        _ => {}
    }
    let key = group_key(requested);
    let by_key: Vec<&Route> = routes
        .iter()
        .filter(live)
        .filter(|r| {
            group_key(&r.alias) == key || r.route_id == key
                || r.linked.iter().any(|l| group_key(&l.alias) == key || l.route_id == key)
        })
        .collect();
    if by_key.len() == 1 {
        return Ok(by_key[0]);
    }
    if routes.iter().any(|r| r.disabled && r.answers_to(requested)) {
        return Err("此模型已在模型目录中停用".into());
    }
    Err("模型或别名不存在（ID 有歧义时请使用别名），请先同步模型目录".into())
}

/// Validate a proposed override against the current routing table.
pub fn validate_override(
    models: &[UpstreamModel],
    settings: &Settings,
    route_id: &str,
    o: &RouteOverride,
) -> Result<(), String> {
    let alias = o.alias.trim();
    if !alias.is_empty()
        && (alias.chars().count() > 128 || alias.chars().any(char::is_control) || alias == "auto" && o.custom)
    {
        return Err("模型别名需为 1–128 个字符，不能包含控制字符".into());
    }
    if let Some(targets) = &o.targets {
        if targets.is_empty() {
            return Err("请至少选择一个上游模型".into());
        }
        let known: HashSet<String> = models.iter().map(|m| m.key()).collect();
        if let Some(bad) = targets.iter().find(|k| !known.contains(*k)) {
            return Err(format!("上游模型 {bad} 不在已同步目录中，请先同步"));
        }
        let mut seen = HashSet::new();
        if targets.iter().any(|k| !seen.insert(k)) {
            return Err("上游模型重复".into());
        }
    }
    if let Some(dst) = &o.merge_into {
        if dst == route_id {
            return Err("不能合并到自身".into());
        }
        if !build(models, settings).iter().any(|r| r.route_id == *dst) {
            return Err("要合并到的模型不存在，请先同步模型目录".into());
        }
        if settings.routes.get(dst).is_some_and(|d| d.merge_into.is_some()) {
            return Err("目标模型已合并到其他模型，请直接合并到最终模型".into());
        }
        if settings.routes.iter().any(|(id, other)| id != route_id && other.merge_into.as_deref() == Some(route_id)) {
            return Err("已有其他模型合并到此模型，请先取消那些关联".into());
        }
    }
    let mut next = settings.clone();
    // A target can belong to one explicit route only.
    if let Some(targets) = &o.targets {
        for (id, other) in next.routes.iter_mut() {
            if id != route_id {
                if let Some(t) = other.targets.as_mut() {
                    if t.iter().any(|k| targets.contains(k)) {
                        return Err("所选上游已属于其他自定义映射，请先从那里移除".into());
                    }
                }
            }
        }
    }
    next.routes.insert(route_id.into(), o.clone());
    let routes = build(models, &next);
    let mut used = HashSet::new();
    for r in &routes {
        let linked = r.linked.iter().map(|l| &l.alias).filter(|a| **a != r.alias);
        for alias in std::iter::once(&r.alias).chain(linked) {
            if !used.insert(alias.clone()) {
                return Err(format!("模型别名重复：{alias}"));
            }
        }
    }
    if !alias.is_empty()
        && routes.iter().any(|r| {
            r.route_id != route_id && r.all_targets.iter().any(|t| t.id == alias)
        })
    {
        return Err("别名不能与其他模型的真实 ID 冲突".into());
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    pub fn model(p: ProviderKind, region: &str, id: &str, name: &str) -> UpstreamModel {
        UpstreamModel {
            provider: Some(p),
            region: region.into(),
            id: id.into(),
            name: name.into(),
            supports_tools: Some(true),
            supports_images: Some(true),
            ..UpstreamModel::default()
        }
    }
    fn catalog() -> Vec<UpstreamModel> {
        use ProviderKind::*;
        vec![
            model(Workbuddy, "cn", "glm-5.3-flash", "GLM-5.3-Flash"),
            model(Workbuddy, "intl", "glm-5.3-flash", "GLM-5.3-Flash"),
            model(Qoder, "cn", "gfmodel", "GLM-5.3-Flash"),
            model(Qoder, "intl", "dfmodel", "DeepSeek-Flash"),
            model(Workbuddy, "cn", "hy3", "Hy3"),
            model(Workbuddy, "cn", "hy3-x", "Hy3"),
            model(Workbuddy, "cn", "auto", "Auto"),
            model(Trae, "cn", "auto", "auto"),
            model(Trae, "cn", "kimi-k3", "Kimi-K3"),
            model(Workbuddy, "cn", "kimi-k3-1", "Kimi-K3"),
        ]
    }
    #[test]
    fn identical_names_merge_across_providers_but_not_within_a_catalog() {
        let routes = build(&catalog(), &Settings::default());
        let glm = resolve(&routes, "GLM-5.3-Flash").unwrap();
        assert_eq!(glm.targets.len(), 3);
        assert_eq!(resolve(&routes, "glm-5.3-flash").unwrap().route_id, glm.route_id);
        assert_eq!(resolve(&routes, "gfmodel").unwrap().route_id, glm.route_id);
        // Two different WorkBuddy models share the display name "Hy3": keep both.
        let hy3 = resolve(&routes, "hy3").unwrap();
        let hy3x = resolve(&routes, "hy3-x").unwrap();
        assert_ne!(hy3.route_id, hy3x.route_id);
        assert_eq!(hy3.targets.len(), 1);
        // "auto" stays per provider.
        assert_eq!(resolve(&routes, "Auto").unwrap().targets[0].provider(), ProviderKind::Workbuddy);
        assert_eq!(resolve(&routes, "auto").unwrap().targets[0].provider(), ProviderKind::Trae);
        // Kimi-K3 merges across Trae and WorkBuddy by official name.
        assert_eq!(resolve(&routes, "Kimi-K3").unwrap().targets.len(), 2);
        assert_eq!(resolve(&routes, "kimi-k3-1").unwrap().alias, "Kimi-K3");
        assert!(resolve(&routes, "missing").is_err());
    }
    #[test]
    fn aliases_explicit_targets_scope_and_conflicts() {
        let models = catalog();
        let mut s = Settings::default();
        let glm = resolve(&build(&models, &s), "GLM-5.3-Flash").unwrap().route_id.clone();
        let o = RouteOverride { alias: "my-glm".into(), ..RouteOverride::default() };
        validate_override(&models, &s, &glm, &o).unwrap();
        s.routes.insert(glm.clone(), o);
        let routes = build(&models, &s);
        assert_eq!(resolve(&routes, "my-glm").unwrap().targets.len(), 3);
        assert!(resolve(&routes, "GLM-5.3-Flash").is_err() || resolve(&routes, "GLM-5.3-Flash").unwrap().alias == "my-glm");
        // Explicit split: Qoder GLM becomes its own custom route.
        let custom = RouteOverride { alias: "qoder-glm".into(), targets: Some(vec!["qoder:cn:gfmodel".into()]), custom: true, ..RouteOverride::default() };
        validate_override(&models, &s, "custom:1", &custom).unwrap();
        s.routes.insert("custom:1".into(), custom);
        let routes = build(&models, &s);
        assert_eq!(resolve(&routes, "my-glm").unwrap().targets.len(), 2);
        assert_eq!(resolve(&routes, "qoder-glm").unwrap().targets[0].id, "gfmodel");
        // Duplicate aliases and alias == other real id are rejected.
        let dup = RouteOverride { alias: "qoder-glm".into(), ..RouteOverride::default() };
        assert!(validate_override(&models, &s, &glm, &dup).is_err());
        let shadow = RouteOverride { alias: "dfmodel".into(), ..RouteOverride::default() };
        assert!(validate_override(&models, &s, &glm, &shadow).is_err());
        // Provider scope removes out-of-scope targets but keeps the mapping.
        s.providers.get_mut(&ProviderKind::Workbuddy).unwrap().regions = vec!["cn".into()];
        let routes = build(&models, &s);
        let r = resolve(&routes, "my-glm").unwrap();
        assert_eq!(r.all_targets.len(), 2);
        assert_eq!(r.targets.len(), 1);
        let scope = Scope { provider: Some(ProviderKind::Qoder), region: None };
        assert!(r.scoped(&scope).is_empty());
        assert_eq!(scope.key(), "qoder");
    }
    #[test]
    fn merged_route_pools_both_upstream_sets_and_keeps_old_names() {
        use ProviderKind::*;
        let models = vec![
            model(Workbuddy, "cn", "deepseek-v4.1-flash", "DeepSeek-V4.1-Flash"),
            model(Trae, "cn", "deepseek-v4.1-flash", "DeepSeek-V4.1-Flash"),
            model(Qoder, "cn", "dfmodel", "DeepSeek-Flash"),
            model(Qoder, "intl", "dfmodel", "DeepSeek-Flash"),
            model(Workbuddy, "cn", "glm", "GLM"),
        ];
        let mut s = Settings::default();
        let routes = build(&models, &s);
        let v41 = resolve(&routes, "DeepSeek-V4.1-Flash").unwrap().route_id.clone();
        let flash = resolve(&routes, "DeepSeek-Flash").unwrap().route_id.clone();
        let glm = resolve(&routes, "GLM").unwrap().route_id.clone();
        let link = |to: &str| RouteOverride { merge_into: Some(to.into()), ..RouteOverride::default() };
        validate_override(&models, &s, &flash, &link(&v41)).unwrap();
        s.routes.insert(flash.clone(), link(&v41));
        let routes = build(&models, &s);
        assert_eq!(routes.len(), 2, "the merged route is no longer listed on its own");
        let merged = resolve(&routes, "DeepSeek-V4.1-Flash").unwrap();
        assert_eq!(merged.targets.len(), 4);
        assert_eq!(merged.linked.len(), 1);
        assert_eq!(merged.linked[0].alias, "DeepSeek-Flash");
        assert_eq!(merged.linked[0].targets, ["qoder:cn:dfmodel", "qoder:intl:dfmodel"]);
        for name in ["DeepSeek-Flash", "dfmodel", "deepseek flash"] {
            assert_eq!(resolve(&routes, name).unwrap().route_id, v41, "{name}");
        }
        // Scope still applies per upstream; the link itself is kept.
        s.providers.get_mut(&Qoder).unwrap().regions = vec!["cn".into()];
        let merged = build(&models, &s).into_iter().find(|r| r.route_id == v41).unwrap();
        assert_eq!((merged.all_targets.len(), merged.targets.len()), (4, 3));
        // One hop only, never into itself.
        assert!(validate_override(&models, &s, &glm, &link(&glm)).is_err());
        assert!(validate_override(&models, &s, &glm, &link(&flash)).is_err());
        assert!(validate_override(&models, &s, &v41, &link(&glm)).is_err());
        assert!(validate_override(&models, &s, &glm, &link("missing")).is_err());
        validate_override(&models, &s, &glm, &link(&v41)).unwrap();
        // A disabled source is not merged, so its upstreams stay unscheduled.
        s.routes.get_mut(&flash).unwrap().disabled = true;
        let routes = build(&models, &s);
        assert_eq!(resolve(&routes, "DeepSeek-V4.1-Flash").unwrap().targets.len(), 2);
        assert_eq!(resolve(&routes, "DeepSeek-Flash").unwrap_err(), "此模型已在模型目录中停用");
        s.routes.get_mut(&flash).unwrap().disabled = false;
        // A destination that disappears leaves the source on its own.
        s.routes.get_mut(&flash).unwrap().merge_into = Some("gone".into());
        assert_eq!(resolve(&build(&models, &s), "DeepSeek-Flash").unwrap().route_id, flash);
    }
    #[test]
    fn shared_capabilities_use_intersection() {
        let mut a = model(ProviderKind::Workbuddy, "cn", "m", "Shared");
        a.efforts = vec!["low".into(), "high".into()];
        a.context_window = Some(200_000);
        let mut b = model(ProviderKind::Qoder, "intl", "q", "Shared");
        b.efforts = vec!["high".into()];
        b.supports_images = Some(false);
        b.context_window = Some(100_000);
        let routes = build(&[a, b], &Settings::default());
        let v = routes[0].view(&Scope::default());
        assert_eq!(v["efforts"], json!(["high"]));
        assert_eq!(v["supportsImages"], false);
        assert_eq!(v["supportsTools"], true);
        assert_eq!(v["contextWindow"], 100000);
        // Clients may offer every level of any account, ordered by depth.
        let (levels, default) = routes[0].reasoning_levels(&Scope::default());
        let names: Vec<&Value> = levels.as_array().unwrap().iter().map(|l| &l["effort"]).collect();
        assert_eq!(names, [&json!("low"), &json!("high")]);
        assert_eq!(default, "high");
        let mut c = model(ProviderKind::Qoder, "cn", "c", "Other");
        c.efforts = vec!["max".into(), "low".into(), "none".into()];
        c.default_effort = Some("max".into());
        let routes = build(&[c, model(ProviderKind::Qoder, "cn", "plain", "Plain")], &Settings::default());
        let (levels, default) = resolve(&routes, "Other").unwrap().reasoning_levels(&Scope::default());
        let names: Vec<&Value> = levels.as_array().unwrap().iter().map(|l| &l["effort"]).collect();
        assert_eq!(names, [&json!("none"), &json!("low"), &json!("max")]);
        assert_eq!(default, "max");
        let (levels, default) = resolve(&routes, "Plain").unwrap().reasoning_levels(&Scope::default());
        assert_eq!((levels, default), (json!([]), Value::Null));
    }
}
