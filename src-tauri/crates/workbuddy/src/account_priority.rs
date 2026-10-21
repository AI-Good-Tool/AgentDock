//! Ranking uses cached billing metadata only; selecting an account never queries upstream.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AccountPriority {
    RoundRobin,
    #[default]
    ExpiryFirst,
    CreditFirst,
}

pub fn credit_rank(credit: Option<&Value>, now: i64) -> (Option<i64>, Option<f64>) {
    let Some(credit) = credit.filter(|v| v["ok"] == true) else {
        return (None, None);
    };
    let Some(resources) = credit["resources"].as_array() else {
        return (
            None,
            credit["totalRemaining"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0),
        );
    };
    let mut earliest = None;
    let mut remaining = 0.0;
    for r in resources {
        let Some(amount) = r["remaining"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.0)
        else {
            continue;
        };
        let expiry = r["expireAt"].as_i64();
        if expiry.is_some_and(|at| at <= now) {
            continue;
        }
        remaining += amount;
        if let Some(at) = expiry {
            earliest = Some(earliest.map_or(at, |old: i64| old.min(at)));
        }
    }
    (earliest, remaining.is_finite().then_some(remaining))
}

pub fn compare(
    priority: AccountPriority,
    a: (Option<i64>, Option<f64>),
    b: (Option<i64>, Option<f64>),
) -> Ordering {
    match priority {
        AccountPriority::RoundRobin => Ordering::Equal,
        AccountPriority::ExpiryFirst => a.0.unwrap_or(i64::MAX).cmp(&b.0.unwrap_or(i64::MAX)),
        AccountPriority::CreditFirst => match (a.1, b.1) {
            (Some(a), Some(b)) => b.total_cmp(&a),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rank_ignores_expired_empty_and_failed_resources() {
        let c = json!({"ok":true,"totalRemaining":9999,"resources":[
            {"remaining":500,"expireAt":99},{"remaining":0,"expireAt":101},
            {"remaining":4,"expireAt":120},{"remaining":6,"expireAt":null}
        ]});
        assert_eq!(credit_rank(Some(&c), 100), (Some(120), Some(10.0)));
        assert_eq!(
            credit_rank(Some(&json!({"ok":false,"totalRemaining":99})), 100),
            (None, None)
        );
        assert_eq!(
            credit_rank(Some(&json!({"ok":true,"totalRemaining":5})), 100),
            (None, Some(5.0))
        );
        assert_eq!(
            credit_rank(Some(&json!({"ok":true,"resources":[]})), 100),
            (None, Some(0.0))
        );
    }
}
