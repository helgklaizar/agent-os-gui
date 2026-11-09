//! W1: browser client static files, own origin, WS token from the subprotocol, dev CORS.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use agentd_api::{Server, ServerConfig};
use common::{TOKEN_CHAT, config, http, start, tcp_get};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::client_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

async fn up(dir: &std::path::Path, tweak: impl FnOnce(&mut ServerConfig)) -> (Server, SocketAddr) {
    let web = dir.join("web");
    std::fs::create_dir_all(web.join("sub")).unwrap();
    std::fs::write(web.join("index.html"), "<html>ok</html>").unwrap();
    std::fs::write(web.join("sub/app.wasm"), [0u8, 97, 115, 109]).unwrap();
    std::fs::write(dir.join("secret.txt"), "secret").unwrap();
    let mut cfg = config(dir);
    cfg.web_root = Some(web);
    tweak(&mut cfg);
    let s = start(cfg).await;
    s.tcp().set_enabled(true);
    for _ in 0..300 {
        if let Some(a) = s.tcp().local_addr() {
            return (s, a);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("TCP did not come up");
}

fn host(a: SocketAddr) -> String {
    format!("127.0.0.1:{}", a.port())
}

#[tokio::test]
async fn static_is_anonymous_with_security_headers() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a) = up(dir.path(), |_| {}).await;
    let r = tcp_get(a, &host(a), "/", &[]).await.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.body, "<html>ok</html>");
    assert!(r.header("content-type").unwrap().starts_with("text/html"));
    let csp = r.header("content-security-policy").unwrap();
    assert!(csp.contains("default-src 'none'") && csp.contains("wasm-unsafe-eval"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.header("cache-control"), Some("no-cache"));
    assert!(r.header("x-request-id").is_some());
    let w = tcp_get(a, &host(a), "/app/sub/app.wasm", &[])
        .await
        .unwrap();
    assert_eq!(w.status, 200);
    assert_eq!(w.header("content-type"), Some("application/wasm"));
    // Conditional request by ETag.
    let etag = r.header("etag").unwrap().to_string();
    let inm = format!("If-None-Match: {etag}");
    let nm = tcp_get(a, &host(a), "/", &[inm.as_str()]).await.unwrap();
    assert_eq!(nm.status, 304);
    // Data still requires a token.
    let api = tcp_get(a, &host(a), "/v1/sessions", &[]).await.unwrap();
    assert_eq!(api.status, 401);
    s.shutdown().await;
}

#[tokio::test]
async fn static_cannot_leave_root() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a) = up(dir.path(), |_| {}).await;
    std::os::unix::fs::symlink(
        dir.path().join("secret.txt"),
        dir.path().join("web/link.txt"),
    )
    .unwrap();
    for route in [
        "/app/../secret.txt",
        "/app/sub/../../secret.txt",
        "/app/link.txt",
        "/app//etc/passwd",
        "/app/%2e%2e/secret.txt",
        "/app/missing.js",
    ] {
        let r = tcp_get(a, &host(a), route, &[]).await.unwrap();
        assert_eq!(r.status, 404, "{route}");
        assert!(!r.body.contains("secret"), "{route}");
    }
    s.shutdown().await;
}

#[tokio::test]
async fn static_off_without_web_root() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a) = up(dir.path(), |c| c.web_root = None).await;
    let r = tcp_get(a, &host(a), "/", &[]).await.unwrap();
    assert_eq!(
        r.status, 401,
        "without --web-root there is no root; the \"everything needs a token\" rule applies"
    );
    s.shutdown().await;
}

#[tokio::test]
async fn own_origin_allowed_foreign_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a) = up(dir.path(), |_| {}).await;
    let own = format!("Origin: http://{}", host(a));
    let tok = format!("Authorization: Bearer {TOKEN_CHAT}");
    let ok = tcp_get(a, &host(a), "/v1/health", &[own.as_str(), tok.as_str()])
        .await
        .unwrap();
    assert_eq!(ok.status, 200);
    let bad = tcp_get(
        a,
        &host(a),
        "/v1/health",
        &["Origin: http://evil.example", tok.as_str()],
    )
    .await
    .unwrap();
    assert_eq!(bad.status, 403);
    assert_eq!(bad.json()["error"]["code"], "forbidden_origin");
    let wrong_host = tcp_get(a, "evil.example", "/", &[]).await.unwrap();
    assert_eq!(wrong_host.status, 403);
    s.shutdown().await;
}

async fn ws_open(
    a: SocketAddr,
    origin: &str,
    protocols: &str,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<TcpStream>,
        tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
    ),
    tokio_tungstenite::tungstenite::Error,
> {
    let stream = TcpStream::connect(a).await.unwrap();
    let mut req = format!("ws://{}/v1/ws", host(a))
        .into_client_request()
        .unwrap();
    req.headers_mut().insert("origin", origin.parse().unwrap());
    if !protocols.is_empty() {
        req.headers_mut()
            .insert("sec-websocket-protocol", protocols.parse().unwrap());
    }
    client_async(req, stream).await
}

#[tokio::test]
async fn ws_token_from_subprotocol() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a) = up(dir.path(), |_| {}).await;
    let origin = format!("http://{}", host(a));
    let proto = format!("agentd.v1, bearer.{TOKEN_CHAT}");
    let (mut ws, resp) = ws_open(a, &origin, &proto).await.expect("handshake");
    let chosen = resp
        .headers()
        .get("sec-websocket-protocol")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        chosen, "agentd.v1",
        "core selects only agentd.v1 and does not echo the token"
    );
    ws.send(Message::Text(json!({"op": "ping"}).to_string().into()))
        .await
        .unwrap();
    let mut got_pong = false;
    for _ in 0..5 {
        if let Some(Ok(Message::Text(t))) = ws.next().await
            && t.contains("pong")
        {
            got_pong = true;
            break;
        }
    }
    assert!(got_pong);
    s.shutdown().await;
}

#[tokio::test]
async fn ws_bad_or_missing_token_closes_4401_foreign_origin_4403() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a) = up(dir.path(), |_| {}).await;
    let origin = format!("http://{}", host(a));
    let (mut ws, _) = ws_open(a, &origin, "agentd.v1, bearer.agt_wrong")
        .await
        .expect("upgrade present");
    match ws.next().await {
        Some(Ok(Message::Close(Some(f)))) => assert_eq!(u16::from(f.code), 4401),
        other => panic!("expected close 4401, got {other:?}"),
    }
    // Without a token the handshake is rejected with HTTP 401 (as with the `Authorization` header).
    match ws_open(a, &origin, "agentd.v1").await {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => assert_eq!(r.status().as_u16(), 401),
        other => panic!("expected 401 rejection, got {:?}", other.map(|_| ())),
    }
    let proto = format!("agentd.v1, bearer.{TOKEN_CHAT}");
    let (mut ws, _) = ws_open(a, "http://evil.example", &proto)
        .await
        .expect("upgrade present");
    match ws.next().await {
        Some(Ok(Message::Close(Some(f)))) => assert_eq!(u16::from(f.code), 4403),
        other => panic!("expected close 4403, got {other:?}"),
    }
    s.shutdown().await;
}

#[tokio::test]
async fn subprotocol_token_ignored_on_plain_http() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a) = up(dir.path(), |_| {}).await;
    let proto = format!("Sec-WebSocket-Protocol: bearer.{TOKEN_CHAT}");
    let r = tcp_get(a, &host(a), "/v1/sessions", &[proto.as_str()])
        .await
        .unwrap();
    assert_eq!(
        r.status, 401,
        "outside a WS upgrade the subprotocol token is not accepted"
    );
    s.shutdown().await;
}

#[tokio::test]
async fn dev_cors_only_for_listed_origins() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a) = up(dir.path(), |c| {
        c.dev_cors = vec!["http://localhost:5173".into()]
    })
    .await;
    let mut st = TcpStream::connect(a).await.unwrap();
    let pre = http(
        &mut st,
        "OPTIONS",
        "/v1/sessions",
        &host(a),
        &[
            "Origin: http://localhost:5173",
            "Access-Control-Request-Method: GET",
            "Access-Control-Request-Headers: authorization",
        ],
    )
    .await
    .unwrap();
    assert_eq!(pre.status, 204);
    assert_eq!(
        pre.header("access-control-allow-origin"),
        Some("http://localhost:5173")
    );
    assert!(
        pre.header("access-control-allow-headers")
            .unwrap()
            .contains("Authorization")
    );
    let tok = format!("Authorization: Bearer {TOKEN_CHAT}");
    let get = tcp_get(
        a,
        &host(a),
        "/v1/health",
        &["Origin: http://localhost:5173", tok.as_str()],
    )
    .await
    .unwrap();
    assert_eq!(get.status, 200);
    assert_eq!(
        get.header("access-control-allow-origin"),
        Some("http://localhost:5173")
    );
    assert_ne!(get.header("access-control-allow-origin"), Some("*"));
    let other = tcp_get(
        a,
        &host(a),
        "/v1/health",
        &["Origin: http://localhost:9999", tok.as_str()],
    )
    .await
    .unwrap();
    assert_eq!(other.status, 403);
    assert!(other.header("access-control-allow-origin").is_none());
    s.shutdown().await;
}

#[tokio::test]
async fn no_cors_headers_without_dev_cors() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a) = up(dir.path(), |_| {}).await;
    let own = format!("Origin: http://{}", host(a));
    let r = tcp_get(a, &host(a), "/", &[own.as_str()]).await.unwrap();
    assert_eq!(r.status, 200);
    assert!(r.header("access-control-allow-origin").is_none());
    s.shutdown().await;
}

#[tokio::test]
async fn static_size_limit_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a) = up(dir.path(), |_| {}).await;
    let web = dir.path().join("web");
    let at = std::fs::File::create(web.join("at.bin")).unwrap();
    at.set_len(32 * 1024 * 1024).unwrap();
    let over = std::fs::File::create(web.join("over.bin")).unwrap();
    over.set_len(32 * 1024 * 1024 + 1).unwrap();
    assert_eq!(
        tcp_get(a, &host(a), "/app/at.bin", &[])
            .await
            .unwrap()
            .status,
        200
    );
    assert_eq!(
        tcp_get(a, &host(a), "/app/over.bin", &[])
            .await
            .unwrap()
            .status,
        404
    );
    // A directory instead of a file is not served.
    assert_eq!(
        tcp_get(a, &host(a), "/app/sub", &[]).await.unwrap().status,
        404
    );
    s.shutdown().await;
}

#[tokio::test]
async fn preflight_needs_options_and_request_method() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a) = up(dir.path(), |c| {
        c.dev_cors = vec!["http://localhost:5173".into()]
    })
    .await;
    let o = "Origin: http://localhost:5173";
    // OPTIONS without `Access-Control-Request-Method` is not a preflight: 204 with permissions is not returned.
    let mut st = TcpStream::connect(a).await.unwrap();
    let r = http(&mut st, "OPTIONS", "/v1/health", &host(a), &[o])
        .await
        .unwrap();
    assert_ne!(r.status, 204, "{r:?}");
    assert!(r.header("access-control-allow-methods").is_none());
    // GET with `Access-Control-Request-Method` is a normal request, not a preflight.
    let r = tcp_get(
        a,
        &host(a),
        "/v1/health",
        &[o, "Access-Control-Request-Method: GET"],
    )
    .await
    .unwrap();
    assert_eq!(r.status, 200, "{r:?}");
    assert!(r.header("access-control-allow-methods").is_none());
    s.shutdown().await;
}
