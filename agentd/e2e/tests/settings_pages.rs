//! A6-12: settings page schemas (fields from S7: `settings_specs.rs`) and validation on `PATCH`:
//! valid fields are saved, invalid and unknown ones give `422` at the field path, the change is published as `{page, rev}`.

mod common;

use std::time::Duration;

use common::{Env, fields};
use serde_json::{Value, json};

async fn patch(e: &Env, page: &str, body: Value) -> (u16, Value) {
    let (_, cur) = e.admin.get(&format!("/settings/{page}")).await;
    e.admin
        .patch_if_match(
            &format!("/settings/{page}"),
            body,
            cur["rev"].as_i64().unwrap_or(0),
        )
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_page_accepts_fields_the_gui_saves() {
    let e = Env::start().await;
    for (page, body) in [
        (
            "account",
            json!({"name": "Owner", "language": "ru", "lat": 32.8, "lon": 35.0,
                   "security": {"observer": true, "telemetry": false, "injection_guard": true}}),
        ),
        (
            "personas",
            json!({"balance": {"fallback": "other", "warn_pct": 80, "priority_persona_id": 1,
                               "distribution": "priority", "max_parallel": 4, "unified_feed": true,
                               "shared_memory": false, "quiet_from": "22:00", "quiet_to": "07:00"},
                   "roles": {"chat": 1, "observer": 2, "code": 3, "feed": 4}}),
        ),
        (
            "schedule",
            json!({"summary": {"enabled": true, "time": "08:30"},
                   "backup": {"enabled": true, "time": "03:00",
                              "recipient": {"primary": "age1x", "recovery": "age1y"}}}),
        ),
        ("permissions", json!({"ask_ttl_min": 15})),
        ("tunnels", json!({"api_external": true})),
        ("memory", json!({"source": "both"})),
        ("models", json!({"context_budget": 8000})),
    ] {
        let (st, v) = patch(&e, page, body.clone()).await;
        assert_eq!(st, 200, "{page}: {v}");
        let (_, got) = e.admin.get(&format!("/settings/{page}")).await;
        for (k, want) in body.as_object().unwrap() {
            assert_eq!(&got[k], want, "{page}.{k} saved: {got}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_page_schema_422_by_field() {
    let e = Env::start().await;
    for (page, body, field) in [
        ("account", json!({"language": "xx"}), "language"),
        ("account", json!({"name": 5}), "name"),
        (
            "account",
            json!({"security": {"observer": "yes"}}),
            "security.observer",
        ),
        (
            "personas",
            json!({"balance": {"fallback": "never"}}),
            "balance.fallback",
        ),
        (
            "personas",
            json!({"balance": {"warn_pct": 0}}),
            "balance.warn_pct",
        ),
        (
            "personas",
            json!({"balance": {"warn_pct": 101}}),
            "balance.warn_pct",
        ),
        (
            "personas",
            json!({"balance": {"max_parallel": 17}}),
            "balance.max_parallel",
        ),
        (
            "personas",
            json!({"balance": {"quiet_from": "25:00"}}),
            "balance.quiet_from",
        ),
        (
            "personas",
            json!({"balance": {"quiet_to": "24:00"}}),
            "balance.quiet_to",
        ),
        (
            "personas",
            json!({"balance": {"quiet_to": "07:60"}}),
            "balance.quiet_to",
        ),
        ("personas", json!({"roles": {"chat": "x"}}), "roles.chat"),
        (
            "schedule",
            json!({"summary": {"time": "8:30"}}),
            "summary.time",
        ),
        (
            "schedule",
            json!({"backup": {"enabled": "on"}}),
            "backup.enabled",
        ),
        ("schedule", json!({"tz": "Europe/Paris"}), "tz"),
        ("permissions", json!({"ask_ttl_min": 0}), "ask_ttl_min"),
        ("tunnels", json!({"api_external": "1"}), "api_external"),
        ("memory", json!({"source": "cloud"}), "source"),
    ] {
        let (st, v) = patch(&e, page, body.clone()).await;
        assert_eq!(st, 422, "{page} {body}: {v}");
        assert!(
            fields(&v).contains(&field.to_string()),
            "{page} {field}: {v}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_page_rejects_unknown_fields() {
    let e = Env::start().await;
    for (page, body, field) in [
        ("account", json!({"whatever": 1}), "whatever"),
        (
            "personas",
            json!({"balance": {"whatever": 1}}),
            "balance.whatever",
        ),
        (
            "personas",
            json!({"roles": {"whatever": 1}}),
            "roles.whatever",
        ),
        (
            "schedule",
            json!({"backup": {"whatever": 1}}),
            "backup.whatever",
        ),
        (
            "personas",
            json!({"balance": {"maxParallel": 2}}),
            "balance.maxParallel",
        ),
        ("tunnels", json!({"whatever": 1}), "whatever"),
        ("memory", json!({"whatever": 1}), "whatever"),
        ("modules", json!({"whatever": 1}), "whatever"),
    ] {
        let (st, v) = patch(&e, page, body.clone()).await;
        assert_eq!(st, 422, "{page} {body}: {v}");
        assert!(
            fields(&v).contains(&field.to_string()),
            "{page} {field}: {v}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_page_failed_patch_changes_nothing() {
    let e = Env::start().await;
    let (st, _) = patch(&e, "schedule", json!({"summary": {"time": "08:30"}})).await;
    assert_eq!(st, 200);
    let (_, before) = e.admin.get("/settings/schedule").await;
    let (st, _) = patch(
        &e,
        "schedule",
        json!({"summary": {"time": "99:99"}, "backup": {"enabled": true}}),
    )
    .await;
    assert_eq!(st, 422);
    let (_, after) = e.admin.get("/settings/schedule").await;
    assert_eq!(
        before, after,
        "a denial changes neither the document nor rev"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_page_change_publishes_page_and_rev() {
    let e = Env::start().await;
    let mut ws = e.ws();
    ws.subscribe("settings");
    let (st, saved) = patch(&e, "schedule", json!({"summary": {"enabled": true}})).await;
    assert_eq!(st, 200, "{saved}");
    let frames = ws.drain(Duration::from_millis(500));
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(frames[0].data["page"], "schedule");
    assert_eq!(frames[0].data["rev"], saved["rev"]);
    let (st, _) = patch(&e, "schedule", json!({"summary": {"time": "99:99"}})).await;
    assert_eq!(st, 422);
    assert!(
        ws.drain(Duration::from_millis(300)).is_empty(),
        "a denial is not published"
    );
}
