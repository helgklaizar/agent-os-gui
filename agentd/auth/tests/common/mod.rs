//! Shared token test helpers: controllable clock, temp-directory DB, raw HTTP.
#![allow(dead_code)]

use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use agentd_auth::{NewToken, TokenPolicy, TokenStore};
use agentd_core::{Clock, Scope, TokenKind};
use agentd_db::{Db, DbConfig};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Controllable clock (test implementation of the `Clock` port).
#[derive(Debug)]
pub struct ManualClock(AtomicI64);

impl ManualClock {
    pub fn new(start_secs: i64) -> Arc<ManualClock> {
        Arc::new(ManualClock(AtomicI64::new(start_secs * 1000)))
    }

    pub fn advance_secs(&self, s: i64) {
        self.0.fetch_add(s * 1000, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_unix(&self) -> i64 {
        self.0.load(Ordering::SeqCst) / 1000
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.0.load(Ordering::SeqCst)).unwrap_or(0)
    }
}

pub const T0: i64 = 1_800_000_000;

pub struct Env {
    pub dir: tempfile::TempDir,
    pub db: Arc<Db>,
    pub clock: Arc<ManualClock>,
    pub store: Arc<TokenStore>,
}

pub fn env() -> Env {
    env_with(TokenPolicy::default())
}

pub fn env_with(policy: TokenPolicy) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(DbConfig::new(dir.path().join("agentos/sessions.db"))).unwrap());
    let clock = ManualClock::new(T0);
    let store = Arc::new(TokenStore::new(db.clone(), clock.clone(), policy));
    Env {
        dir,
        db,
        clock,
        store,
    }
}

pub fn new_token(kind: TokenKind, scope: Scope) -> NewToken {
    NewToken {
        label: "test".into(),
        kind,
        scope,
        ttl_days: None,
        allowlist: None,
        created_by: None,
    }
}

pub fn external(scope: Scope) -> NewToken {
    new_token(TokenKind::External, scope)
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

pub async fn http<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    method: &str,
    path: &str,
    host: &str,
    extra: &[String],
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

/// Request with a body; the server may answer and close the connection before reading the whole body (413).
pub async fn http_body<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    method: &str,
    path: &str,
    host: &str,
    extra: &[String],
    body: &str,
) -> Option<Resp> {
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for h in extra {
        req.push_str(h);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    req.push_str(body);
    let _ = s.write_all(req.as_bytes()).await;
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf).await;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, rest) = text.split_once("\r\n\r\n")?;
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
        dechunk(rest)
    } else {
        rest.to_string()
    };
    Some(Resp {
        status,
        headers,
        body,
    })
}
