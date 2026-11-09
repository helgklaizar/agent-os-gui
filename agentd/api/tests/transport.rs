//! A1-04: transports: unix socket and TCP, uid check, Origin/Host, HTTP and WS on one socket.

mod common;

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::time::Duration;

use common::{TOKEN_CHAT, config, start, tcp_get, uid, unix_get};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

fn ws_request(
    host: &str,
    token: Option<&str>,
    origin: Option<&str>,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut req = format!("ws://{host}/v1/ws").into_client_request().unwrap();
    if let Some(t) = token {
        req.headers_mut()
            .insert("authorization", format!("Bearer {t}").parse().unwrap());
    }
    if let Some(o) = origin {
        req.headers_mut().insert("origin", o.parse().unwrap());
    }
    req
}

async fn expect_close_code<S>(ws: &mut tokio_tungstenite::WebSocketStream<S>) -> u16
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("server closed WS in time")
        {
            Some(Ok(Message::Close(Some(f)))) => return u16::from(f.code),
            Some(Ok(Message::Close(None))) | None => panic!("close without a code"),
            Some(Ok(_)) => continue,
            Some(Err(e)) => panic!("WS error: {e}"),
        }
    }
}

#[tokio::test]
async fn socket_mode_0600() {
    let dir = tempfile::tempdir().unwrap();
    let server = start(config(dir.path())).await;
    let meta = std::fs::metadata(server.socket_path()).expect("socket created");
    assert!(meta.file_type().is_socket(), "a socket is at the path");
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let health = unix_get(server.socket_path(), "/v1/health", &[])
        .await
        .expect("response over the socket");
    assert_eq!(health.status, 200);
    server.shutdown().await;
}

#[tokio::test]
async fn socket_replaces_stale_file_but_not_live_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agentd.sock");
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap()); // dead socket file left behind
    let first = start(config(dir.path())).await;
    assert_eq!(
        unix_get(&path, "/v1/health", &[])
            .await
            .expect("response")
            .status,
        200,
        "stale file replaced"
    );
    let second = agentd_api::Server::start(config(dir.path()), common::auth()).await;
    assert!(second.is_err(), "live socket is not taken over");
    assert_eq!(
        unix_get(&path, "/v1/health", &[])
            .await
            .expect("first is alive")
            .status,
        200
    );
    first.shutdown().await;
}

#[tokio::test]
async fn peercred_foreign_uid_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.expected_uid = uid().wrapping_add(1); // "foreign" user: our uid does not match the expected one
    let server = start(cfg).await;
    assert!(
        unix_get(server.socket_path(), "/v1/health", &[])
            .await
            .is_none(),
        "no response for a foreign uid: connection closed"
    );
    server.shutdown().await;
    let dir2 = tempfile::tempdir().unwrap();
    let ok = start(config(dir2.path())).await;
    assert_eq!(
        unix_get(ok.socket_path(), "/v1/health", &[])
            .await
            .expect("own uid")
            .status,
        200
    );
    ok.shutdown().await;
}

#[tokio::test]
async fn tcp_disabled_by_default() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        !agentd_api::ServerConfig::new(dir.path().join("s")).tcp_enabled,
        "TCP is off by default"
    );
    assert_eq!(
        agentd_api::ServerConfig::new(dir.path().join("s"))
            .tcp_addr
            .to_string(),
        "127.0.0.1:4096"
    );
    let server = start(config(dir.path())).await;
    assert!(server.tcp().local_addr().is_none(), "TCP is not listening");
    server.tcp().set_enabled(true);
    let addr = wait_bound(&server).await;
    let on = tcp_get(
        addr,
        &format!("127.0.0.1:{}", addr.port()),
        "/v1/health",
        &[],
    )
    .await
    .expect("TCP enabled without restart");
    assert_eq!(on.status, 200);
    server.tcp().set_enabled(false);
    wait_unbound(&server).await;
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "port closed after disable"
    );
    server.shutdown().await;
}

async fn wait_bound(server: &agentd_api::Server) -> std::net::SocketAddr {
    for _ in 0..300 {
        if let Some(a) = server.tcp().local_addr() {
            return a;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("TCP did not come up");
}

async fn wait_unbound(server: &agentd_api::Server) {
    for _ in 0..300 {
        if server.tcp().local_addr().is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("TCP did not stop");
}

#[tokio::test]
async fn tcp_origin_host_allowlist() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.tcp_enabled = true;
    cfg.allowed_hosts = vec!["t.example.com".into()];
    cfg.allowed_origins = vec!["https://t.example.com".into()];
    let server = start(cfg).await;
    let addr = wait_bound(&server).await;
    let own = format!("127.0.0.1:{}", addr.port());
    let get = |host: String, extra: Vec<String>| async move {
        let extra: Vec<&str> = extra.iter().map(String::as_str).collect();
        tcp_get(addr, &host, "/v1/health", &extra)
            .await
            .expect("response")
    };
    assert_eq!(
        get(own.clone(), vec![]).await.status,
        200,
        "own Host without Origin"
    );
    assert_eq!(
        get(format!("localhost:{}", addr.port()), vec![])
            .await
            .status,
        200,
        "localhost:port"
    );
    assert_eq!(
        get(
            "t.example.com".into(),
            vec!["Origin: https://t.example.com".into()]
        )
        .await
        .status,
        200,
        "tunnel address"
    );
    for (host, origin) in [
        ("evil.example".to_string(), None),
        (own.clone(), Some("http://evil.example".to_string())),
        (own.clone(), Some("null".to_string())),
        (own.clone(), Some("file://".to_string())),
        (
            own.clone(),
            Some(format!("http://localhost:{}", addr.port())),
        ), // WebKit page without explicit permission
        (
            "t.example.com".to_string(),
            Some("https://other.example.com".to_string()),
        ),
    ] {
        let extra = origin.iter().map(|o| format!("Origin: {o}")).collect();
        let r = get(host.clone(), extra).await;
        assert_eq!(r.status, 403, "{host} {origin:?}");
        let body = r.json();
        assert_eq!(body["error"]["code"], "forbidden_origin");
        assert_eq!(body["error"]["retryable"], false);
        assert_eq!(
            body["error"]["request_id"].as_str(),
            r.header("x-request-id"),
            "request_id matches in body and header"
        );
    }
    // WS with a foreign Origin: upgrade and close 4403
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_request(
        &own,
        Some(TOKEN_CHAT),
        Some("http://evil.example"),
    ))
    .await
    .expect("upgrade succeeds");
    assert_eq!(expect_close_code(&mut ws).await, 4403);
    // Host and Origin are not checked over the unix socket
    let local = unix_get(
        &dir.path().join("agentd.sock"),
        "/v1/health",
        &["Origin: http://anything.example"],
    )
    .await
    .expect("response");
    assert_eq!(local.status, 200);
    server.shutdown().await;
}

#[tokio::test]
async fn http_and_ws_on_one_socket() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.tcp_enabled = true;
    let server = start(cfg).await;
    let sock = server.socket_path().to_path_buf();
    assert_eq!(
        unix_get(&sock, "/v1/health", &[])
            .await
            .expect("HTTP")
            .status,
        200
    );
    let stream = tokio::net::UnixStream::connect(&sock).await.unwrap();
    let (mut ws, resp) =
        tokio_tungstenite::client_async(ws_request("localhost", Some(TOKEN_CHAT), None), stream)
            .await
            .expect("WS on the same socket");
    assert_eq!(resp.status().as_u16(), 101);
    ws.send(Message::Ping(b"x".to_vec().into())).await.unwrap();
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("pong in time")
        {
            Some(Ok(Message::Pong(p))) => {
                assert_eq!(&p[..], b"x");
                break;
            }
            Some(Ok(_)) => continue,
            other => panic!("expected pong: {other:?}"),
        }
    }
    // while WS is open, HTTP on the same socket keeps responding
    assert_eq!(
        unix_get(&sock, "/v1/health", &[])
            .await
            .expect("HTTP while WS is open")
            .status,
        200
    );
    ws.close(None).await.unwrap();
    // same over TCP
    let addr = wait_bound(&server).await;
    let own = format!("127.0.0.1:{}", addr.port());
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut ws2, resp2) =
        tokio_tungstenite::client_async(ws_request(&own, Some(TOKEN_CHAT), None), tcp)
            .await
            .expect("WS over TCP");
    assert_eq!(resp2.status().as_u16(), 101);
    ws2.close(None).await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn ws_without_token_is_401_and_bad_token_closes_4401() {
    let dir = tempfile::tempdir().unwrap();
    let server = start(config(dir.path())).await;
    let sock = server.socket_path().to_path_buf();
    let stream = tokio::net::UnixStream::connect(&sock).await.unwrap();
    let err = tokio_tungstenite::client_async(ws_request("localhost", None, None), stream)
        .await
        .expect_err("no upgrade without a token");
    match err {
        tokio_tungstenite::tungstenite::Error::Http(r) => assert_eq!(r.status().as_u16(), 401),
        other => panic!("expected HTTP 401: {other:?}"),
    }
    let stream = tokio::net::UnixStream::connect(&sock).await.unwrap();
    let (mut ws, _) =
        tokio_tungstenite::client_async(ws_request("localhost", Some("agt_wrong"), None), stream)
            .await
            .expect("upgrade, then close");
    assert_eq!(expect_close_code(&mut ws).await, 4401);
    server.shutdown().await;
}
