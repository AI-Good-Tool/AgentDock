//! Cross-provider account selection. Every account of every provider and
//! region that can serve the requested model joins one pool; the configured
//! strategy picks one account per attempt. The server only asks for another
//! attempt after an explicit pre-output rejection, excluding tried accounts.
use crate::settings::{Settings, Strategy};
use agentdock_core::provider::{PoolAccount, UpstreamModel};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Provider-specific execution data resolved during eligibility.
#[derive(Clone, Debug)]
pub enum Exec {
    /// WorkBuddy region model configuration.
    Workbuddy(Value),
    /// Qoder region model information.
    Qoder(Value),
    /// Trae per-account native config id (None = "auto").
    Trae(Option<String>),
}

#[derive(Default)]
pub struct Runtime {
    pub in_flight: HashMap<String, usize>,
    /// Account key -> excluded until (ms).
    pub cooldown: HashMap<String, i64>,
    /// (account key, real model id) -> excluded until (ms); the account
    /// still serves its other models.
    pub model_cooldown: HashMap<(String, String), i64>,
    /// Excluded until the user re-enables the account.
    pub auth_failed: HashSet<String>,
    /// Round-robin position per route.
    pub cursors: HashMap<String, usize>,
}

pub struct Pick {
    pub account: PoolAccount,
    pub target: UpstreamModel,
    pub exec: Exec,
    /// Accounts that offer the model but could not take this request:
    /// (provider, region, reason) -> count.
    pub skipped: BTreeMap<(String, String, &'static str), usize>,
}

pub type Eligibility<'a> = dyn Fn(&PoolAccount, &UpstreamModel) -> Result<Exec, &'static str> + 'a;

#[allow(clippy::too_many_arguments)]
pub fn select(
    accounts: &[PoolAccount],
    targets: &[&UpstreamModel],
    settings: &Settings,
    rt: &mut Runtime,
    route_id: &str,
    pinned: Option<&str>,
    now: i64,
    eligible: &Eligibility,
) -> Result<Pick, String> {
    select_excluding(accounts, targets, settings, rt, route_id, pinned, now, eligible, &HashSet::new())
}

/// [`select`] without the accounts already tried for this request.
#[allow(clippy::too_many_arguments)]
pub fn select_excluding(
    accounts: &[PoolAccount],
    targets: &[&UpstreamModel],
    settings: &Settings,
    rt: &mut Runtime,
    route_id: &str,
    pinned: Option<&str>,
    now: i64,
    eligible: &Eligibility,
    excluded: &HashSet<String>,
) -> Result<Pick, String> {
    if let Some(key) = pinned {
        if !accounts.iter().any(|a| a.key() == key) {
            return Err("指定账号不存在或已移除；未向上游发送请求".into());
        }
    }
    let n = accounts.len();
    if n == 0 {
        return Err("没有可用账号：尚未导入任何账号".into());
    }
    let cursor = rt.cursors.get(route_id).copied().unwrap_or(0) % n;
    let order: Vec<usize> = match settings.strategy {
        Strategy::Ordered => (0..n).collect(),
        _ => (0..n).map(|i| (cursor + i) % n).collect(),
    };
    let mut reasons = BTreeMap::<&'static str, usize>::new();
    let mut skipped = BTreeMap::<(String, String, &'static str), usize>::new();
    let mut candidates: Vec<(usize, &PoolAccount, &UpstreamModel, Exec)> = vec![];
    for index in order {
        let a = &accounts[index];
        if pinned.is_some_and(|k| a.key() != k) {
            continue;
        }
        let scope = settings.scope(a.provider);
        // A merged route can offer one account several targets; use the first
        // that this account may serve now.
        let mut served = targets.iter().filter(|t| t.provider() == a.provider && t.region == a.region).peekable();
        let reason = if !scope.enabled || !scope.regions.iter().any(|r| *r == a.region) {
            Err("端或地区未参与反代")
        } else if served.peek().is_some() {
            if excluded.contains(&a.key()) {
                Err("本次请求已尝试")
            } else if !a.enabled {
                Err("未启用反代")
            } else if a.blocked.is_some() {
                Err("登录凭证不可用")
            } else if a.expires_at.is_some_and(|t| t <= now) {
                Err("登录已过期")
            } else if rt.cooldown.get(&a.key()).is_some_and(|t| *t > now) {
                Err("账号冷却中")
            } else if rt.auth_failed.contains(&a.key()) {
                Err("鉴权已失败")
            } else if rt.in_flight.get(&a.key()).copied().unwrap_or(0) >= scope.concurrency {
                Err("并发已满")
            } else {
                let mut first = None;
                let found = served.find_map(|target| {
                    let result = if rt.model_cooldown.get(&(a.key(), target.id.clone())).is_some_and(|t| *t > now) {
                        Err("该模型冷却中")
                    } else {
                        eligible(a, target).map(|exec| (*target, exec))
                    };
                    result.map_err(|e| { first.get_or_insert(e); }).ok()
                });
                found.ok_or_else(|| first.unwrap())
            }
        } else {
            Err("该端或地区不提供此模型")
        };
        match reason {
            Ok((target, exec)) => candidates.push((index, a, target, exec)),
            Err(r) => {
                *reasons.entry(r).or_default() += 1;
                if !matches!(r, "端或地区未参与反代" | "该端或地区不提供此模型") {
                    *skipped.entry((a.provider.key().to_owned(), a.region.clone(), r)).or_default() += 1;
                }
            }
        }
    }
    if candidates.is_empty() {
        return Err(if reasons.is_empty() {
            "没有可用账号：指定账号不在当前范围内".into()
        } else {
            format!(
                "没有可用账号：{}（每个账号显示首个不满足条件）",
                reasons
                    .iter()
                    .map(|(r, c)| format!("{r} {c} 个"))
                    .collect::<Vec<_>>()
                    .join("；")
            )
        });
    }
    // Stable sorts keep round-robin fairness among equally ranked accounts.
    match settings.strategy {
        Strategy::CreditFirst => candidates.sort_by(|a, b| {
            match (a.1.credit.remaining, b.1.credit.remaining) {
                (Some(x), Some(y)) => y.total_cmp(&x),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            }
        }),
        Strategy::ExpiryFirst => candidates.sort_by_key(|c| c.1.credit.earliest_expiry.unwrap_or(i64::MAX)),
        Strategy::Random => {
            let i = rand_index(candidates.len());
            candidates.swap(0, i);
        }
        Strategy::RoundRobin | Strategy::Ordered => {}
    }
    let (index, account, target, exec) = candidates.swap_remove(0);
    rt.cursors.insert(route_id.into(), (index + 1) % n);
    *rt.in_flight.entry(account.key()).or_default() += 1;
    Ok(Pick {
        account: account.clone(),
        target: target.clone(),
        exec,
        skipped,
    })
}

fn rand_index(n: usize) -> usize {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    (h.finish() as usize) % n
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentdock_core::provider::{CreditRank, ProviderKind};
    fn acct(p: ProviderKind, id: &str, region: &str, credit: Option<f64>, expiry: Option<i64>) -> PoolAccount {
        PoolAccount {
            provider: p,
            id: id.into(),
            region: region.into(),
            name: id.into(),
            enabled: true,
            expires_at: None,
            credit: CreditRank { remaining: credit, earliest_expiry: expiry, unit: "credits".into(), updated_at: None },
            order: 0,
            blocked: None,
        }
    }
    fn m(p: ProviderKind, region: &str, id: &str) -> UpstreamModel {
        crate::catalog::tests::model(p, region, id, id)
    }
    fn ok(_: &PoolAccount, t: &UpstreamModel) -> Result<Exec, &'static str> {
        Ok(Exec::Qoder(serde_json::json!({"id": t.id})))
    }
    fn pick(accounts: &[PoolAccount], targets: &[&UpstreamModel], s: &Settings, rt: &mut Runtime, route: &str) -> String {
        let p = select(accounts, targets, s, rt, route, None, 0, &ok).unwrap();
        *rt.in_flight.get_mut(&p.account.key()).unwrap() -= 1;
        p.account.id
    }
    #[test]
    fn shared_models_rotate_over_every_provider_and_region_per_route() {
        use ProviderKind::*;
        let accounts = vec![
            acct(Workbuddy, "wb-cn", "cn", Some(10.0), None),
            acct(Qoder, "q-intl", "intl", Some(50.0), None),
            acct(Trae, "t-cn", "cn", None, None),
        ];
        let (a, b, c) = (m(Workbuddy, "cn", "x"), m(Qoder, "intl", "x2"), m(Trae, "cn", "x3"));
        let shared = [&a, &b, &c];
        let only = [&b];
        let s = Settings::default();
        let mut rt = Runtime::default();
        let seq: Vec<_> = (0..4).map(|_| pick(&accounts, &shared, &s, &mut rt, "shared")).collect();
        assert_eq!(seq, ["wb-cn", "q-intl", "t-cn", "wb-cn"]);
        // A model only one provider has only uses that provider's accounts.
        for _ in 0..3 {
            assert_eq!(pick(&accounts, &only, &s, &mut rt, "only"), "q-intl");
        }
        // Per-route cursors are independent.
        assert_eq!(pick(&accounts, &shared, &s, &mut rt, "shared"), "q-intl");
    }
    #[test]
    fn credit_expiry_ordered_and_exclusions() {
        use ProviderKind::*;
        let accounts = vec![
            acct(Workbuddy, "low", "cn", Some(5.0), Some(1_000)),
            acct(Qoder, "high", "cn", Some(500.0), Some(9_000)),
            acct(Trae, "usd", "intl", None, None),
        ];
        let (a, b, c) = (m(Workbuddy, "cn", "x"), m(Qoder, "cn", "x"), m(Trae, "intl", "x"));
        let t = [&a, &b, &c];
        let mut s = Settings::default();
        let mut rt = Runtime::default();
        s.strategy = Strategy::CreditFirst;
        assert_eq!(pick(&accounts, &t, &s, &mut rt, "r"), "high");
        assert_eq!(pick(&accounts, &t, &s, &mut rt, "r"), "high");
        s.strategy = Strategy::ExpiryFirst;
        assert_eq!(pick(&accounts, &t, &s, &mut rt, "r"), "low");
        s.strategy = Strategy::Ordered;
        assert_eq!(pick(&accounts, &t, &s, &mut rt, "r"), "low");
        // Concurrency, cooldown, auth failure and provider scope exclude accounts.
        s.strategy = Strategy::CreditFirst;
        rt.cooldown.insert("qoder/high".into(), 10);
        assert_eq!(pick(&accounts, &t, &s, &mut rt, "r"), "low");
        rt.cooldown.clear();
        rt.auth_failed.insert("qoder/high".into());
        s.providers.get_mut(&Workbuddy).unwrap().enabled = false;
        assert_eq!(pick(&accounts, &t, &s, &mut rt, "r"), "usd");
        s.providers.get_mut(&Trae).unwrap().regions = vec!["cn".into()];
        let err = select(&accounts, &t, &s, &mut rt, "r", None, 0, &ok).err().unwrap();
        assert!(err.contains("端或地区未参与反代 2 个") && err.contains("鉴权已失败 1 个"), "{err}");
        rt.auth_failed.clear();
        let one = select(&accounts, &t, &s, &mut rt, "r", None, 0, &ok).unwrap();
        assert_eq!(one.account.id, "high");
        s.providers.get_mut(&Qoder).unwrap().concurrency = 1;
        assert!(select(&accounts, &t, &s, &mut rt, "r", None, 0, &ok).err().unwrap().contains("并发已满"));
    }
    #[test]
    fn successful_picks_report_accounts_that_could_not_serve() {
        use ProviderKind::*;
        let accounts = vec![
            acct(Workbuddy, "wb", "cn", None, None),
            acct(Trae, "t1", "cn", None, None),
            acct(Trae, "t2", "cn", None, None),
            acct(Qoder, "q", "cn", None, None),
        ];
        let (a, b) = (m(Workbuddy, "cn", "x"), m(Trae, "cn", "x"));
        let no_tools = |acc: &PoolAccount, t: &UpstreamModel| -> Result<Exec, &'static str> {
            if acc.provider == Trae { Err("Trae 仅支持纯文本对话") } else { ok(acc, t) }
        };
        let mut rt = Runtime::default();
        let p = select(&accounts, &[&a, &b], &Settings::default(), &mut rt, "r", None, 0, &no_tools).unwrap();
        assert_eq!(p.account.id, "wb");
        // Qoder lacks the model, so it is not reported as skipped.
        assert_eq!(p.skipped.len(), 1);
        assert_eq!(p.skipped.get(&("trae".into(), "cn".into(), "Trae 仅支持纯文本对话")), Some(&2));
    }
    #[test]
    fn retries_skip_tried_accounts_and_model_cooldown_is_per_model() {
        use ProviderKind::*;
        let accounts = vec![acct(Qoder, "a", "cn", None, None), acct(Qoder, "b", "cn", None, None)];
        let (x, y) = (m(Qoder, "cn", "x"), m(Qoder, "cn", "y"));
        let s = Settings::default();
        let mut rt = Runtime::default();
        let tried = HashSet::from(["qoder/a".to_owned()]);
        for _ in 0..2 {
            let p = select_excluding(&accounts, &[&x], &s, &mut rt, "r", None, 0, &ok, &tried).unwrap();
            assert_eq!(p.account.id, "b");
            *rt.in_flight.get_mut("qoder/b").unwrap() -= 1;
        }
        let all = HashSet::from(["qoder/a".to_owned(), "qoder/b".to_owned()]);
        let err = select_excluding(&accounts, &[&x], &s, &mut rt, "r", None, 0, &ok, &all).err().unwrap();
        assert!(err.contains("本次请求已尝试 2 个"), "{err}");
        rt.model_cooldown.insert(("qoder/a".into(), "x".into()), 10);
        assert_eq!(pick(&accounts, &[&x], &s, &mut rt, "x"), "b");
        assert_eq!(pick(&accounts, &[&x], &s, &mut rt, "x"), "b");
        assert_eq!(pick(&accounts, &[&y], &s, &mut rt, "y"), "a");
    }
    #[test]
    fn pins_cannot_widen_the_model_pool() {
        use ProviderKind::*;
        let accounts = vec![acct(Workbuddy, "a", "cn", None, None), acct(Qoder, "b", "intl", None, None)];
        let a = m(Workbuddy, "cn", "only-cn");
        let s = Settings::default();
        let mut rt = Runtime::default();
        assert!(select(&accounts, &[&a], &s, &mut rt, "r", Some("qoder/b"), 0, &ok).is_err());
        assert!(select(&accounts, &[&a], &s, &mut rt, "r", Some("nope/x"), 0, &ok).err().unwrap().contains("指定账号不存在"));
        assert_eq!(select(&accounts, &[&a], &s, &mut rt, "r", Some("workbuddy/a"), 0, &ok).unwrap().account.id, "a");
        let mut expired = accounts.clone();
        expired[0].expires_at = Some(5);
        assert!(select(&expired, &[&a], &s, &mut rt, "r", None, 10, &ok).err().unwrap().contains("登录已过期"));
        let deny = |_: &PoolAccount, _: &UpstreamModel| -> Result<Exec, &'static str> { Err("模型限额") };
        assert!(select(&accounts, &[&a], &s, &mut rt, "r", None, 0, &deny).err().unwrap().contains("模型限额"));
    }
}
