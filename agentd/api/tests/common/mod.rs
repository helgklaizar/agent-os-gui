//! Transport test helpers: raw HTTP/1.1 over any stream and a swappable token-check port.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use agentd_api::{AuthFailure, Authenticator, Principal, Server, ServerConfig};
use agentd_core::{Scope, TokenKind};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const TOKEN_CHAT: &str = "agt_test_chat";
pub const TOKEN_READ: &str = "agt_test_read";
pub const TOKEN_CHAT_ONLY: &str = "agt_test_chat_only";
pub const TOKEN_MODULE: &str = "agt_test_module";
pub const TOKEN_AGENT: &str = "agt_test_agent";

/// Auth port stand-in (tests only, not in the product): known values -> principal.
pub struct StaticAuth(pub HashMap<String, Principal>);

impl Authenticator for StaticAuth {
    fn authenticate<'a>(&'a self, token: &'a str) -> agentd_api::AuthFuture<'a> {
        Box::pin(async move { self.0.get(token).cloned().ok_or(AuthFailure::Invalid) })
    }
}

fn principal(id: i64, kind: TokenKind, scope: Scope, allowlist: Option<&[&str]>) -> Principal {
    Principal {
        token_id: id,
        kind,
        scope,
        allowlist: allowlist.map(|l| l.iter().map(|s| (*s).to_string()).collect()),
        expires_at: None,
    }
}

pub fn auth() -> Arc<StaticAuth> {
    let mut m = HashMap::new();
    m.insert(
        TOKEN_READ.to_string(),
        principal(2, TokenKind::External, Scope::Read, None),
    );
    m.insert(
        TOKEN_CHAT_ONLY.to_string(),
        principal(3, TokenKind::External, Scope::Chat, None),
    );
    m.insert(
        TOKEN_MODULE.to_string(),
        principal(
            4,
            TokenKind::Module,
            Scope::Chat,
            Some(&["GET /sessions", "POST /sessions"]),
        ),
    );
    // An agent with an allowlist that "by mistake" contains admin routes: it still cannot decide permissions.
    m.insert(
        TOKEN_AGENT.to_string(),
        principal(
            5,
            TokenKind::Agent,
            Scope::Chat,
            Some(&[
                "POST /permission-requests/{id}/decision",
                "PUT /permissions/{scope}/{scope_id}/{action}",
                "GET /sessions",
            ]),
        ),
    );
    m.insert(
        TOKEN_CHAT.to_string(),
        Principal {
            token_id: 1,
            kind: TokenKind::Gui,
            scope: Scope::Admin,
            allowlist: None,
            expires_at: None,
        },
    );
    Arc::new(StaticAuth(m))
}

pub fn uid() -> u32 {
    agentd_sys::current_uid()
}

pub fn config(dir: &std::path::Path) -> ServerConfig {
    let mut c = ServerConfig::new(dir.join("agentd.sock"));
    c.expected_uid = uid();
    c.tcp_addr = "127.0.0.1:0".parse().unwrap();
    c
}

pub async fn start(cfg: ServerConfig) -> Server {
    Server::start(cfg, auth()).await.expect("server starts")
}

#[derive(Debug)]
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Resp {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

/// One request with `Connection: close`; `extra` is header lines without trailing CRLF.
pub async fn http<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    method: &str,
    path: &str,
    host: &str,
    extra: &[&str],
) -> Option<Resp> {
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for h in extra {
        req.push_str(h);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await.ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.ok()?;
    if buf.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n")?;
    let mut lines = head.lines();
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let body = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        dechunk(body)
    } else {
        body.to_string()
    };
    Some(Resp {
        status,
        headers,
        body,
    })
}

fn dechunk(mut body: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = body.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 || rest.len() < n {
            break;
        }
        out.push_str(&rest[..n]);
        body = rest[n..].trim_start_matches("\r\n");
    }
    out
}

pub async fn unix_get(path: &std::path::Path, route: &str, extra: &[&str]) -> Option<Resp> {
    let mut s = tokio::net::UnixStream::connect(path).await.ok()?;
    http(&mut s, "GET", route, "localhost", extra).await
}

pub async fn tcp_get(
    addr: std::net::SocketAddr,
    host: &str,
    route: &str,
    extra: &[&str],
) -> Option<Resp> {
    let mut s = tokio::net::TcpStream::connect(addr).await.ok()?;
    http(&mut s, "GET", route, host, extra).await
}

/// One request over the unix socket with a token (or without).
pub async fn unix_req(
    path: &std::path::Path,
    method: &str,
    route: &str,
    token: Option<&str>,
) -> Resp {
    let mut s = tokio::net::UnixStream::connect(path).await.expect("socket");
    let auth = token.map(|t| format!("Authorization: Bearer {t}"));
    let extra: Vec<&str> = auth.iter().map(String::as_str).collect();
    http(&mut s, method, route, "localhost", &extra)
        .await
        .expect("response")
}

/// One request over TCP with a token (or without); `Host` is the server's own address.
pub async fn tcp_req(
    addr: std::net::SocketAddr,
    method: &str,
    route: &str,
    token: Option<&str>,
) -> Resp {
    let mut s = tokio::net::TcpStream::connect(addr).await.expect("TCP");
    let auth = token.map(|t| format!("Authorization: Bearer {t}"));
    let extra: Vec<&str> = auth.iter().map(String::as_str).collect();
    http(&mut s, method, route, &addr.to_string(), &extra)
        .await
        .expect("response")
}

/// Route from `openapi-draft.yaml`: method, path (relative to `/v1`), minimum scope, local-only, anonymous.
#[derive(Debug, Clone)]
pub struct OaRoute {
    pub method: String,
    pub path: String,
    pub scope: Option<String>,
    pub local_only: bool,
    pub anonymous: bool,
}

/// Line-by-line parse of the `paths` section (extensions `x-scope`, `x-local-only`, `security: []`).
pub fn openapi_routes() -> Vec<OaRoute> {
    let file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/reference/openapi-draft.yaml"
    );
    let text = std::fs::read_to_string(file).expect("openapi-draft.yaml is readable");
    let mut path = String::new();
    let mut out: Vec<OaRoute> = Vec::new();
    for line in text.lines() {
        if line.starts_with("components:") {
            break;
        }
        if let Some(p) = line.strip_prefix("  /").and_then(|p| p.strip_suffix(':')) {
            path = format!("/{p}");
            continue;
        }
        if let Some(m) = line
            .strip_prefix("    ")
            .and_then(|m| m.strip_suffix(':'))
            .filter(|m| ["get", "post", "put", "patch", "delete"].contains(m))
        {
            out.push(OaRoute {
                method: m.to_ascii_uppercase(),
                path: path.clone(),
                scope: None,
                local_only: false,
                anonymous: false,
            });
            continue;
        }
        let Some(last) = out.last_mut() else { continue };
        if let Some(sc) = line.strip_prefix("      x-scope: ") {
            last.scope = Some(sc.trim().to_string());
        }
        if line.trim() == "x-local-only: true" && line.starts_with("      x-") {
            last.local_only = true;
        }
        if line.trim() == "security: []" && line.starts_with("      security") {
            last.anonymous = true;
        }
    }
    out
}

/// Substitutes a concrete value for `{...}`.
pub fn concrete(path: &str) -> String {
    let mut out = String::new();
    let mut skip = false;
    for c in path.chars() {
        match c {
            '{' => {
                skip = true;
                out.push('1');
            }
            '}' => skip = false,
            _ if !skip => out.push(c),
            _ => {}
        }
    }
    out
}

/// A real DB in a temp dir and services on top of it (token check: `StaticAuth`, state: `Db`).
pub fn db_services(dir: &std::path::Path) -> (Arc<agentd_db::Db>, agentd_api::Services) {
    let db = Arc::new(
        agentd_db::Db::open(agentd_db::DbConfig::new(dir.join("agentos/sessions.db")))
            .expect("DB opens"),
    );
    let mut s = agentd_api::Services::new(auth());
    s.status = Some(db.clone());
    s.settings = Some(Arc::new(agentd_db::SettingsRepo::new(
        db.clone(),
        Arc::new(agentd_core::SystemClock),
    )));
    (db, s)
}

pub async fn start_with(cfg: ServerConfig, services: agentd_api::Services) -> Server {
    let server = Server::start_booting(cfg).await.expect("server starts");
    server.set_ready(services);
    server
}

/// Request with a body and arbitrary headers (token: `Authorization`).
pub async fn unix_body(
    path: &std::path::Path,
    method: &str,
    route: &str,
    token: &str,
    headers: &[&str],
    body: &[u8],
) -> Resp {
    let mut s = tokio::net::UnixStream::connect(path).await.expect("socket");
    let mut req = format!(
        "{method} {route} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    for h in headers {
        req.push_str(h);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    let mut bytes = req.into_bytes();
    bytes.extend_from_slice(body);
    s.write_all(&bytes).await.expect("write");
    let mut buf = Vec::new();
    // The server may respond and close before the body ends (413): a read abort is not a test error.
    let _ = s.read_to_end(&mut buf).await;
    parse_response(&buf).expect("response parsed")
}

pub fn parse_response(buf: &[u8]) -> Option<Resp> {
    let text = String::from_utf8_lossy(buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n")?;
    let mut lines = head.lines();
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let body = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        dechunk(body)
    } else {
        body.to_string()
    };
    Some(Resp {
        status,
        headers,
        body,
    })
}
