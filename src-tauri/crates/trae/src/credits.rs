//! Official entitlement packs. CN credits and international USD balances are
//! kept in separate units and never added together.
use agentdock_core::provider::CreditRank;
use serde_json::{json, Value};

/// CN credit packs sorted by expiry (unknown last); hidden and non-credit packs excluded.
pub fn credit_packs(account: &Value, now_ms: i64) -> Vec<Value> {
    if account["variant"] != "cn" {
        return vec![];
    }
    let mut rows: Vec<Value> = account["snapshot"]["usage"]["user_entitlement_pack_list"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let base = &p["entitlement_base_info"];
            let total = base["quota"]["credits_limit"].as_f64()?;
            if p["is_hide"] == true || !total.is_finite() || total <= 0.0 {
                return None;
            }
            let used = p["usage"]["credits_amount"].as_f64().unwrap_or(0.0);
            if !used.is_finite() || used < 0.0 {
                return None;
            }
            let seconds = p["expire_time"]
                .as_f64()
                .filter(|s| *s > 0.0)
                .or_else(|| base["end_time"].as_f64())
                .filter(|s| s.is_finite() && *s > 0.0);
            let expires_ms = seconds.map(|s| (s * 1000.0) as i64);
            let remaining = (total - used).max(0.0);
            let remaining = (remaining * 1e6).round() / 1e6;
            Some(json!({
                "name": p["display_desc"].as_str().filter(|s| !s.is_empty())
                    .or_else(|| p["group_name"].as_str().filter(|s| !s.is_empty()))
                    .or_else(|| base["product_extra"]["package_extra"]["package_name"].as_str())
                    .unwrap_or("积分包"),
                "total": total,
                "used": used,
                "remaining": remaining,
                "expiresAt": expires_ms.map(|ms| chrono::DateTime::from_timestamp_millis(ms)
                    .unwrap_or_default()
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
                "expiresAtMs": expires_ms,
                "active": remaining > 0.0 && expires_ms.is_none_or(|ms| ms > now_ms),
            }))
        })
        .collect();
    rows.sort_by_key(|p| p["expiresAtMs"].as_i64().unwrap_or(i64::MAX));
    rows
}

/// Displayed balance: CN credits from usage_summary, international USD.
pub fn balance(account: &Value) -> Value {
    let u = &account["snapshot"]["usage"];
    if u["usage_summary"].is_object() {
        let total = u["usage_summary"]["total_amount"].as_f64().unwrap_or(0.0);
        let used = u["usage_summary"]["consumed_amount"].as_f64().unwrap_or(0.0);
        return json!({"remaining": total - used, "total": total, "used": used, "unit": "credits"});
    }
    if u["is_dollar_usage_billing"] == true {
        let packs = u["user_entitlement_pack_list"].as_array().cloned().unwrap_or_default();
        let total: f64 = packs
            .iter()
            .filter_map(|p| p["entitlement_base_info"]["quota"]["basic_usage_limit"].as_f64())
            .sum();
        let used: f64 = packs
            .iter()
            .filter_map(|p| p["usage"]["basic_usage_amount"].as_f64())
            .sum();
        return json!({"remaining": total - used, "total": total, "used": used, "unit": "usd"});
    }
    json!({"remaining": null, "total": null, "used": null,
        "unit": if account["variant"] == "cn" { "credits" } else { "usd" }})
}

/// Ranking: comparable credits only; USD balances are reported as unknown.
pub fn rank(account: &Value, now_ms: i64) -> CreditRank {
    let packs = credit_packs(account, now_ms);
    let b = balance(account);
    let unit = b["unit"].as_str().unwrap_or("credits").to_owned();
    let active: Vec<_> = packs.iter().filter(|p| p["active"] == true).collect();
    let remaining = if unit != "credits" {
        None
    } else if !active.is_empty() {
        Some(active.iter().filter_map(|p| p["remaining"].as_f64()).sum())
    } else {
        b["remaining"].as_f64()
    };
    CreditRank {
        remaining,
        earliest_expiry: active.iter().filter_map(|p| p["expiresAtMs"].as_i64()).min(),
        unit,
        updated_at: account["snapshot"]["time"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp_millis()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pack(total: Option<f64>, used: f64, expiry: i64, extra: Value) -> Value {
        let mut p = json!({"entitlement_base_info":{"quota":{"credits_limit":total},"end_time":expiry},"usage":{"credits_amount":used}});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    }
    #[test]
    fn packs_sorted_hidden_excluded_and_active_counted() {
        let a = json!({"variant":"cn","snapshot":{"usage":{"user_entitlement_pack_list":[
            pack(Some(100.0),25.35,2000,json!({"display_desc":"签到奖励"})),
            pack(Some(500.0),0.0,1500,json!({})),
            pack(Some(100.0),100.0,1600,json!({})),
            pack(Some(10.0),0.0,900,json!({})),
            pack(None,0.0,2000,json!({})),
            pack(Some(30.0),0.0,2000,json!({"is_hide":true})),
        ]}}});
        let rows = credit_packs(&a, 1000 * 1000);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows.iter().filter(|p| p["active"] == true).count(), 2);
        assert_eq!(
            rows.iter().map(|p| p["total"].as_f64().unwrap()).collect::<Vec<_>>(),
            vec![10.0, 500.0, 100.0, 100.0]
        );
        assert_eq!(rows[3]["remaining"], 74.65);
        assert_eq!(rows[3]["name"], "签到奖励");
        let mut intl = a.clone();
        intl["variant"] = json!("intl");
        assert!(credit_packs(&intl, 0).is_empty());
        let r = rank(&a, 1000 * 1000);
        assert_eq!(r.remaining, Some(574.65));
        assert_eq!(r.earliest_expiry, Some(1_500_000));
    }
    #[test]
    fn zero_expire_time_falls_back_and_usd_is_not_ranked() {
        let a = json!({"variant":"cn","snapshot":{"usage":{"user_entitlement_pack_list":[{"expire_time":0,"entitlement_base_info":{"quota":{"credits_limit":100},"end_time":2000}}]}}});
        let rows = credit_packs(&a, 1000);
        assert_eq!(rows[0]["remaining"], 100.0);
        assert_eq!(rows[0]["expiresAt"], "1970-01-01T00:33:20.000Z");
        let usd = json!({"variant":"intl","snapshot":{"usage":{"is_dollar_usage_billing":true,"user_entitlement_pack_list":[{"entitlement_base_info":{"quota":{"basic_usage_limit":20}},"usage":{"basic_usage_amount":5}}]}}});
        assert_eq!(balance(&usd)["remaining"], 15.0);
        assert_eq!(rank(&usd, 0).remaining, None);
        assert_eq!(rank(&usd, 0).unit, "usd");
    }
}
