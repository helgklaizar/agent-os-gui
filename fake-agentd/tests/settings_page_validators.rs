//! Settings page field checks and indicator order (`PATCH /v1/settings/{page}`, `/v1/settings/indicators`).

mod s7_common;

use s7_common::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn page_validators() {
    let r = rig().await;
    let cases = [
        (
            "personas",
            json!({"balance": {"rotate": true}}),
            "balance.rotate",
        ),
        (
            "personas",
            json!({"balance": {"fallback": "nonsense"}}),
            "balance.fallback",
        ),
        (
            "personas",
            json!({"balance": {"warn_pct": 0}}),
            "balance.warn_pct",
        ),
        (
            "personas",
            json!({"balance": {"max_parallel": 17}}),
            "balance.max_parallel",
        ),
        (
            "personas",
            json!({"balance": {"distribution": "x"}}),
            "balance.distribution",
        ),
        (
            "personas",
            json!({"balance": {"quiet_from": "25:00"}}),
            "balance.quiet_from",
        ),
        ("personas", json!({"roles": {"chat": "a"}}), "roles.chat"),
        ("personas", json!({"roles": {"judge": 1}}), "roles.judge"),
        ("account", json!({"language": "de"}), "language"),
        ("account", json!({"name": "x".repeat(81)}), "name"),
        (
            "account",
            json!({"security": {"observer": "yes"}}),
            "security.observer",
        ),
        (
            "schedule",
            json!({"summary": {"time": "9am"}}),
            "summary.time",
        ),
        (
            "schedule",
            json!({"backup": {"enabled": 1}}),
            "backup.enabled",
        ),
        ("permissions", json!({"ask_ttl_min": 0}), "ask_ttl_min"),
        ("permissions", json!({"x": 1}), "x"),
        ("tunnels", json!({"api_external": "yes"}), "api_external"),
        ("memory", json!({"source": "cloud"}), "source"),
        (
            "models",
            json!({"accounts": {"3": {"api_key": "x"}}}),
            "accounts.3.api_key",
        ),
        (
            "models",
            json!({"accounts": {"3": {"base_url": "https://u:p@h/"}}}),
            "accounts.3.base_url",
        ),
        ("models", json!({"context_budget": 255}), "context_budget"),
        (
            "models",
            json!({"accounts": {"3": {"base_url": "http://h/a b"}}}),
            "accounts.3.base_url",
        ),
        (
            "models",
            json!({"accounts": {"3": {"base_url": "http://h/a\u{7}b"}}}),
            "accounts.3.base_url",
        ),
        ("models", json!({"accounts": {"03": {}}}), "accounts.03"),
        ("models", json!({"accounts": {"+3": {}}}), "accounts.+3"),
        ("models", json!({"accounts": {"0": {}}}), "accounts.0"),
        ("models", json!({"accounts": {"-1": {}}}), "accounts.-1"),
        ("models", json!({"accounts": {"x": {}}}), "accounts.x"),
        ("models", json!({"roles": {}}), "roles"),
        (
            "connections",
            json!({"mcp": [{"name": "a__b", "transport": "http", "url": "http://x.y/z"}]}),
            "mcp[0].name",
        ),
        (
            "connections",
            json!({"routes": [{"contact": "Nobody", "session_id": 1}]}),
            "routes[0].contact",
        ),
    ];
    for (page, body, field) in cases {
        let w = r.patch_page(page, body.clone()).await;
        assert_eq!(w.status, 422, "{page} {body}: {}", text(&w));
        assert!(
            fields(&w).contains(&field.to_string()),
            "{page}: {field} is not in {:?}",
            fields(&w)
        );
    }
    let good = [
        (
            "personas",
            json!({"balance": {"fallback": "wait", "warn_pct": 80, 
            "distribution": "priority", "max_parallel": 4, "unified_feed": true, "shared_memory": false,
            "quiet_from": "22:00", "quiet_to": "07:30"}, "roles": {"chat": 1}}),
        ),
        (
            "account",
            json!({"name": "A", "language": "ru", "security": {"observer": true, "telemetry": false, "injection_guard": true}}),
        ),
        (
            "schedule",
            json!({"summary": {"enabled": true, "time": "08:00"}, "backup": {"enabled": false, "time": "03:00", "recipient": {"primary": "k1", "recovery": "k2"}}}),
        ),
        ("permissions", json!({"ask_ttl_min": 15})),
        ("tunnels", json!({"api_external": true})),
        ("memory", json!({"source": "both"})),
        (
            "models",
            json!({"accounts": {"3": {"model": "m", "secret_ref": "account.3", "base_url": "http://127.0.0.1:11434/v1"}}, "context_budget": 1000}),
        ),
        (
            "connections",
            json!({"contacts": [{"name": "Bob", "handles": {"tg": "@v"}}], "routes": [{"contact": "Bob", "session_id": 3}],
            "mcp": [{"name": "files", "transport": "stdio", "command": "mcp-files"}]}),
        ),
    ];
    for (page, body) in good {
        let w = r.patch_page(page, body.clone()).await;
        assert_eq!(w.status, 200, "{page}: {}", text(&w));
        let back = r.get(&format!("/v1/settings/{page}")).await;
        for (k, v) in body.as_object().into_iter().flatten() {
            assert_eq!(&back[k], v, "{page}.{k}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn indicators_page_order_and_enable_persist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let r = rig_db(&db).await;
    let page = r.get("/v1/settings/indicators").await;
    let items = page["items"].as_array().cloned().unwrap_or_default();
    assert!(items.len() >= 3, "{page}");
    for it in &items {
        assert!(it["id"].is_string() && it["enabled"].is_boolean() && it["position"].is_number());
    }
    let mut swapped = items.clone();
    swapped.reverse();
    for (i, it) in swapped.iter_mut().enumerate() {
        it["position"] = json!(i);
    }
    swapped[0]["enabled"] = json!(false);
    let first = swapped[0]["id"].clone();
    let rev = page["rev"].as_i64().unwrap_or(-1);
    let w = r
        .call_if(
            "PATCH",
            "/v1/settings/indicators",
            rev,
            json!({"items": swapped}),
        )
        .await;
    assert_eq!(w.status, 200, "{}", text(&w));
    assert_eq!(w.json()["rev"], rev + 1);
    let stale = r
        .call_if(
            "PATCH",
            "/v1/settings/indicators",
            rev,
            json!({"items": swapped}),
        )
        .await;
    assert_eq!(stale.status, 412);
    let bad = r
        .call_if(
            "PATCH",
            "/v1/settings/indicators",
            rev + 1,
            json!({"items": [{"id": "", "enabled": 1, "position": -1}]}),
        )
        .await;
    assert_eq!(bad.status, 422);
    let unknown = r
        .call_if(
            "PATCH",
            "/v1/settings/indicators",
            rev + 1,
            json!({"items": [{"id": "no.such", "enabled": true, "position": 0}]}),
        )
        .await;
    assert_eq!(unknown.status, 422, "unknown indicator");
    drop(r);
    let r = rig_db(&db).await;
    let page = r.get("/v1/settings/indicators").await;
    let items = page["items"].as_array().cloned().unwrap_or_default();
    assert_eq!(items[0]["id"], swapped[0]["id"], "order after a restart");
    let hit = items.iter().find(|i| i["id"] == first);
    assert_eq!(hit.map(|i| i["enabled"].clone()), Some(json!(false)));
}
