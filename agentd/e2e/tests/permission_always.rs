//! A4-04: "Always" is a persona or global rule, `session` is not allowed, stricter levels are not weakened.

mod common;

use common::{Env, ask_delete, decide, fields, session};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn always_global_rule() {
    let e = Env::start().await;
    let s = session(&e, None).await;
    let id = ask_delete(&e, s).await["id"].as_i64().unwrap();
    let (st, v) = decide(
        &e.admin,
        id,
        json!({"decision": "always", "rule": {"scope": "global", "scope_id": 0}}),
    )
    .await;
    assert_eq!((st, v["status"].as_str()), (200, Some("allowed")), "{v}");
    let (_, rows) = e.read.get("/permissions?scope=global").await;
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .any(|p| p["action"] == "delete" && p["mode"] == "allow"),
        "{rows}"
    );
    // The next request does not ask.
    let s2 = session(&e, None).await;
    let (st, _) = e.agent.delete(&format!("/sessions/{s2}")).await;
    assert_eq!(st, 204);
}

#[tokio::test(flavor = "multi_thread")]
async fn always_creates_persona_rule() {
    let e = Env::start().await;
    let p = e.admin.make("/personas", common::persona("P")).await["id"]
        .as_i64()
        .unwrap();
    // Globally allowed, the persona asks: ask at persona level.
    assert_eq!(
        e.admin
            .put("/permissions/global/0/delete", json!({"mode": "allow"}))
            .await
            .0,
        200
    );
    assert_eq!(
        e.admin
            .put(
                &format!("/permissions/persona/{p}/delete"),
                json!({"mode": "ask"})
            )
            .await
            .0,
        200
    );
    let s = session(&e, Some(p)).await;
    let id = ask_delete(&e, s).await["id"].as_i64().unwrap();
    let body = json!({"decision": "always", "rule": {"scope": "persona", "scope_id": p}});
    let (st, v) = decide(&e.admin, id, body).await;
    assert_eq!((st, v["status"].as_str()), (200, Some("allowed")), "{v}");
    let (_, rows) = e
        .read
        .get(&format!("/permissions?scope=persona&scope_id={p}"))
        .await;
    assert_eq!(
        rows,
        json!([{"scope": "persona", "scope_id": p, "action": "delete", "mode": "allow"}])
    );
    let (st, _) = e.agent.delete(&format!("/sessions/{s}")).await;
    assert_eq!(st, 204, "the next request does not ask");
}

#[tokio::test(flavor = "multi_thread")]
async fn always_session_scope_422() {
    let e = Env::start().await;
    let s = session(&e, None).await;
    let id = ask_delete(&e, s).await["id"].as_i64().unwrap();
    let (st, v) = decide(
        &e.admin,
        id,
        json!({"decision": "always", "rule": {"scope": "session", "scope_id": s}}),
    )
    .await;
    assert_eq!(
        (st, fields(&v)),
        (422, vec!["rule.scope".to_string()]),
        "{v}"
    );
    let (_, rows) = e
        .read
        .get(&format!("/permissions?scope=session&scope_id={s}"))
        .await;
    assert_eq!(rows, json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn always_blocked_by_stricter_rule() {
    let e = Env::start().await;
    let p = e.admin.make("/personas", common::persona("P")).await["id"]
        .as_i64()
        .unwrap();
    let s = session(&e, Some(p)).await;
    // Global ask (default): a persona `allow` rule would weaken it: 403, the request stays open.
    let id = ask_delete(&e, s).await["id"].as_i64().unwrap();
    let body = json!({"decision": "always", "rule": {"scope": "persona", "scope_id": p}});
    let (st, v) = decide(&e.admin, id, body.clone()).await;
    assert_eq!(
        (st, v["error"]["code"].as_str()),
        (403, Some("permission_denied")),
        "{v}"
    );
    let (_, cur) = e.read.get(&format!("/permission-requests/{id}")).await;
    assert_eq!(cur["status"], "pending");
    let (_, rows) = e
        .read
        .get(&format!("/permissions?scope=persona&scope_id={p}"))
        .await;
    assert_eq!(rows, json!([]), "rule not created");
    // Global allow, but the session itself asks `ask`: "Always" on the persona passes, while the session still asks.
    assert_eq!(
        e.admin
            .put("/permissions/global/0/delete", json!({"mode": "allow"}))
            .await
            .0,
        200
    );
    let (st, v) = e
        .chat
        .patch(
            &format!("/sessions/{s}"),
            json!({"cfg": {"perms": {"delete": "ask"}}}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let id2 = ask_delete(&e, s).await["id"].as_i64().unwrap();
    let (st, v) = decide(&e.admin, id2, body).await;
    assert_eq!((st, v["status"].as_str()), (200, Some("allowed")), "{v}");
    let (_, rows) = e
        .read
        .get(&format!("/permissions?scope=session&scope_id={s}"))
        .await;
    assert_eq!(
        rows,
        json!([{"scope": "session", "scope_id": s, "action": "delete", "mode": "ask"}]),
        "session rule is not weakened"
    );
    // Another session of the same persona with the same rule: "Always" does not release it, it still asks.
    let s2 = session(&e, Some(p)).await;
    let (st, _) = e
        .chat
        .patch(
            &format!("/sessions/{s2}"),
            json!({"cfg": {"perms": {"delete": "ask"}}}),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(
        e.agent.delete(&format!("/sessions/{s2}")).await.0,
        202,
        "session rule is stricter than the persona"
    );
}
