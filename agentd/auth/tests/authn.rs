//! Token checks on transports: real `TokenStore` and `agentd-api` over socket and TCP.

mod common;

use std::io::Write;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use agentd_api::{Server, ServerConfig};
use agentd_core::{AuthFuture, Authenticator, Scope};
use common::{Env, Resp, env, external, http};
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// Wrapper over the real check: counts store accesses (proof that "the table was not touched").
struct Counting {
    inner: Arc<agentd_auth::TokenStore>,
    calls: AtomicUsize,
}

impl Authenticator for Counting {
    fn authenticate<'a>(&'a self, token: &'a str) -> AuthFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.authenticate(token)
    }
}

static LOG: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn log_text() -> String {
    String::from_utf8_lossy(&LOG.get().unwrap().lock().unwrap()).into_owned()
}

fn init_logs() {
    LOG.get_or_init(|| {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let sink = buf.clone();
        let sub = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || LogWriter(sink.clone()))
            .finish();
        tracing::subscriber::set_global_default(sub).unwrap();
        buf
    });
}

struct Rig {
    env: Env,
    server: Server,
    auth: Arc<Counting>,
}

async fn rig(tcp: bool, tune: impl FnOnce(&mut ServerConfig)) -> Rig {
    init_logs();
    let env = env();
    let auth = Arc::new(Counting {
        inner: env.store.clone(),
        calls: AtomicUsize::new(0),
    });
    let mut cfg = ServerConfig::new(env.dir.path().join("agentd.sock"));
    cfg.tcp_addr = "127.0.0.1:0".parse().unwrap();
    cfg.tcp_enabled = tcp;
    cfg.clock = env.clock.clone();
    tune(&mut cfg);
    let server = Server::start(cfg, auth.clone()).await.unwrap();
    Rig { env, server, auth }
}

impl Rig {
    async fn unix(&self, method: &str, path: &str, extra: &[String]) -> Resp {
        let mut s = tokio::net::UnixStream::connect(self.server.socket_path())
            .await
            .unwrap();
        http(&mut s, method, path, "localhost", extra)
            .await
            .expect("response")
    }

    async fn addr(&self) -> SocketAddr {
        for _ in 0..300 {
            if let Some(a) = self.server.tcp().local_addr() {
                return a;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("TCP did not come up");
    }

    async fn tcp(&self, method: &str, path: &str, extra: &[String]) -> Resp {
        let addr = self.addr().await;
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        http(
            &mut s,
            method,
            path,
            &format!("127.0.0.1:{}", addr.port()),
            extra,
        )
        .await
        .expect("response")
    }

    async fn issue(&self, scope: Scope) -> String {
        self.env
            .store
            .issue(external(scope))
            .await
            .unwrap()
            .value
            .expose()
            .clone()
    }
}

fn bearer(t: &str) -> Vec<String> {
    vec![format!("Authorization: Bearer {t}")]
}

fn fake_token() -> String {
    agentd_auth::token::generate()
}

fn ws_req(
    url: &str,
    token: Option<&str>,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut r = url.into_client_request().unwrap();
    if let Some(t) = token {
        r.headers_mut()
            .insert("authorization", format!("Bearer {t}").parse().unwrap());
    }
    r
}

async fn close_code<S>(ws: &mut tokio_tungstenite::WebSocketStream<S>) -> u16
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("close in time")
        {
            Some(Ok(Message::Close(Some(f)))) => return u16::from(f.code),
            Some(Ok(_)) => continue,
            other => panic!("expected a close with a code: {other:?}"),
        }
    }
}

async fn ws_unix(
    r: &Rig,
    path: &str,
    token: Option<&str>,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>,
    tokio_tungstenite::tungstenite::Error,
> {
    let s = tokio::net::UnixStream::connect(r.server.socket_path())
        .await
        .unwrap();
    tokio_tungstenite::client_async(ws_req(&format!("ws://localhost{path}"), token), s)
        .await
        .map(|(w, _)| w)
}

#[tokio::test]
async fn query_token_rejected_401() {
    let r = rig(true, |_| {}).await;
    let secret = fake_token();
    let good = r.issue(Scope::Chat).await;
    let calls_before = r.auth.calls.load(Ordering::SeqCst);
    for q in [
        format!("token={secret}"),
        format!("access_token={secret}"),
        format!("%74oken={secret}"),
        format!("access%5Ftoken={secret}"),
        format!("x=1&TOKEN={secret}"),
        format!("a=b&%41ccess_token={secret}"),
    ] {
        for route in ["/v1/health", "/v1/sessions", "/v1/ws"] {
            if route == "/v1/ws" {
                continue; // WS is checked below
            }
            let resp = r.unix("GET", &format!("{route}?{q}"), &bearer(&good)).await;
            assert_eq!(resp.status, 401, "{route}?{q}");
            let body = resp.json();
            assert_eq!(body["error"]["code"], "unauthorized");
            assert_eq!(body["error"]["details"]["reason"], "token_in_url");
            let tcp = r.tcp("GET", &format!("{route}?{q}"), &bearer(&good)).await;
            assert_eq!(tcp.status, 401, "TCP {route}?{q}");
        }
        let mut ws = ws_unix(&r, &format!("/v1/ws?{q}"), Some(&good))
            .await
            .expect("upgrade, then close");
        assert_eq!(close_code(&mut ws).await, 4401, "WS ?{q}");
    }
    assert_eq!(
        r.auth.calls.load(Ordering::SeqCst),
        calls_before,
        "the token table was not accessed"
    );
    let ok = r
        .unix("GET", "/v1/browser/quicklinks?x=token", &bearer(&good))
        .await;
    // 503 core_down: authentication passed, but the rig has no services (the bookmarks handler is implemented).
    assert_eq!(
        ok.status, 503,
        "a `token` value not in a parameter name is left alone: {ok:?}"
    );
    assert!(
        !log_text().contains(&secret),
        "the token value from the URL did not reach the log"
    );
    r.server.shutdown().await;
}

#[tokio::test]
async fn ws_close_4401() {
    let r = rig(false, |_| {}).await;
    let mut short = external(Scope::Chat);
    short.ttl_days = Some(1);
    let expiring = r
        .env
        .store
        .issue(short)
        .await
        .unwrap()
        .value
        .expose()
        .clone();
    let revoked = r.env.store.issue(external(Scope::Chat)).await.unwrap();
    r.env.store.revoke(revoked.id).await.unwrap();
    r.env.clock.advance_secs(2 * 86_400);
    let good = r
        .env
        .store
        .issue(external(Scope::Chat))
        .await
        .unwrap()
        .value
        .expose()
        .clone();
    for (name, tok) in [
        ("expired", expiring),
        ("revoked", revoked.value.expose().clone()),
        ("unknown", fake_token()),
    ] {
        let mut ws = ws_unix(&r, "/v1/ws", Some(&tok))
            .await
            .expect("upgrade, then close");
        assert_eq!(close_code(&mut ws).await, 4401, "{name}");
    }
    let ws = ws_unix(&r, "/v1/ws", Some(&good)).await;
    assert!(ws.is_ok(), "valid token accepted");
    r.server.shutdown().await;
}

#[tokio::test]
async fn rate_limit_429() {
    let r = rig(true, |_| {}).await;
    let valid = r.issue(Scope::Chat).await;
    let from = |ip: &str| vec![format!("X-Forwarded-For: {ip}")];
    let attempt = |ip: &'static str, tok: String| {
        let mut h = bearer(&tok);
        h.extend(from(ip));
        h
    };
    for i in 0..5 {
        let resp = r
            .tcp("GET", "/v1/sessions", &attempt("203.0.113.5", fake_token()))
            .await;
        assert_eq!(resp.status, 401, "attempt {i}");
    }
    let limited = r
        .tcp("GET", "/v1/sessions", &attempt("203.0.113.5", fake_token()))
        .await;
    assert_eq!(limited.status, 429);
    let retry: u64 = limited
        .header("retry-after")
        .expect("Retry-After")
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry), "{retry}");
    let body = limited.json();
    assert_eq!(body["error"]["code"], "rate_limited");
    assert_eq!(body["error"]["retryable"], true);
    assert_eq!(
        body["error"]["request_id"].as_str(),
        limited.header("x-request-id")
    );
    let still = r
        .tcp(
            "GET",
            "/v1/sessions",
            &attempt("203.0.113.5", valid.clone()),
        )
        .await;
    assert_eq!(
        still.status, 429,
        "address is paused, even a valid token waits"
    );
    // another address is unaffected
    let other = r
        .tcp(
            "GET",
            "/v1/browser/quicklinks",
            &attempt("198.51.100.9", valid.clone()),
        )
        .await;
    assert_eq!(
        other.status, 503,
        "authentication passed, the rig has no services (core_down)"
    );
    // no limit over the unix socket
    for _ in 0..8 {
        assert_eq!(
            r.unix("GET", "/v1/sessions", &bearer(&fake_token()))
                .await
                .status,
            401
        );
    }
    // the window expired
    r.env.clock.advance_secs(61);
    let after = r
        .tcp("GET", "/v1/sessions", &attempt("203.0.113.5", fake_token()))
        .await;
    assert_eq!(after.status, 401, "the count is reset after a minute");
    r.server.shutdown().await;
}

#[tokio::test]
async fn rate_limit_forwarded_for_rightmost() {
    let r = rig(true, |c| {
        c.trusted_proxies.push("10.9.0.1".parse().unwrap())
    })
    .await;
    let hdr = |xff: &str| {
        let mut h = bearer(&fake_token());
        h.push(format!("X-Forwarded-For: {xff}"));
        h
    };
    // forgery on the left: bucket by the first untrusted address from the right (203.0.113.7)
    for i in 0..5 {
        let resp = r
            .tcp(
                "GET",
                "/v1/sessions",
                &hdr(&format!("1.2.3.{i}, 203.0.113.7")),
            )
            .await;
        assert_eq!(resp.status, 401);
    }
    assert_eq!(
        r.tcp("GET", "/v1/sessions", &hdr("9.9.9.9, 203.0.113.7"))
            .await
            .status,
        429,
        "spoofing the left address does not bypass the limit"
    );
    // trusted proxies on the right are skipped
    assert_eq!(
        r.tcp("GET", "/v1/sessions", &hdr("203.0.113.7, 127.0.0.1"))
            .await
            .status,
        429
    );
    assert_eq!(
        r.tcp(
            "GET",
            "/v1/sessions",
            &hdr("203.0.113.7, 10.9.0.1, 127.0.0.1")
        )
        .await
        .status,
        429
    );
    // another client behind the same proxy
    for _ in 0..5 {
        assert_eq!(
            r.tcp("GET", "/v1/sessions", &hdr("198.51.100.1, 10.9.0.1"))
                .await
                .status,
            401
        );
    }
    assert_eq!(
        r.tcp("GET", "/v1/sessions", &hdr("198.51.100.1, 10.9.0.1"))
            .await
            .status,
        429
    );
    assert_eq!(
        r.tcp("GET", "/v1/sessions", &hdr("198.51.100.2, 10.9.0.1"))
            .await
            .status,
        401,
        "the neighbouring address is alive"
    );
    // without the header, by the connection address (127.0.0.1)
    let plain = |t: String| bearer(&t);
    for _ in 0..5 {
        assert_eq!(
            r.tcp("GET", "/v1/sessions", &plain(fake_token()))
                .await
                .status,
            401
        );
    }
    assert_eq!(
        r.tcp("GET", "/v1/sessions", &plain(fake_token()))
            .await
            .status,
        429
    );
    r.server.shutdown().await;
}

#[tokio::test]
async fn unknown_tokens_are_not_remembered_by_limiter() {
    // The bucket is the address, not the token value: a thousand different wrong tokens from different addresses do not grow memory without bound.
    let r = rig(true, |_| {}).await;
    for i in 0..60 {
        let mut h = bearer(&fake_token());
        h.push(format!("X-Forwarded-For: 192.0.2.{i}"));
        assert_eq!(r.tcp("GET", "/v1/sessions", &h).await.status, 401);
    }
    r.env.clock.advance_secs(61);
    let mut h = bearer(&fake_token());
    h.push("X-Forwarded-For: 192.0.2.1".into());
    assert_eq!(r.tcp("GET", "/v1/sessions", &h).await.status, 401);
    r.server.shutdown().await;
}

#[tokio::test]
async fn peercred() {
    let r = rig(false, |_| {}).await;
    // the same uid without an access token gets nothing: the token decides, not the uid
    assert_eq!(r.unix("GET", "/v1/sessions", &[]).await.status, 401);
    assert_eq!(r.unix("GET", "/v1/ws", &[]).await.status, 401);
    let good = r.issue(Scope::Read).await;
    assert_eq!(
        r.unix("GET", "/v1/browser/quicklinks", &bearer(&good))
            .await
            .status,
        503,
        "with a token authentication passes (the rig has no services: core_down)"
    );
    r.server.shutdown().await;
}

#[tokio::test]
async fn no_anonymous_route_except_health() {
    let r = rig(true, |_| {}).await;
    let routes = [
        ("GET", "/v1/health", 200),
        ("GET", "/health", 401),
        ("HEAD", "/v1/health", 200),
        ("POST", "/v1/health", 401),
        ("GET", "/v1/ws", 401),
        ("GET", "/v1/tokens", 401),
        ("POST", "/v1/tokens", 401),
        ("DELETE", "/v1/tokens/1", 401),
        ("GET", "/v1/settings/theme", 401),
        ("PATCH", "/v1/settings/theme", 401),
        ("GET", "/v1/metrics", 401),
        ("GET", "/v1/sessions", 401),
        ("PUT", "/v1/secrets/x", 401),
        ("GET", "/", 401),
        ("GET", "/v1", 401),
        ("GET", "/v1/", 401),
        ("GET", "/v1/health/", 401),
        ("GET", "/v1/%68ealth", 401),
        ("GET", "/v1/../v1/tokens", 401),
        ("OPTIONS", "/v1/tokens", 401),
        ("GET", "/_test/advance", 401),
    ];
    for (method, path, want) in routes {
        let u = r.unix(method, path, &[]).await;
        assert_eq!(u.status, want, "unix {method} {path}");
        let t = r.tcp(method, path, &[]).await;
        assert_eq!(t.status, want, "tcp {method} {path}");
        if want == 401 {
            assert_eq!(u.json()["error"]["code"], "unauthorized", "{method} {path}");
        }
    }
    for junk in [
        "Bearer",
        "Bearer ",
        "Basic abc",
        "bearer",
        "Token xyz",
        "Bearer agt_",
    ] {
        let resp = r
            .unix("GET", "/v1/sessions", &[format!("Authorization: {junk}")])
            .await;
        assert_eq!(resp.status, 401, "{junk:?}");
    }
    assert_eq!(
        r.auth.calls.load(Ordering::SeqCst),
        1,
        "the store was queried only for `Bearer agt_`: empty and foreign schemes are cut off before the port"
    );

    r.server.shutdown().await;
}
