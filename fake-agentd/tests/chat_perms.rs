//! "Always", allow/deny/ask modes and the decision log.

mod chat_common;

use std::time::Duration;

use chat_common::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn ask_flow_roundtrip() {
    let r = rig().await;
    let s = r.session().await;
    let w = watch(&r.d, &r.admin, &format!("session.{s}")).await;
    let req = ask(&r, s, "/ask send email to client").await;
    let id = req["id"].as_i64().unwrap_or(-1);
    assert_eq!(req["status"], "pending");
    assert_eq!(req["action"], "send");
    assert_eq!(req["source"], "policy");
    assert_eq!(req["level"], "ask");
    assert_eq!(req["target"], "email to client");
    let created = req["created_at"].as_i64().unwrap_or(0);
    assert_eq!(
        req["expires_at"].as_i64(),
        Some(created + 600),
        "10 minutes"
    );
    let card = w
        .event("permission.request", 5)
        .await
        .unwrap_or_else(|| panic!("no permission.request"));
    assert_eq!(card["request"]["id"], id);
    let msgs = r.messages(s).await;
    let cardmsg = msgs
        .iter()
        .find(|m| m["role"] == "system")
        .unwrap_or_else(|| panic!("no card: {msgs:?}"));
    assert_eq!(cardmsg["meta"]["kind"], "permission_request");
    assert_eq!(cardmsg["meta"]["request_id"], id);
    assert_eq!(
        rows(&r, "send")[0][0],
        None,
        "decision is empty before the decision"
    );
    let got = r.get(&format!("/v1/permission-requests/{id}")).await;
    assert_eq!(got["status"], "pending");
    let ok = r.decide(id, json!({"decision": "allow"})).await;
    assert_eq!(ok.status, 200, "{}", String::from_utf8_lossy(&ok.body));
    assert_eq!(ok.json()["status"], "allowed");
    assert!(ok.json()["decided_by"].is_string() && ok.json()["decided_at"].is_i64());
    let resolved = w
        .event("permission.resolved", 5)
        .await
        .unwrap_or_else(|| panic!("no permission.resolved"));
    assert_eq!(resolved["request_id"], id);
    assert_eq!(resolved["status"], "allowed");
    let log = rows(&r, "send");
    assert_eq!(log[0][0].as_deref(), Some("ask-approved"));
    assert_eq!(log[0][2].as_deref(), Some("0"), "send is irreversible");
    let after = r.messages(s).await;
    let cardmsg = after
        .iter()
        .find(|m| m["role"] == "system")
        .map(|m| m["meta"].clone())
        .unwrap_or_default();
    assert_eq!(
        cardmsg["request"]["status"], "allowed",
        "card resolved in meta"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_deny_logged_and_mode_allow_deny_paths() {
    let r = rig().await;
    let s = r.session().await;
    let req = ask(&r, s, "/ask delete file").await;
    let id = req["id"].as_i64().unwrap_or(-1);
    assert_eq!(
        r.decide(id, json!({"decision": "deny"})).await.json()["status"],
        "denied"
    );
    assert_eq!(rows(&r, "delete")[0][0].as_deref(), Some("ask-denied"));
    r.d.seed_permission("global", 0, "browse", "allow")
        .expect("rule");
    r.say(s, "/ask browse site", UUID_B).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(r.d.pending_requests().is_empty(), "allow: no card");
    assert_eq!(rows(&r, "browse")[0][0].as_deref(), Some("allow"));
    r.d.seed_permission("global", 0, "pay", "deny")
        .expect("rule");
    r.say(s, "/ask pay bill", "0192f1a0-0000-7000-8000-00000000000c")
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(r.d.pending_requests().is_empty(), "deny: no card");
    let pay = rows(&r, "pay");
    assert_eq!(pay[0][0].as_deref(), Some("deny"));
    assert_eq!(pay[0][2].as_deref(), Some("0"), "payment is irreversible");
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_always_persists_permission() {
    let r = rig().await;
    let s = r.session().await;
    let persona = r.get("/v1/personas").await[0]["id"].as_i64().unwrap_or(-1);
    r.call(
        "PATCH",
        &format!("/v1/sessions/{s}"),
        Some(json!({"persona_id": persona})),
    )
    .await;
    let req = ask(&r, s, "/ask send email").await;
    let id = req["id"].as_i64().unwrap_or(-1);
    let res = r
        .decide(
            id,
            json!({"decision": "always", "rule": {"scope": "persona", "scope_id": persona}}),
        )
        .await;
    assert_eq!(res.status, 200, "{}", String::from_utf8_lossy(&res.body));
    let perms = r
        .get(&format!("/v1/permissions?scope=persona&scope_id={persona}"))
        .await;
    let row = perms
        .as_array()
        .and_then(|a| a.iter().find(|p| p["action"] == "send"))
        .cloned()
        .unwrap_or_default();
    assert_eq!(row["mode"], "allow", "mode defaults to allow");
    assert_eq!(row["scope"], "persona");
    r.say(s, "/ask send another email", UUID_B).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        r.d.pending_requests().is_empty(),
        "the next request does not ask"
    );
    assert_eq!(
        rows(&r, "send")
            .last()
            .and_then(|x| x[0].clone())
            .as_deref(),
        Some("allow")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_always_stricter_rule_still_applies() {
    let r = rig().await;
    let s = r.session().await;
    let persona = r.get("/v1/personas").await[0]["id"].as_i64().unwrap_or(-1);
    r.call(
        "PATCH",
        &format!("/v1/sessions/{s}"),
        Some(json!({"persona_id": persona})),
    )
    .await;
    r.d.seed_permission("session", s, "send", "ask")
        .expect("session rule");
    let req = ask(&r, s, "/ask send email").await;
    let id = req["id"].as_i64().unwrap_or(-1);
    let res = r.decide(id, json!({"decision": "always", "rule": {"scope": "persona", "scope_id": persona, "mode": "allow"}})).await;
    assert_eq!(res.status, 200);
    r.say(s, "/ask send second", UUID_B).await;
    let again = pending(&r).await;
    assert_ne!(again["id"], req["id"]);
    assert_eq!(
        again["status"], "pending",
        "the session rule is not loosened, the request asks"
    );
    let sess = r
        .get(&format!("/v1/permissions?scope=session&scope_id={s}"))
        .await;
    assert_eq!(sess[0]["mode"], "ask");
}

#[tokio::test(flavor = "multi_thread")]
async fn always_validation_and_global_deny() {
    let r = rig().await;
    let s = r.session().await;
    let req = ask(&r, s, "/ask send email").await;
    let id = req["id"].as_i64().unwrap_or(-1);
    assert_eq!(
        r.decide(id, json!({"decision": "always"})).await.status,
        422,
        "rule is required"
    );
    let sess = r
        .decide(
            id,
            json!({"decision": "always", "rule": {"scope": "session", "scope_id": s}}),
        )
        .await;
    assert_eq!(sess.status, 422, "session level is not allowed");
    assert_eq!(r.decide(id, json!({"decision": "maybe"})).await.status, 422);
    r.d.seed_permission("global", 0, "send", "deny")
        .expect("rule");
    let weaken = r.decide(id, json!({"decision": "always", "rule": {"scope": "global", "scope_id": 0, "mode": "allow"}})).await;
    assert_eq!(weaken.status, 403, "a global deny cannot be loosened");
    assert_eq!(
        r.get(&format!("/v1/permission-requests/{id}")).await["status"],
        "pending",
        "the request stayed open"
    );
}
