//! A4-04: permission request decision: first-wins, TTL, session deletion, who may decide.

mod common;

use std::time::Duration;

use common::{Env, ask_delete, db_decision, decide, fields, session};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn decision_allow_executes_and_resolves() {
    let mut e = Env::start().await;
    let s = session(&e, None).await;
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    let req = ask_delete(&e, s).await;
    let id = req["id"].as_i64().unwrap();
    let (st, done) = decide(&e.admin, id, json!({"decision": "allow"})).await;
    assert_eq!(st, 200, "{done}");
    assert_eq!(done["status"], "allowed");
    assert!(
        done["decided_by"].as_str().unwrap().starts_with("gui:"),
        "{done}"
    );
    assert!(done["decided_at"].as_i64().unwrap() > 0);
    // To all subscribers: `permission.resolved`.
    let frames = ws.until(Duration::from_secs(5), |f| {
        f.data["kind"] == "permission.resolved"
    });
    let d = &frames.last().unwrap().data;
    assert_eq!(
        (
            d["request_id"].as_i64(),
            d["status"].as_str(),
            d["result"].as_str()
        ),
        (Some(id), Some("allowed"), Some("allowed")),
        "{d}"
    );
    assert_eq!(d["decided_by"], done["decided_by"]);
    // The card is updated.
    let msgs = e.messages(s).await;
    let card = msgs.iter().find(|m| m["role"] == "system").unwrap();
    assert_eq!(
        (
            card["meta"]["status"].as_str(),
            card["meta"]["request"]["status"].as_str()
        ),
        (Some("allowed"), Some("allowed"))
    );
    // A mutation replayed with the same token runs once; the next one asks again.
    let (st, _) = e.agent.delete(&format!("/sessions/{s}")).await;
    assert_eq!(st, 204);
    let s2 = session(&e, None).await;
    let (st, _) = e.agent.delete(&format!("/sessions/{s2}")).await;
    assert_eq!(st, 202, "the permission applied to the first target only");
    // Journal (after replays: inspecting the DB restarts the core, and one-shot permissions live in memory).
    assert_eq!(
        db_decision(&mut e, id).await,
        (Some("ask-approved".into()), Some("allowed".into()))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn decision_deny_keeps_the_action_closed() {
    let mut e = Env::start().await;
    let s = session(&e, None).await;
    let req = ask_delete(&e, s).await;
    let id = req["id"].as_i64().unwrap();
    let (st, done) = decide(&e.admin, id, json!({"decision": "deny"})).await;
    assert_eq!(
        (st, done["status"].as_str()),
        (200, Some("denied")),
        "{done}"
    );
    assert_eq!(
        db_decision(&mut e, id).await,
        (Some("ask-denied".into()), Some("denied".into()))
    );
    let (st, again) = e.agent.delete(&format!("/sessions/{s}")).await;
    assert_eq!(st, 202, "a denial creates no permission: {again}");
    assert_ne!(again["id"], req["id"]);
    let (st, _) = e.read.get(&format!("/sessions/{s}")).await;
    assert_eq!(st, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn decision_second_409() {
    let e = Env::start().await;
    let s = session(&e, None).await;
    let id = ask_delete(&e, s).await["id"].as_i64().unwrap();
    let (a, b) = tokio::join!(
        decide(&e.admin, id, json!({"decision": "allow"})),
        decide(&e.admin2, id, json!({"decision": "deny"})),
    );
    let mut codes = [a.0, b.0];
    codes.sort_unstable();
    assert_eq!(codes, [200, 409], "{a:?} {b:?}");
    let loser = if a.0 == 409 { &a.1 } else { &b.1 };
    assert_eq!(loser["error"]["code"], "conflict");
    assert_eq!(loser["error"]["details"]["reason"], "already_decided");
    let winner = if a.0 == 200 { &a.1 } else { &b.1 };
    let (_, cur) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(
        cur["status"], winner["status"],
        "status follows the first decision"
    );
    // Later it is 409 too, the status does not change.
    let (st, v) = decide(&e.admin, id, json!({"decision": "deny"})).await;
    assert_eq!(
        (st, v["error"]["details"]["reason"].as_str()),
        (409, Some("already_decided"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn decision_external_or_chat_403() {
    let mut e = Env::start().await;
    let s = session(&e, None).await;
    let id = ask_delete(&e, s).await["id"].as_i64().unwrap();
    for (who, api) in [
        ("chat", &e.chat),
        ("read", &e.read),
        ("agent", &e.agent),
        ("module", &e.module),
    ] {
        for d in ["allow", "deny"] {
            let (st, v) = decide(api, id, json!({"decision": d})).await;
            assert_eq!(st, 403, "{who} {d}: {v}");
        }
    }
    let (_, cur) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(cur["status"], "pending", "the request stayed open");
    assert_eq!(
        db_decision(&mut e, id).await,
        (None, None),
        "journal without a decision"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn decision_validation_and_unknown() {
    let e = Env::start().await;
    let s = session(&e, None).await;
    let id = ask_delete(&e, s).await["id"].as_i64().unwrap();
    for (body, field) in [
        (json!({"decision": "maybe"}), "decision"),
        (json!({}), "decision"),
        (json!({"decision": "allow", "x": 1}), "x"),
        (
            json!({"decision": "allow", "rule": {"scope": "global"}}),
            "rule",
        ),
        (json!({"decision": "always"}), "rule"),
        (
            json!({"decision": "always", "rule": {"scope": "session", "scope_id": s}}),
            "rule.scope",
        ),
        (
            json!({"decision": "always", "rule": {"scope": "global", "scope_id": 5}}),
            "rule.scope_id",
        ),
        (
            json!({"decision": "always", "rule": {"scope": "persona"}}),
            "rule.scope_id",
        ),
        (
            json!({"decision": "always", "rule": {"scope": "global", "mode": "sure"}}),
            "rule.mode",
        ),
        (
            json!({"decision": "always", "rule": {"scope": "global", "zzz": 1}}),
            "rule.zzz",
        ),
        (
            json!({"decision": "always", "rule": {"scope": "persona", "scope_id": 9999}}),
            "rule.scope_id",
        ),
    ] {
        let (st, v) = decide(&e.admin, id, body.clone()).await;
        assert_eq!(
            (st, fields(&v)),
            (422, vec![field.to_string()]),
            "{body}: {v}"
        );
    }
    let (st, _) = decide(&e.admin, 987_654, json!({"decision": "allow"})).await;
    assert_eq!(st, 404);
    let (st, _) = e
        .admin
        .post(
            "/permission-requests/abc/decision",
            json!({"decision": "allow"}),
        )
        .await;
    assert_eq!(st, 400);
    let (_, cur) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(cur["status"], "pending", "nothing invalid was applied");
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_expires_after_10_min() {
    let mut e = Env::start_clock().await;
    let s = session(&e, None).await;
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    let req = ask_delete(&e, s).await;
    let id = req["id"].as_i64().unwrap();
    e.advance(599).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let (_, cur) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(cur["status"], "pending", "open before expiry");
    e.advance(2).await;
    let frames = ws.until(Duration::from_secs(5), |f| {
        f.data["kind"] == "permission.resolved"
    });
    let d = &frames.last().unwrap().data;
    assert_eq!(
        (
            d["status"].as_str(),
            d["result"].as_str(),
            d["decided_by"].as_str()
        ),
        (Some("expired"), Some("expired"), Some("timeout")),
        "{d}"
    );
    let (_, cur) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(cur["status"], "expired");
    // A reply after expiry is rejected, the action is not executed.
    let (st, v) = decide(&e.admin, id, json!({"decision": "allow"})).await;
    assert_eq!(
        (st, v["error"]["details"]["status"].as_str()),
        (409, Some("expired")),
        "{v}"
    );
    assert_eq!(e.read.get(&format!("/sessions/{s}")).await.0, 200);
    assert_eq!(
        db_decision(&mut e, id).await,
        (Some("deny".into()), Some("expired".into()))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_ttl_setting_applied() {
    let e = Env::start().await;
    let (_, page) = e.admin.get("/settings/permissions").await;
    let rev = page["rev"].as_i64().unwrap();
    let (st, v) = e
        .admin
        .patch_if_match("/settings/permissions", json!({"ask_ttl_min": 0}), rev)
        .await;
    assert_eq!(
        (st, fields(&v)),
        (422, vec!["ask_ttl_min".to_string()]),
        "{v}"
    );
    let (st, v) = e
        .admin
        .patch_if_match("/settings/permissions", json!({"ask_ttl_min": 1}), rev)
        .await;
    assert_eq!(st, 200, "{v}");
    let s = session(&e, None).await;
    let req = ask_delete(&e, s).await;
    assert_eq!(
        req["expires_at"].as_i64().unwrap() - req["created_at"].as_i64().unwrap(),
        60
    );
}
