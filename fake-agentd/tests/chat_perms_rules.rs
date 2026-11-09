//! Loosening permissions, first-wins, token scope, request expiry, stop.

mod chat_common;

use std::time::Duration;

use chat_common::*;
use fake_agentd::{Kind, Scope};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn weaken_attempt_logged() {
    let r = rig().await;
    let s = r.session().await;
    r.d.seed_permission("global", 0, "shell", "deny")
        .expect("rule");
    let res = r.put_rule("session", s, "shell", "allow").await;
    assert_eq!(res.status, 403);
    assert_eq!(code(&res), "permission_denied");
    let f = r.log_rows("SELECT kind, severity, ref_table, verdict FROM observer_findings");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0][0].as_deref(), Some("permission_loosen"));
    assert_eq!(f[0][2].as_deref(), Some("permissions"));
    assert_eq!(f[0][3].as_deref(), Some("blocked"));
    let perms = r
        .get(&format!("/v1/permissions?scope=session&scope_id={s}"))
        .await;
    assert!(
        perms.as_array().is_none_or(Vec::is_empty),
        "the rule was not applied"
    );
    assert_eq!(
        r.put_rule("session", s, "shell", "deny").await.status,
        200,
        "tightening is allowed"
    );
    let findings = r.get("/v1/observer-findings?kind=permission_loosen").await;
    assert_eq!(findings["items"].as_array().map(Vec::len), Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_second_decision_409() {
    let r = rig().await;
    let s = r.session().await;
    let id = ask(&r, s, "/ask send email").await["id"]
        .as_i64()
        .unwrap_or(-1);
    let (a, b) = tokio::join!(
        r.decide(id, json!({"decision": "allow"})),
        r.decide(id, json!({"decision": "deny"}))
    );
    let mut st = [a.status, b.status];
    st.sort_unstable();
    assert_eq!(st, [200, 409], "first accepted, second 409");
    let loser = if a.status == 409 { a } else { b };
    assert_eq!(loser.json()["error"]["code"], "conflict");
    assert_eq!(
        loser.json()["error"]["details"]["reason"],
        "already_decided"
    );
    assert_eq!(rows(&r, "send").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_decision_external_token_403() {
    let r = rig().await;
    let s = r.session().await;
    let id = ask(&r, s, "/ask send email").await["id"]
        .as_i64()
        .unwrap_or(-1);
    for (scope, kind) in [
        (Scope::Chat, Kind::External),
        (Scope::Chat, Kind::Test),
        (Scope::Read, Kind::External),
    ] {
        let t = r.token(scope, kind);
        let res = r
            .call_as(
                &t,
                "POST",
                &format!("/v1/permission-requests/{id}/decision"),
                Some(json!({"decision": "allow"})),
            )
            .await;
        assert_eq!(res.status, 403, "{scope:?}/{kind:?}");
    }
    assert_eq!(
        r.get(&format!("/v1/permission-requests/{id}")).await["status"],
        "pending"
    );
    assert_eq!(rows(&r, "send")[0][0], None, "no decision in the log");
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_token_cannot_decide_or_change_permissions() {
    let r = rig().await;
    let s = r.session().await;
    let id = ask(&r, s, "/ask send email").await["id"]
        .as_i64()
        .unwrap_or(-1);
    let chat = r.token(Scope::Chat, Kind::Test);
    let d = r
        .call_as(
            &chat,
            "POST",
            &format!("/v1/permission-requests/{id}/decision"),
            Some(json!({"decision": "allow"})),
        )
        .await;
    assert_eq!((d.status, code(&d).as_str()), (403, "insufficient_scope"));
    let p = r
        .call_as(
            &chat,
            "PUT",
            "/v1/permissions/global/0/send",
            Some(json!({"mode": "allow"})),
        )
        .await;
    assert_eq!(p.status, 403);
    assert_eq!(rows(&r, "send")[0][0], None);
    assert!(
        r.get("/v1/permissions?scope=global")
            .await
            .as_array()
            .is_none_or(|a| a.iter().all(|x| x["action"] != "send"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_expires_after_10_min() {
    let r = rig().await;
    let s = r.session().await;
    let w = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    let id = ask(&r, s, "/ask delete file").await["id"]
        .as_i64()
        .unwrap_or(-1);
    r.d.advance(Duration::from_secs(599));
    assert_eq!(
        r.get(&format!("/v1/permission-requests/{id}")).await["status"],
        "pending"
    );
    r.d.advance(Duration::from_secs(2));
    assert_eq!(
        r.get(&format!("/v1/permission-requests/{id}")).await["status"],
        "expired"
    );
    let res = w
        .event("permission.resolved", 5)
        .await
        .unwrap_or_else(|| panic!("no permission.resolved"));
    assert_eq!(res["status"], "expired");
    let late = r.decide(id, json!({"decision": "allow"})).await;
    assert_eq!(late.status, 409, "an answer after expiry is rejected");
    let log = rows(&r, "delete");
    assert_eq!(log[0][0].as_deref(), Some("deny"));
    assert_eq!(log[0][1].as_deref(), Some("expired"));
    assert!(r.d.pending_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_cancels_open_requests() {
    let r = rig().await;
    let s = r.session().await;
    let id = ask(&r, s, "/ask delete file").await["id"]
        .as_i64()
        .unwrap_or(-1);
    assert_eq!(
        r.call("POST", &format!("/v1/sessions/{s}/stop"), None)
            .await
            .status,
        204
    );
    assert_eq!(
        r.get(&format!("/v1/permission-requests/{id}")).await["status"],
        "cancelled"
    );
    assert_eq!(r.decide(id, json!({"decision": "allow"})).await.status, 409);
}
