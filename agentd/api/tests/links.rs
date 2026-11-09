//! Session bookmarks and quick links over HTTP (api.md 5.8): `ETag`, `If-Match`, body validation, scope, cleanup on delete.

mod common;

use std::sync::Arc;

use agentd_core::{Notifier, SystemClock};
use common::{
    TOKEN_CHAT, TOKEN_CHAT_ONLY, TOKEN_READ, config, db_services, start_with, unix_body, unix_req,
};
use serde_json::{Value, json};

struct Quiet;

impl Notifier for Quiet {
    fn publish(&self, _topic: &str, _data: Value) {}
}

async fn server(dir: &std::path::Path) -> agentd_api::Server {
    let (db, mut services) = db_services(dir);
    services.tree = Some(Arc::new(agentd_db::TreeRepo::new(
        db.clone(),
        Arc::new(SystemClock),
        Arc::new(Quiet),
        None,
    )));
    std::mem::forget(db);
    start_with(config(dir), services).await
}

async fn session(s: &agentd_api::Server) -> i64 {
    let r = unix_body(
        s.socket_path(),
        "POST",
        "/v1/sessions",
        TOKEN_CHAT,
        &[],
        json!({"name": "test"}).to_string().as_bytes(),
    )
    .await;
    assert_eq!(r.status, 201, "{}", r.body);
    r.json()["id"].as_i64().expect("session id")
}

async fn get(s: &agentd_api::Server, route: &str, token: &str) -> common::Resp {
    unix_req(s.socket_path(), "GET", route, Some(token)).await
}

async fn put(
    s: &agentd_api::Server,
    route: &str,
    token: &str,
    if_match: Option<&str>,
    body: &Value,
) -> common::Resp {
    let h = if_match.map(|v| format!("If-Match: {v}"));
    let hs: Vec<&str> = h.iter().map(String::as_str).collect();
    unix_body(
        s.socket_path(),
        "PUT",
        route,
        token,
        &hs,
        body.to_string().as_bytes(),
    )
    .await
}

fn code(r: &common::Resp) -> String {
    r.json()["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn link(t: &str, u: &str) -> Value {
    json!({"title": t, "url": u})
}

#[tokio::test]
async fn bookmarks_roundtrip_with_etag() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let id = session(&s).await;
    let route = format!("/v1/sessions/{id}/bookmarks");
    let g = get(&s, &route, TOKEN_READ).await;
    assert_eq!(
        (g.status, g.json(), g.header("etag")),
        (200, json!([]), Some("\"0\""))
    );
    let list = json!([
        link("Example", "https://example.org/"),
        link("Two", "http://a.b/c?d=1")
    ]);
    let p = put(&s, &route, TOKEN_CHAT_ONLY, Some("\"0\""), &list).await;
    assert_eq!(
        (p.status, p.json(), p.header("etag")),
        (200, list.clone(), Some("\"1\"")),
        "{}",
        p.body
    );
    let g = get(&s, &route, TOKEN_READ).await;
    assert_eq!(
        (g.status, g.json(), g.header("etag")),
        (200, list, Some("\"1\""))
    );
}

#[tokio::test]
async fn bookmarks_if_match_rules() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let id = session(&s).await;
    let route = format!("/v1/sessions/{id}/bookmarks");
    let one = json!([link("a", "https://a.b/")]);
    assert_eq!(put(&s, &route, TOKEN_CHAT, None, &one).await.status, 428);
    assert_eq!(
        put(&s, &route, TOKEN_CHAT, Some("abc"), &one).await.status,
        400
    );
    assert_eq!(
        put(&s, &route, TOKEN_CHAT, Some("\"0\""), &one)
            .await
            .status,
        200
    );
    let stale = put(&s, &route, TOKEN_CHAT, Some("\"0\""), &one).await;
    assert_eq!(
        (stale.status, code(&stale)),
        (412, "precondition_failed".to_string())
    );
    assert_eq!(
        get(&s, &route, TOKEN_READ).await.header("etag"),
        Some("\"1\"")
    );
}

#[tokio::test]
async fn links_body_validation() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let id = session(&s).await;
    let route = format!("/v1/sessions/{id}/bookmarks");
    for bad in [
        json!({"title": "a"}),
        json!([{"title": "", "url": "https://a.b/"}]),
        json!([{"title": "a", "url": "javascript:alert(1)"}]),
        json!([{"title": "a", "url": "https://a b/"}]),
        json!([{"title": "a", "url": "https://a.b/", "x": 1}]),
        json!(["https://a.b/"]),
    ] {
        let r = put(&s, &route, TOKEN_CHAT, Some("\"0\""), &bad).await;
        assert_eq!(
            (r.status, code(&r)),
            (422, "validation_failed".to_string()),
            "{bad}"
        );
    }
    let r = unix_body(
        s.socket_path(),
        "PUT",
        &route,
        TOKEN_CHAT,
        &["If-Match: \"0\""],
        b"not json",
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(
        get(&s, &route, TOKEN_READ).await.header("etag"),
        Some("\"0\"")
    );
}

#[tokio::test]
async fn bookmarks_scope_and_missing_session() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let id = session(&s).await;
    let route = format!("/v1/sessions/{id}/bookmarks");
    let one = json!([link("a", "https://a.b/")]);
    assert_eq!(
        put(&s, &route, TOKEN_READ, Some("\"0\""), &one)
            .await
            .status,
        403
    );
    assert_eq!(get(&s, &route, "wrong").await.status, 401);
    assert_eq!(
        get(&s, "/v1/sessions/9999/bookmarks", TOKEN_READ)
            .await
            .status,
        404
    );
    assert_eq!(
        put(
            &s,
            "/v1/sessions/9999/bookmarks",
            TOKEN_CHAT,
            Some("\"0\""),
            &one
        )
        .await
        .status,
        404
    );
}

#[tokio::test]
async fn bookmarks_are_per_session_and_dropped_with_session() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let (a, b) = (session(&s).await, session(&s).await);
    let (ra, rb) = (
        format!("/v1/sessions/{a}/bookmarks"),
        format!("/v1/sessions/{b}/bookmarks"),
    );
    let one = json!([link("a", "https://a.b/")]);
    assert_eq!(
        put(&s, &ra, TOKEN_CHAT, Some("\"0\""), &one).await.status,
        200
    );
    assert_eq!(get(&s, &rb, TOKEN_READ).await.json(), json!([]));
    let d = unix_req(
        s.socket_path(),
        "DELETE",
        &format!("/v1/sessions/{a}"),
        Some(TOKEN_CHAT),
    )
    .await;
    assert_eq!(d.status, 204, "{}", d.body);
    assert_eq!(get(&s, &ra, TOKEN_READ).await.status, 404);
}

#[tokio::test]
async fn quicklinks_default_and_update() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let g = get(&s, "/v1/browser/quicklinks", TOKEN_READ).await;
    assert_eq!((g.status, g.header("etag")), (200, Some("\"0\"")));
    let def = g.json();
    assert_eq!(def.as_array().map(Vec::len), Some(5), "{def}");
    let mine = json!([link("Mine", "https://my.example/")]);
    let p = put(
        &s,
        "/v1/browser/quicklinks",
        TOKEN_CHAT_ONLY,
        Some("\"0\""),
        &mine,
    )
    .await;
    assert_eq!(
        (p.status, p.header("etag")),
        (200, Some("\"1\"")),
        "{}",
        p.body
    );
    let g = get(&s, "/v1/browser/quicklinks", TOKEN_READ).await;
    assert_eq!((g.json(), g.header("etag")), (mine, Some("\"1\"")));
    let empty = put(
        &s,
        "/v1/browser/quicklinks",
        TOKEN_CHAT,
        Some("\"1\""),
        &json!([]),
    )
    .await;
    assert_eq!(empty.status, 200);
    assert_eq!(
        get(&s, "/v1/browser/quicklinks", TOKEN_READ).await.json(),
        json!([])
    );
    assert_eq!(
        put(
            &s,
            "/v1/browser/quicklinks",
            TOKEN_READ,
            Some("\"2\""),
            &json!([])
        )
        .await
        .status,
        403
    );
}
