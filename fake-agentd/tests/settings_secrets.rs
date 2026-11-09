//! Write-only secrets (`/secrets`), `admin` and local only, the value is never returned.

mod s7_common;

use fake_agentd::{Config, Kind, Scope};
use s7_common::*;
use serde_json::json;

const SECRET: &str = "FIXTURE-SECRET-VALUE-0001";

#[tokio::test(flavor = "multi_thread")]
async fn secrets_are_write_only() {
    let r = rig().await;
    let put = r
        .call(
            "PUT",
            "/v1/secrets/provider.anthropic",
            Some(json!({"value": SECRET})),
        )
        .await;
    assert_eq!(put.status, 200, "{}", text(&put));
    assert_eq!(put.json()["ref"], "provider.anthropic");
    assert_eq!(put.json()["set"], true);
    assert!(put.json()["updated_at"].is_number());
    assert!(
        !text(&put).contains(SECRET),
        "the value is not in the response"
    );
    assert_eq!(
        r.d.secret_value("provider.anthropic").as_deref(),
        Some(SECRET),
        "value written"
    );
    let list = r.call("GET", "/v1/secrets", None).await;
    assert_eq!(list.status, 200);
    assert_eq!(list.json()[0]["ref"], "provider.anthropic");
    assert_eq!(list.json()[0]["set"], true);
    assert!(!text(&list).contains(SECRET));
    let single = r.call("GET", "/v1/secrets/provider.anthropic", None).await;
    assert!(
        matches!(single.status, 404 | 405),
        "GET of a secret does not exist: {}",
        single.status
    );
    assert!(!text(&single).contains(SECRET));
    let sec = r.get("/v1/settings/security").await;
    assert_eq!(
        sec["secrets_present"], true,
        "presence without names and values"
    );
    assert!(!sec.to_string().contains("anthropic"));
    // overwrite and deletion
    let again = r
        .call(
            "PUT",
            "/v1/secrets/provider.anthropic",
            Some(json!({"value": "other-FIXTURE"})),
        )
        .await;
    assert_eq!(again.status, 200);
    assert_eq!(
        r.call("DELETE", "/v1/secrets/provider.anthropic", None)
            .await
            .status,
        204
    );
    assert_eq!(r.d.secret_value("provider.anthropic"), None);
    assert_eq!(r.call("GET", "/v1/secrets", None).await.json(), json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_value_never_in_errors_pages_or_files() {
    let r = rig().await;
    r.call(
        "PUT",
        "/v1/secrets/account.3",
        Some(json!({"value": SECRET})),
    )
    .await;
    let mut seen = String::new();
    for path in [
        "/v1/settings/models",
        "/v1/settings/connections",
        "/v1/settings/security",
        "/v1/accounts",
        "/v1/files",
        "/v1/secrets",
    ] {
        seen.push_str(&text(&r.call("GET", path, None).await));
    }
    let bad = r
        .call("PUT", "/v1/secrets/bad-ref", Some(json!({"value": SECRET})))
        .await;
    seen.push_str(&text(&bad));
    assert_eq!(bad.status, 422);
    assert_eq!(fields(&bad), vec!["ref".to_string()]);
    assert!(!seen.contains(SECRET));
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_validation() {
    let r = rig().await;
    for bad_ref in [
        "provider.",
        "provider.UP",
        "account.0",
        "account.x",
        "other.a",
        "provider.a/b",
    ] {
        let w = r
            .call(
                "PUT",
                &format!("/v1/secrets/{bad_ref}"),
                Some(json!({"value": "v"})),
            )
            .await;
        assert!(matches!(w.status, 422 | 404), "{bad_ref}: {}", w.status);
    }
    for body in [
        json!({"value": ""}),
        json!({"value": "x".repeat(8193)}),
        json!({"value": 5}),
        json!({}),
    ] {
        let w = r.call("PUT", "/v1/secrets/provider.ok", Some(body)).await;
        assert_eq!(w.status, 422, "{}", text(&w));
        assert_eq!(fields(&w), vec!["value".to_string()]);
    }
    for good in [
        "provider.a1",
        "provider.a_b",
        "provider.a-b",
        "provider.9",
        "account.12",
    ] {
        let w = r
            .call(
                "PUT",
                &format!("/v1/secrets/{good}"),
                Some(json!({"value": "v"})),
            )
            .await;
        assert_eq!(w.status, 200, "{good}: {}", text(&w));
    }
    let edge = r
        .call(
            "PUT",
            "/v1/secrets/provider.ok",
            Some(json!({"value": "x".repeat(8192)})),
        )
        .await;
    assert_eq!(edge.status, 200, "exactly 8 KiB is allowed");
    assert_eq!(
        r.call("PUT", "/v1/secrets/provider.ok", None).await.status,
        400
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_only_endpoints_reject_chat_token() {
    let r = rig().await;
    let chat = r.token(Scope::Chat, Kind::External);
    for (m, p, b) in [
        ("PUT", "/v1/secrets/provider.x", Some(json!({"value": "v"}))),
        ("DELETE", "/v1/secrets/provider.x", None),
        ("GET", "/v1/secrets", None),
        (
            "PUT",
            "/v1/permissions/global/0/send",
            Some(json!({"mode": "deny"})),
        ),
        ("GET", "/v1/files", None),
    ] {
        let w = r.call_as(&chat, m, p, b).await;
        assert_eq!(
            (w.status, code(&w)),
            (403, "insufficient_scope".into()),
            "{m} {p}"
        );
    }
    assert_eq!(
        r.d.secret_value("provider.x"),
        None,
        "the refusal wrote nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_and_files_are_local_only() {
    let r = start(Config::temp().no_ticker().manual_clock().with_tcp()).await;
    let addr = r.d.tcp_addr().map(|a| a.to_string()).unwrap_or_default();
    assert!(!addr.is_empty());
    let tcp_status = |method: &'static str, path: &'static str| {
        let (addr, token) = (addr.clone(), r.admin.clone());
        async move { tcp_call(&addr, &token, method, path).await }
    };
    for (m, p) in [
        ("GET", "/v1/secrets"),
        ("PUT", "/v1/secrets/provider.x"),
        ("GET", "/v1/files"),
    ] {
        let (status, body) = tcp_status(m, p).await;
        assert_eq!(status, 403, "{m} {p}: {body}");
        assert!(body.contains("local_only"), "{body}");
    }
}

async fn tcp_call(addr: &str, token: &str, method: &str, path: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.expect("tcp");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    s.write_all(req.as_bytes()).await.expect("write");
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.expect("read");
    let text = String::from_utf8_lossy(&out).to_string();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (status, text)
}
