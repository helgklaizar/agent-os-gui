//! A6-04 (route): `POST /modules/{module_id}/token` (`admin`) is the token of an open panel: `kind = module`, `scope = chat`,
//! method list from the manifest; installed and enabled module; disabling and removal revoke the tokens.

mod common;

use common::{Env, raw};
use serde_json::{Value, json};

fn manifest(id: &str) -> Value {
    json!({"id": id, "name": "Notes", "version": "1", "entry": "index.html", "placement": "panel",
           "permissions": ["GET /sessions"], "network": [], "fs": []})
}

async fn install(e: &Env, id: &str) {
    assert_eq!(
        e.admin
            .post("/modules", json!({"manifest": manifest(id)}))
            .await
            .0,
        201
    );
}

async fn issue(e: &Env, id: &str) -> (u16, Value) {
    e.admin
        .post(&format!("/modules/{id}/token"), json!({}))
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn module_token_route_issues_limited_token() {
    let e = Env::start().await;
    install(&e, "notes").await;
    let (st, v) = issue(&e, "notes").await;
    assert_eq!(st, 201, "{v}");
    let token = v["token"].as_str().unwrap().to_string();
    assert!(v["expires_at"].as_i64().unwrap() > 0);
    // The token works only within the manifest method list.
    assert_eq!(
        raw(&e.sock, &token, "GET", "/sessions", None, b"")
            .await
            .status,
        200
    );
    assert_eq!(
        raw(&e.sock, &token, "GET", "/tokens", None, b"")
            .await
            .status,
        403,
        "outside the list and outside scope"
    );
    assert_eq!(
        raw(
            &e.sock,
            &token,
            "POST",
            "/sessions",
            Some("application/json"),
            b"{\"name\":\"x\"}"
        )
        .await
        .status,
        403
    );
    let (_, list) = e.admin.get("/tokens").await;
    let t = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "module:notes")
        .unwrap();
    assert_eq!(
        (t["scope"].as_str(), t["name"].as_str()),
        (Some("chat"), Some("module:notes"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn module_token_route_checks_state_and_scope() {
    let e = Env::start().await;
    assert_eq!(issue(&e, "nope").await.0, 404, "no such module");
    install(&e, "notes").await;
    for api in [&e.read, &e.chat] {
        assert_eq!(
            api.post("/modules/notes/token", json!({})).await.0,
            403,
            "admin only"
        );
    }
    e.admin
        .patch("/modules/notes", json!({"enabled": false}))
        .await;
    assert_eq!(
        issue(&e, "notes").await.0,
        409,
        "no token is issued for a disabled module"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn module_token_is_revoked_on_disable_and_delete() {
    let e = Env::start().await;
    install(&e, "notes").await;
    let t1 = issue(&e, "notes").await.1["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        raw(&e.sock, &t1, "GET", "/sessions", None, b"")
            .await
            .status,
        200
    );
    e.admin
        .patch("/modules/notes", json!({"enabled": false}))
        .await;
    assert_eq!(
        raw(&e.sock, &t1, "GET", "/sessions", None, b"")
            .await
            .status,
        401,
        "disabling revoked the token"
    );
    e.admin
        .patch("/modules/notes", json!({"enabled": true}))
        .await;
    let t2 = issue(&e, "notes").await.1["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        raw(&e.sock, &t2, "GET", "/sessions", None, b"")
            .await
            .status,
        200
    );
    assert_eq!(e.admin.delete("/modules/notes").await.0, 204);
    assert_eq!(
        raw(&e.sock, &t2, "GET", "/sessions", None, b"")
            .await
            .status,
        401,
        "removal revoked the token"
    );
}

fn now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

/// Panel token lifetime is exactly one day (A6-04): not two, not twelve hours.
#[tokio::test(flavor = "multi_thread")]
async fn module_token_route_lives_exactly_one_day() {
    let e = Env::start().await;
    install(&e, "notes").await;
    let before = now();
    let (st, v) = issue(&e, "notes").await;
    let after = now();
    assert_eq!(st, 201, "{v}");
    let exp = v["expires_at"].as_i64().unwrap();
    assert!(
        (before + 86_400..=after + 86_400).contains(&exp),
        "lifetime {exp}, expected {}..{} (a day from issue)",
        before + 86_400,
        after + 86_400
    );
}
