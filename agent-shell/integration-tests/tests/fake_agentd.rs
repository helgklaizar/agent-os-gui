//! `fake-agentd` over a real socket and TCP: token required, scope, `Origin`, conditional requests.

mod common;

use common::{WAIT, error_of, http, issue, quiet, send, start, tcp};
use core_client::Token;
use core_client::http::{RawResponse, Request};
use fake_agentd::{Config, Kind, Scope};
use serde_json::{Value, json};

#[tokio::test]
async fn fake_agentd_requires_token() {
    let d = quiet().await;
    let anon = http(&d, None);
    for (method, path) in [
        ("GET", "/v1/settings/theme"),
        ("GET", "/v1/tokens"),
        ("POST", "/v1/_test/echo"),
        ("GET", "/v1/no-such-route"),
        ("DELETE", "/v1/tokens/1"),
    ] {
        let req = match method {
            "GET" => Request::get(path),
            "POST" => Request::post(path, json!({})),
            _ => Request::delete(path),
        };
        let r = send(&anon, req.anonymous()).await;
        let (code, _) = error_of(&r);
        assert_eq!(
            (r.status, code.as_str()),
            (401, "unauthorized"),
            "{method} {path} without a token"
        );
    }
    let health = send(&anon, Request::get("/v1/health").anonymous()).await;
    assert_eq!(health.status, 200, "health is anonymous");
    assert_eq!(
        health.json::<Value>().map(|v| v["status"].clone()).ok(),
        Some(json!("ok"))
    );
    // our own format accepted, a foreign one rejected
    let ok = send(
        &http(&d, Some(d.admin_token())),
        Request::get("/v1/settings/theme"),
    )
    .await;
    assert_eq!(ok.status, 200, "the issued token accepted");
    let bad = send(
        &http(&d, Some("agt_not-a-token")),
        Request::get("/v1/settings/theme"),
    )
    .await;
    assert_eq!(bad.status, 401, "a token of a foreign format rejected");
    let r = send(
        &anon,
        Request::get("/v1/settings/theme").header("Authorization", "Basic abc"),
    )
    .await;
    assert_eq!(r.status, 401, "non-Bearer rejected");
}

#[tokio::test]
async fn request_id_is_echoed_and_generated() {
    let d = quiet().await;
    let c = http(&d, Some(d.admin_token()));
    let r = send(&c, Request::get("/v1/settings/theme")).await;
    let sent = d
        .requests()
        .last()
        .map(|x| x.request_id.clone())
        .unwrap_or_default();
    assert_eq!(
        r.header("x-request-id"),
        Some(sent.as_str()),
        "the response returns the same id"
    );
    let parsed = uuid::Uuid::parse_str(&sent);
    assert!(
        matches!(&parsed, Ok(u) if u.get_version_num() == 7),
        "the client sets a UUID v7: {sent}"
    );
    // the error duplicates the id in the body
    let e = send(
        &http(&d, None),
        Request::get("/v1/tokens")
            .anonymous()
            .header("X-Request-Id", "0192f3a8-7c1e-7d44-9a3b-5e6f1c2d3b4a"),
    )
    .await;
    let body: Value = serde_json::from_slice(&e.body).unwrap_or(Value::Null);
    assert_eq!(
        body["error"]["request_id"],
        json!("0192f3a8-7c1e-7d44-9a3b-5e6f1c2d3b4a")
    );
    assert_eq!(
        e.header("x-request-id"),
        Some("0192f3a8-7c1e-7d44-9a3b-5e6f1c2d3b4a")
    );
    // no header or not a UUID: the core creates its own
    for bad in ["not-a-uuid", ""] {
        let r = send(
            &c,
            Request::get("/v1/settings/theme").header("X-Request-Id", bad),
        )
        .await;
        let own = r.header("x-request-id").unwrap_or("");
        assert!(
            uuid::Uuid::parse_str(own).is_ok() && own != bad,
            "{bad:?} replaced with its own: {own}"
        );
    }
    assert!(r_version_header(
        &send(&c, Request::get("/v1/settings/theme")).await
    ));
}

fn r_version_header(r: &RawResponse) -> bool {
    r.header("x-agentd-version").is_some()
}

#[tokio::test]
async fn fake_agentd_scope_enforced() {
    let d = quiet().await;
    let read = http(&d, Some(&issue(&d, Scope::Read, Kind::Test)));
    let chat = http(&d, Some(&issue(&d, Scope::Chat, Kind::Test)));
    let admin = http(&d, Some(d.admin_token()));
    let patch = |rev: i64| {
        Request::patch("/v1/settings/theme", json!({ "theme": "dark" }))
            .header("If-Match", &rev.to_string())
    };

    assert_eq!(
        send(&read, Request::get("/v1/settings/theme")).await.status,
        200,
        "read reads"
    );
    let r = send(&read, Request::get("/v1/tokens")).await;
    assert_eq!(
        (r.status, error_of(&r).0.as_str()),
        (403, "insufficient_scope"),
        "read on an admin route"
    );
    let r = send(&read, patch(1)).await;
    assert_eq!(
        (r.status, error_of(&r).0.as_str()),
        (403, "insufficient_scope"),
        "read on a chat route"
    );
    let r = send(&chat, Request::get("/v1/tokens")).await;
    assert_eq!(
        (r.status, error_of(&r).0.as_str()),
        (403, "insufficient_scope"),
        "chat on an admin route"
    );
    assert_eq!(
        send(&chat, patch(1)).await.status,
        200,
        "chat edits the theme"
    );
    assert_eq!(
        send(&admin, Request::get("/v1/tokens")).await.status,
        200,
        "admin sees tokens"
    );
}

#[tokio::test]
async fn fake_agentd_tcp_origin_checked() {
    let d = start(
        Config::temp()
            .no_ticker()
            .with_tcp()
            .allow_origin("https://tunnel.example"),
    )
    .await;
    let c =
        core_client::HttpClient::new(tcp(&d), Some(Token::new(d.admin_token()))).with_timeout(WAIT);
    let get = |path: &str| Request::get(path);
    assert_eq!(
        send(&c, get("/v1/settings/theme")).await.status,
        200,
        "without Origin (not a browser) accepted"
    );
    assert_eq!(
        send(
            &c,
            get("/v1/settings/theme").header("Origin", "https://tunnel.example")
        )
        .await
        .status,
        200,
        "Origin from the allowlist accepted"
    );
    for origin in [
        "http://evil.example",
        "null",
        "file://",
        "http://127.0.0.1.evil.example",
    ] {
        let r = send(&c, get("/v1/settings/theme").header("Origin", origin)).await;
        assert_eq!(
            (r.status, error_of(&r).0.as_str()),
            (403, "forbidden_origin"),
            "foreign Origin {origin}"
        );
    }
    let r = send(&c, get("/v1/settings/theme").header("Host", "evil.example")).await;
    assert_eq!(
        (r.status, error_of(&r).0.as_str()),
        (403, "forbidden_origin"),
        "foreign Host"
    );
    // x-local-only: rejected over TCP regardless of the token
    let r = send(&c, get("/v1/tokens")).await;
    assert_eq!(
        (r.status, error_of(&r).0.as_str()),
        (403, "insufficient_scope"),
        "local route over TCP"
    );
    assert_eq!(
        send(&http(&d, Some(d.admin_token())), get("/v1/tokens"))
            .await
            .status,
        200,
        "the same token over the socket"
    );
    // a foreign Origin is rejected for health too: the check runs before routes
    let r = send(
        &c,
        get("/v1/health")
            .anonymous()
            .header("Origin", "http://evil.example"),
    )
    .await;
    assert_eq!(r.status, 403);
}
