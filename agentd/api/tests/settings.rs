//! A1-09: settings pages over HTTP: `ETag`, `If-Match`, JSON merge patch, `rev`, scope.

mod common;

use std::time::Duration;

use common::{
    TOKEN_CHAT, TOKEN_CHAT_ONLY, TOKEN_READ, config, db_services, start_with, unix_body, unix_req,
};
use serde_json::{Value, json};

async fn server(dir: &std::path::Path) -> agentd_api::Server {
    let (db, services) = db_services(dir);
    std::mem::forget(db); // the DB lives as long as the test process: the temp dir closes with it
    start_with(config(dir), services).await
}

fn code(r: &common::Resp) -> String {
    r.json()["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

async fn patch(
    s: &agentd_api::Server,
    page: &str,
    token: &str,
    if_match: Option<&str>,
    body: &Value,
) -> common::Resp {
    let h = if_match.map(|v| format!("If-Match: {v}"));
    let hs: Vec<&str> = h.iter().map(String::as_str).collect();
    unix_body(
        s.socket_path(),
        "PATCH",
        &format!("/v1/settings/{page}"),
        token,
        &hs,
        body.to_string().as_bytes(),
    )
    .await
}

#[tokio::test]
async fn settings_etag_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let g = unix_req(
        s.socket_path(),
        "GET",
        "/v1/settings/theme",
        Some(TOKEN_READ),
    )
    .await;
    assert_eq!(g.status, 200);
    assert_eq!(g.json(), json!({"rev": 0, "theme": "auto"}));
    assert_eq!(g.header("etag"), Some("\"0\""));
    let p = patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        Some("\"0\""),
        &json!({"theme": "dark"}),
    )
    .await;
    assert_eq!(p.status, 200, "{}", p.body);
    assert_eq!(p.json(), json!({"rev": 1, "theme": "dark"}));
    assert_eq!(p.header("etag"), Some("\"1\""));
    let g = unix_req(
        s.socket_path(),
        "GET",
        "/v1/settings/theme",
        Some(TOKEN_READ),
    )
    .await;
    assert_eq!(
        (g.json()["theme"].clone(), g.header("etag")),
        (json!("dark"), Some("\"1\""))
    );
    // Unquoted If-Match and the weak form are accepted the same way.
    let p = patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        Some("1"),
        &json!({"theme": "light"}),
    )
    .await;
    assert_eq!((p.status, p.json()["rev"].clone()), (200, json!(2)));
    let p = patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        Some("W/\"2\""),
        &json!({"theme": "auto"}),
    )
    .await;
    assert_eq!((p.status, p.json()["rev"].clone()), (200, json!(3)));
    s.shutdown().await;
}

#[tokio::test]
async fn generic_page_merge_patch_and_rev_field() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let a = patch(
        &s,
        "schedule",
        TOKEN_CHAT,
        Some("0"),
        &json!({"summary": {"time": "08:30", "enabled": true}}),
    )
    .await;
    assert_eq!(
        (a.status, a.json()["rev"].clone()),
        (200, json!(1)),
        "{}",
        a.body
    );
    // `rev` in the body is a service field: the client may return the document as is; the core does not treat it as data.
    let b = patch(
        &s,
        "schedule",
        TOKEN_CHAT,
        Some("1"),
        &json!({"rev": 99, "summary": {"enabled": null}}),
    )
    .await;
    assert_eq!(b.status, 200, "{}", b.body);
    assert_eq!(b.json(), json!({"rev": 2, "summary": {"time": "08:30"}}));
    let g = unix_req(
        s.socket_path(),
        "GET",
        "/v1/settings/schedule",
        Some(TOKEN_READ),
    )
    .await;
    assert_eq!(g.json(), json!({"rev": 2, "summary": {"time": "08:30"}}));
    s.shutdown().await;
}

#[tokio::test]
async fn settings_patch_without_if_match_428() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let r = patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        None,
        &json!({"theme": "dark"}),
    )
    .await;
    assert_eq!(
        (r.status, code(&r).as_str()),
        (428, "precondition_required")
    );
    let r = patch(
        &s,
        "schedule",
        TOKEN_CHAT,
        None,
        &json!({"summary": {"enabled": true}}),
    )
    .await;
    assert_eq!(r.status, 428);
    let g = unix_req(
        s.socket_path(),
        "GET",
        "/v1/settings/theme",
        Some(TOKEN_READ),
    )
    .await;
    assert_eq!(g.json()["rev"], 0, "nothing written without If-Match");
    let r = patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        Some("abc"),
        &json!({"theme": "dark"}),
    )
    .await;
    assert_eq!((r.status, code(&r).as_str()), (400, "bad_request"));
    s.shutdown().await;
}

#[tokio::test]
async fn settings_stale_if_match_412() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        Some("0"),
        &json!({"theme": "dark"}),
    )
    .await;
    let r = patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        Some("0"),
        &json!({"theme": "light"}),
    )
    .await;
    assert_eq!((r.status, code(&r).as_str()), (412, "precondition_failed"));
    assert_eq!(r.json()["error"]["details"]["rev"], 1);
    let g = unix_req(
        s.socket_path(),
        "GET",
        "/v1/settings/theme",
        Some(TOKEN_READ),
    )
    .await;
    assert_eq!(g.json()["theme"], "dark", "value unchanged");
    s.shutdown().await;
}

#[tokio::test]
async fn settings_theme_chat_scope() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let r = patch(
        &s,
        "theme",
        TOKEN_READ,
        Some("0"),
        &json!({"theme": "dark"}),
    )
    .await;
    assert_eq!((r.status, code(&r).as_str()), (403, "insufficient_scope"));
    let r = patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        Some("0"),
        &json!({"theme": "dark"}),
    )
    .await;
    assert_eq!(r.status, 200);
    // Only admin changes the other pages.
    let r = patch(
        &s,
        "schedule",
        TOKEN_CHAT_ONLY,
        Some("0"),
        &json!({"summary": {"enabled": true}}),
    )
    .await;
    assert_eq!((r.status, code(&r).as_str()), (403, "insufficient_scope"));
    s.shutdown().await;
}

#[tokio::test]
async fn settings_rev_monotonic() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let mut rev = 0;
    for i in 0..15 {
        let r = patch(
            &s,
            "account",
            TOKEN_CHAT,
            Some(&rev.to_string()),
            &json!({"lat": i}),
        )
        .await;
        assert_eq!(r.status, 200);
        let new = r.json()["rev"].as_i64().unwrap();
        assert_eq!(new, rev + 1, "rev grows by exactly 1");
        rev = new;
    }
    // Two "tabs" with the same rev: the first wins, the second gets 412 and overwrites nothing.
    let a = patch(
        &s,
        "account",
        TOKEN_CHAT,
        Some(&rev.to_string()),
        &json!({"name": "gui"}),
    )
    .await;
    let b = patch(
        &s,
        "account",
        TOKEN_CHAT,
        Some(&rev.to_string()),
        &json!({"name": "phone"}),
    )
    .await;
    assert_eq!((a.status, b.status), (200, 412));
    let g = unix_req(
        s.socket_path(),
        "GET",
        "/v1/settings/account",
        Some(TOKEN_READ),
    )
    .await;
    assert_eq!(g.json()["name"], "gui");
    s.shutdown().await;
}

#[tokio::test]
async fn invalid_unknown_readonly_and_bad_bodies() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    let r = patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        Some("0"),
        &json!({"theme": "red"}),
    )
    .await;
    assert_eq!((r.status, code(&r).as_str()), (422, "validation_failed"));
    assert_eq!(r.json()["error"]["details"]["fields"][0]["field"], "theme");
    let r = patch(
        &s,
        "indicators",
        TOKEN_CHAT,
        Some("0"),
        &json!({"items": [{"id": "cpu"}]}),
    )
    .await;
    assert_eq!(r.status, 422);
    let ok = json!({"items": [{"id": "cpu", "enabled": true, "position": 0}]});
    let r = patch(&s, "indicators", TOKEN_CHAT, Some("0"), &ok).await;
    assert_eq!(
        (r.status, r.json()["items"][0]["id"].clone()),
        (200, json!("cpu"))
    );
    let r = patch(&s, "nope", TOKEN_CHAT, Some("0"), &json!({})).await;
    assert_eq!((r.status, code(&r).as_str()), (404, "not_found"));
    let r = unix_req(
        s.socket_path(),
        "GET",
        "/v1/settings/nope",
        Some(TOKEN_READ),
    )
    .await;
    assert_eq!(r.status, 404);
    let r = patch(&s, "about", TOKEN_CHAT, Some("0"), &json!({"x": 1})).await;
    assert_eq!((r.status, code(&r).as_str()), (403, "permission_denied"));
    let bad = unix_body(
        s.socket_path(),
        "PATCH",
        "/v1/settings/schedule",
        TOKEN_CHAT,
        &["If-Match: 0"],
        b"{oops",
    )
    .await;
    assert_eq!((bad.status, code(&bad).as_str()), (400, "bad_request"));
    s.shutdown().await;
}

#[tokio::test]
async fn tunnels_page_switches_tcp_without_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.tcp_enabled = false;
    let (db, services) = db_services(dir.path());
    std::mem::forget(db);
    let s = start_with(cfg, services).await;
    assert!(s.tcp().local_addr().is_none());
    let r = patch(
        &s,
        "tunnels",
        TOKEN_CHAT,
        Some("0"),
        &json!({"api_external": true}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let mut up = false;
    for _ in 0..100 {
        if s.tcp().local_addr().is_some() {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(up, "TCP came up per the setting");
    patch(
        &s,
        "tunnels",
        TOKEN_CHAT,
        Some("1"),
        &json!({"api_external": false}),
    )
    .await;
    let mut down = false;
    for _ in 0..100 {
        if s.tcp().local_addr().is_none() {
            down = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(down, "TCP stopped per the setting");
    s.shutdown().await;
}

#[tokio::test]
async fn settings_topic_snapshot_and_delta_over_ws() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path()).await;
    patch(
        &s,
        "schedule",
        TOKEN_CHAT,
        Some("0"),
        &json!({"summary": {"enabled": true}}),
    )
    .await;
    let stream = tokio::net::UnixStream::connect(s.socket_path())
        .await
        .unwrap();
    let mut req = "ws://localhost/v1/ws".into_client_request().unwrap();
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {TOKEN_READ}").parse().unwrap(),
    );
    let (mut ws, _) = tokio_tungstenite::client_async(req, stream).await.unwrap();
    ws.send(Message::text(
        json!({"op": "subscribe", "topic": "settings"}).to_string(),
    ))
    .await
    .unwrap();
    let snap = next_text(&mut ws).await;
    assert_eq!(
        (snap["type"].clone(), snap["topic"].clone()),
        (json!("snapshot"), json!("settings"))
    );
    assert_eq!(snap["data"]["pages"]["schedule"], 1);
    patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        Some("0"),
        &json!({"theme": "dark"}),
    )
    .await;
    let d = next_text(&mut ws).await;
    assert_eq!(d["type"], "delta");
    assert_eq!(d["data"], json!({"page": "theme", "rev": 1}));
    assert_eq!(d["seq"], 2);
    // A rejection (412) publishes no changes.
    patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        Some("0"),
        &json!({"theme": "light"}),
    )
    .await;
    patch(
        &s,
        "theme",
        TOKEN_CHAT_ONLY,
        Some("1"),
        &json!({"theme": "light"}),
    )
    .await;
    let d = next_text(&mut ws).await;
    assert_eq!(
        d["data"],
        json!({"page": "theme", "rev": 2}),
        "nothing extra between frames"
    );
    s.shutdown().await;
}

async fn next_text(ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>) -> Value {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    loop {
        let m = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("frame arrived")
            .unwrap()
            .unwrap();
        if let Message::Text(t) = m {
            return serde_json::from_str::<Value>(t.as_str()).unwrap();
        }
    }
}
