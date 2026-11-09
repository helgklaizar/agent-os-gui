//! Token check: token in the URL, attempt limit, format, hash, expiry, revocation.

use hyper::HeaderMap;
use hyper::header::AUTHORIZATION;

use crate::reply::ApiErr;
use crate::state::Shared;
use crate::store::{Kind, Scope, StoreError};
use crate::token::is_valid_format;

#[derive(Debug, Clone)]
pub struct Authed {
    pub id: i64,
    pub scope: Scope,
    pub kind: Kind,
    pub allowlist: Option<Vec<String>>,
}

#[derive(Debug)]
pub enum AuthFail {
    /// No header or the scheme is not Bearer.
    Missing,
    TokenInUrl,
    /// Foreign format, hash not in the table, token revoked or expired.
    Invalid,
    RateLimited(u64),
    /// DB failure while reading tokens: the client gets `500`, the error text goes only to the log.
    Store(StoreError),
}

impl AuthFail {
    pub fn into_api(self) -> ApiErr {
        match self {
            AuthFail::Missing => {
                ApiErr::new(401, "unauthorized", "Token required: Authorization: Bearer")
            }
            AuthFail::TokenInUrl => {
                ApiErr::new(401, "unauthorized", "Token in the URL is not accepted")
                    .details(serde_json::json!({ "reason": "token_in_url" }))
            }
            AuthFail::Invalid => {
                ApiErr::new(401, "unauthorized", "Token is invalid, expired or revoked")
            }
            AuthFail::RateLimited(s) => {
                ApiErr::new(429, "rate_limited", "Too many invalid tokens").retry_after(s)
            }
            AuthFail::Store(e) => {
                tracing::error!(error = %e, "token DB unavailable");
                ApiErr::new(500, "internal", "Token store failure")
            }
        }
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |x: u8| char::from(x).to_digit(16);
        match (
            b[i],
            b.get(i + 1).and_then(|c| hex(*c)),
            b.get(i + 2).and_then(|c| hex(*c)),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_lowercase()
}

/// The query string has a `token` or `access_token` parameter (including percent-encoded).
pub fn query_has_token(query: Option<&str>) -> bool {
    query.is_some_and(|q| {
        q.split('&').any(|pair| {
            let key = percent_decode(pair.split('=').next().unwrap_or(""));
            key == "token" || key == "access_token"
        })
    })
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, rest) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(rest.trim())
}

/// Source for the limit: client address; with `X-Forwarded-For` right to left, skipping loopback.
pub fn source_key(peer_ip: Option<std::net::IpAddr>, headers: &HeaderMap) -> String {
    let Some(ip) = peer_ip else {
        return "unix".to_string();
    };
    if ip.is_loopback() {
        let fwd = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let client = fwd.rsplit(',').map(str::trim).find(|a| {
            a.parse::<std::net::IpAddr>()
                .is_ok_and(|p| !p.is_loopback())
        });
        if let Some(c) = client {
            return c.to_string();
        }
    }
    ip.to_string()
}

pub fn authenticate(
    shared: &Shared,
    headers: &HeaderMap,
    query: Option<&str>,
    key: &str,
) -> Result<Authed, AuthFail> {
    if query_has_token(query) {
        return Err(AuthFail::TokenInUrl);
    }
    let now_ms = shared.clock.now_ms();
    let now = shared.clock.now_unix();
    let mut inner = shared.lock();
    inner
        .limiter
        .check(key, now_ms)
        .map_err(AuthFail::RateLimited)?;
    let Some(raw) = bearer(headers) else {
        return Err(AuthFail::Missing);
    };
    let row = if is_valid_format(raw) {
        inner.tokens.lookup(raw).map_err(AuthFail::Store)?
    } else {
        None
    };
    let usable = row.filter(|r| r.revoked_at.is_none() && r.expires_at.is_none_or(|e| e > now));
    match usable {
        Some(r) => {
            if let Err(e) = inner.tokens.touch(r.id, now) {
                tracing::warn!(error = %e, "last_used_at not written");
            }
            Ok(Authed {
                id: r.id,
                scope: r.scope,
                kind: r.kind,
                allowlist: r.allowlist,
            })
        }
        None => {
            inner.limiter.record_failure(key, now_ms);
            Err(AuthFail::Invalid)
        }
    }
}
