//! `fake-agentd` tree API: token permissions, hostile input, `sessions` topic state.

mod tree_common;

use fake_agentd::{Kind, Scope, TokenSpec};
use serde_json::json;
use tree_common::rawhttp::{Req, send};
use tree_common::*;

// ---------- security and hostile input ----------

#[tokio::test(flavor = "multi_thread")]
async fn tree_routes_require_scope_and_token() {
    let r = rig().await;
    let read =
        r.d.issue(TokenSpec::new(Scope::Read, Kind::Test))
            .map(|t| t.token)
            .unwrap_or_default();
    let sock = r.d.socket_path();
    let ok = send(sock, Req::new("GET", "/v1/folders").token(&read)).await;
    assert_eq!(ok.status, 200, "read can read the tree");
    let ok = send(sock, Req::new("GET", "/v1/personas").token(&read)).await;
    assert_eq!(ok.status, 200);
    for (m, p, b) in [
        ("POST", "/v1/folders", Some(json!({"name": "x"}))),
        ("POST", "/v1/sessions", Some(json!({"name": "x"}))),
        ("PATCH", "/v1/sessions/1", Some(json!({"name": "x"}))),
        ("DELETE", "/v1/sessions/1", None),
        ("DELETE", "/v1/folders/1", None),
        ("POST", "/v1/sessions/1/read", Some(json!({}))),
        ("POST", "/v1/sessions/1/threads", Some(json!({"name": "x"}))),
    ] {
        let mut q = Req::new(m, p).token(&read);
        if let Some(b) = b.clone() {
            q = q.json(b);
        }
        let denied = send(sock, q).await;
        assert_eq!(denied.status, 403, "{m} {p} with a read token");
        assert_eq!(code(&denied), "insufficient_scope");
        let mut q = Req::new(m, p);
        if let Some(b) = b {
            q = q.json(b);
        }
        assert_eq!(send(sock, q).await.status, 401, "{m} {p} without a token");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evil_and_long_names_roundtrip_literally() {
    let r = rig().await;
    for evil in [
        "<img src=x onerror=alert(1)>",
        "\" onmouseover=\"x",
        "&amp;<b>x</b>",
    ] {
        let id = r.session(evil, None).await;
        assert_eq!(r.session_json(id).await["name"], evil);
        let f = r.folder(evil, None, None).await;
        assert!(
            r.get("/v1/folders")
                .await
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x["id"] == f && x["name"] == evil))
        );
    }
    let long = "u".repeat(100_000);
    let id = r.session(&long, None).await;
    assert_eq!(
        r.session_json(id).await["name"].as_str().map(str::len),
        Some(long.len())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_bodies_rejected() {
    let r = rig().await;
    let req = Req::new("POST", "/v1/sessions").token(&r.admin);
    let mut raw = req;
    raw.body = Some(json!("a string instead of an object"));
    assert_eq!(send(r.d.socket_path(), raw).await.status, 400);
    let bad = r.post("/v1/sessions", json!({"name": 5})).await;
    assert_eq!(bad.status, 422);
    let bad = r.patch("/v1/folders/1", json!({"open": "yes"})).await;
    assert_eq!(bad.status, 422);
}

// ---------- sessions topic state ----------

#[tokio::test(flavor = "multi_thread")]
async fn sessions_topic_state_tracks_tree() {
    let r = rig().await;
    let snap = r.d.snapshot("sessions");
    assert!(
        snap["folders"].as_array().is_some_and(|a| !a.is_empty()),
        "{snap}"
    );
    assert!(snap["sessions"].as_array().is_some_and(|a| !a.is_empty()));
    assert!(
        snap["sessions"][0]["effective_persona_id"].is_i64()
            || snap["sessions"][0]["effective_persona_id"].is_null()
    );
    let id = r.session("New in topic", None).await;
    let snap = r.d.snapshot("sessions");
    assert!(
        snap["sessions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|s| s["id"] == id)),
        "creation got into the state"
    );
    r.call("DELETE", &format!("/v1/sessions/{id}"), None).await;
    let snap = r.d.snapshot("sessions");
    assert!(
        snap["sessions"]
            .as_array()
            .is_some_and(|a| a.iter().all(|s| s["id"] != id)),
        "deletion left the state"
    );
    assert!(
        snap.get("op").is_none(),
        "delta fields do not get into the state: {snap}"
    );
}
