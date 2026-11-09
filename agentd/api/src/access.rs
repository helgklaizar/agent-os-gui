//! Access: token in the URL, TCP rate limit, `Authorization: Bearer`, verification through the `Authenticator` port, then the route
//! from the table (`table.rs`): scope, "local only", the token's method list (`allowlist`).
//! Everything except `GET /v1/health` requires a token before routing: an unknown path without a token is `401`, not `404`.

use std::net::IpAddr;
use std::sync::Arc;

use agentd_core::{AuthFailure, Authenticator, Principal, Scope};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::error::ApiError;
use crate::layers::WS_PATH;
use crate::peer::{PeerInfo, Transport};
use crate::state::ApiState;
use crate::table::{self, Lookup, Route};

/// Route of the permission-request decision (template from the route table).
const DECISION_ROUTE: &str = "/permission-requests/{id}/decision";

/// Access denial: what to return over HTTP and what to do with a WS.
pub(crate) enum Denied {
    /// HTTP response as is (for WS it is also an HTTP response without an upgrade).
    Reply(ApiError),
    /// Token presented but not accepted: over HTTP this response, over WS an upgrade and close `4401`.
    Unauthorized(ApiError),
}

/// Route found in the table: put into request and response extensions (metrics take the template from it).
#[derive(Debug, Clone, Copy)]
pub(crate) struct MatchedRoute(pub &'static Route);

/// `user_scope` label for the log (not the token itself).
#[derive(Debug, Clone, Copy)]
pub(crate) struct UserScope(pub &'static str);

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| char::from(c).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (
                b.get(i + 1).copied().and_then(hex),
                b.get(i + 2).copied().and_then(hex),
            )
        {
            out.push(u8::try_from(h * 16 + l).unwrap_or(b'%'));
            i += 3;
        } else {
            out.push(if b[i] == b'+' { b' ' } else { b[i] });
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether the query string has a `token` or `access_token` parameter (name after percent-decoding, case-insensitive).
pub(crate) fn token_in_query(query: Option<&str>) -> bool {
    query.is_some_and(|q| {
        q.split('&').any(|pair| {
            let name =
                percent_decode(pair.split_once('=').map_or(pair, |(k, _)| k)).to_ascii_lowercase();
            name == "token" || name == "access_token"
        })
    })
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let Some(v) = headers.get(header::AUTHORIZATION) else {
        // A browser WS cannot set `Authorization`: a token in `Sec-WebSocket-Protocol` is accepted only on upgrade.
        return headers
            .get(header::UPGRADE)
            .and_then(|u| u.to_str().ok())
            .filter(|u| u.eq_ignore_ascii_case("websocket"))
            .and_then(|_| crate::web::ws_protocol_token(headers));
    };
    let v = v.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

fn parse_ip(s: &str) -> Option<IpAddr> {
    let s = s.trim();
    s.parse::<IpAddr>()
        .ok()
        .or_else(|| s.parse::<std::net::SocketAddr>().ok().map(|a| a.ip()))
}

/// Client address for the rate-limit bucket: `X-Forwarded-For` right to left, skipping trusted proxies. If the peer itself
/// is not a trusted proxy, the header cannot be trusted and the connection address is used.
pub(crate) fn client_key(
    peer: &PeerInfo,
    headers: &HeaderMap,
    trusted: &[IpAddr],
) -> Option<IpAddr> {
    let remote = peer.remote?.ip();
    if !trusted.contains(&remote) {
        return Some(remote);
    }
    let chain: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .collect();
    for entry in chain.iter().rev() {
        match parse_ip(entry) {
            Some(ip) if trusted.contains(&ip) => {}
            Some(ip) => return Some(ip),
            None => return Some(remote),
        }
    }
    Some(remote)
}

fn token_in_url() -> ApiError {
    ApiError::unauthorized().details(json!({ "reason": "token_in_url" }))
}

/// Determines who came in. Order: token in the URL, TCP limit, token presence, verification. A failed TCP verification
/// is recorded in the limit.
pub(crate) async fn identify(
    state: &ApiState,
    auth: &Arc<dyn Authenticator>,
    peer: &PeerInfo,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Result<Principal, Denied> {
    if token_in_query(query) {
        tracing::warn!(
            scope = "auth",
            event = "token_in_url",
            "token in URL rejected"
        );
        return Err(Denied::Unauthorized(token_in_url()));
    }
    let key = (peer.transport == Transport::Tcp)
        .then(|| client_key(peer, headers, &state.policy.trusted_proxies))
        .flatten();
    if let Some(ip) = key {
        let verdict = state
            .limiter
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .check(ip, state.clock.now_ms());
        if let Err(secs) = verdict {
            tracing::warn!(
                scope = "auth",
                event = "rate_limited",
                retry_after = secs,
                "failed-attempt limit"
            );
            return Err(Denied::Reply(
                ApiError::new(429, "rate_limited", "Too many failed attempts").retry_after(secs),
            ));
        }
    }
    let Some(token) = bearer(headers) else {
        return Err(Denied::Reply(ApiError::unauthorized()));
    };
    match auth.authenticate(token).await {
        Ok(p) => Ok(p),
        Err(AuthFailure::Invalid) => {
            if let Some(ip) = key {
                state
                    .limiter
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_failure(ip, state.clock.now_ms());
            }
            Err(Denied::Unauthorized(ApiError::unauthorized()))
        }
        Err(AuthFailure::Unavailable) => Err(Denied::Reply(ApiError::core_down(
            "Token store unavailable",
        ))),
    }
}

fn is_health(req: &Request) -> bool {
    req.uri().path() == "/v1/health" && matches!(*req.method(), Method::GET | Method::HEAD)
}

/// Settings pages that a web token of scope `decide` edits over TCP (W-100): without secrets, provider addresses,
/// permissions, tunnels and connections; those stay local (`admin`).
const WEB_SETTINGS_PAGES: [&str; 5] = ["account", "personas", "schedule", "memory", "indicators"];

/// A `decide` web token edits one of the safe settings pages: `PATCH /settings/{page}`; `rel` is the path without `/v1`.
pub(crate) fn web_settings_grant(route: &Route, principal: &Principal, rel: &str) -> bool {
    principal.can_decide()
        && route.method == "PATCH"
        && (route.template == "/settings/{page}" || route.template == "/settings/indicators")
        && rel
            .strip_prefix("/settings/")
            .is_some_and(|page| WEB_SETTINGS_PAGES.contains(&page))
}

/// Route checks for the resolved principal: "local only", scope, method list; `grant` is a separate
/// web-token permission for this request (`web_settings_grant`).
pub(crate) fn authorize(
    route: &Route,
    principal: &Principal,
    transport: Transport,
    grant: bool,
    rel: &str,
) -> Result<(), ApiError> {
    // The only exception to "decisions are local-only and `admin`-only": a `decide` web token decides
    // permission requests (ADR-0010 item 4); the other `admin` and `local_only` routes stay closed to it.
    let web_decision = grant
        || (principal.can_decide()
            && principal.scope >= Scope::Chat
            && route.method == "POST"
            && route.template == DECISION_ROUTE);
    if route.local_only && transport == Transport::Tcp && !web_decision {
        return Err(ApiError::insufficient_scope(
            "local_only",
            route.scope.as_str(),
        ));
    }
    // Scope first: the method list cannot grant more than the token's scope allows.
    if !principal.scope.satisfies(route.scope) && !web_decision {
        return Err(ApiError::insufficient_scope("scope", route.scope.as_str()));
    }
    if let Some(list) = &principal.allowlist {
        // A list entry is a route template (`POST /sessions/{id}/messages`) or a concrete path (`POST /sessions/3/messages`):
        // this binds an agent token to its own session.
        let key = format!("{} {}", route.method, route.template);
        let exact = format!("{} {}", route.method, rel);
        if !list.contains(&key) && !list.contains(&exact) {
            tracing::warn!(
                scope = "auth",
                event = "forbidden_method",
                token_id = principal.token_id,
                kind = principal.kind.as_str(),
                route = route.template,
                "method not in token list"
            );
            return Err(ApiError::forbidden_method());
        }
    }
    Ok(())
}

/// Access layer: before routing. `GET /v1/health` is anonymous, WS is checked by its own handler (close codes).
pub(crate) async fn guard(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(peer): ConnectInfo<PeerInfo>,
    mut req: Request,
    next: Next,
) -> Response {
    if req.uri().path() == WS_PATH {
        return next.run(req).await;
    }
    // Browser client statics are anonymous: they are code, not data (ADR-0010 item 3).
    if state.policy.web_root.is_some() && crate::web::is_static(req.method(), req.uri().path()) {
        let inm = req
            .headers()
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        return crate::web::serve(&state, req.method(), req.uri().path(), inm.as_deref()).await;
    }
    // A token in the URL is rejected on every route, including anonymous `health`: the value has already leaked into the address.
    if token_in_query(req.uri().query()) {
        tracing::warn!(
            scope = "auth",
            event = "token_in_url",
            "token in URL rejected"
        );
        return token_in_url().into_response();
    }
    if is_health(&req) {
        return next.run(req).await;
    }
    let Some(services) = state.services() else {
        return state.not_ready().into_response();
    };
    let principal = match identify(
        &state,
        &services.auth,
        &peer,
        req.headers(),
        req.uri().query(),
    )
    .await
    {
        Ok(p) => p,
        Err(Denied::Reply(e) | Denied::Unauthorized(e)) => return e.into_response(),
    };
    let scope_label = principal.scope.as_str();
    let Some(rel) = req.uri().path().strip_prefix("/v1") else {
        return ApiError::not_found().into_response();
    };
    let route = match table::lookup(req.method().as_str(), rel) {
        Lookup::NotFound => return ApiError::not_found().into_response(),
        Lookup::MethodNotAllowed(allow) => {
            return ApiError::method_not_allowed(&allow).into_response();
        }
        Lookup::Found(r) => r,
    };
    let grant = web_settings_grant(route, &principal, rel);
    if let Err(e) = authorize(route, &principal, peer.transport, grant, rel) {
        let mut resp = e.into_response();
        resp.extensions_mut().insert(MatchedRoute(route));
        resp.extensions_mut().insert(UserScope(scope_label));
        return resp;
    }
    req.extensions_mut().insert(principal);
    req.extensions_mut().insert(MatchedRoute(route));
    let mut resp = next.run(req).await;
    resp.extensions_mut().insert(MatchedRoute(route));
    resp.extensions_mut().insert(UserScope(scope_label));
    resp
}

#[cfg(test)]
mod tests {
    use super::{percent_decode, token_in_query};

    #[test]
    fn detects_token_names_in_any_encoding() {
        assert!(token_in_query(Some("token=x")));
        assert!(token_in_query(Some("a=1&access_token=x")));
        assert!(token_in_query(Some("%74oken=x")));
        assert!(token_in_query(Some("access%5Ftoken=x")));
        assert!(token_in_query(Some("TOKEN=x")));
        assert!(token_in_query(Some("token")));
        assert!(!token_in_query(Some("x=token&y=access_token")));
        assert!(!token_in_query(Some("tokens=1")));
        assert!(!token_in_query(None));
    }

    #[test]
    fn percent_decode_keeps_broken_sequences() {
        assert_eq!(percent_decode("%74%6Fken"), "token");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
    }
}

#[cfg(test)]
mod decide_tests {
    use super::*;
    use agentd_core::TokenKind;

    fn who(kind: TokenKind, scope: Scope) -> Principal {
        Principal {
            token_id: 1,
            kind,
            scope,
            allowlist: None,
            expires_at: None,
        }
    }

    fn route(method: &str, path: &str) -> &'static Route {
        match table::lookup(method, path) {
            Lookup::Found(r) => r,
            _ => panic!("no such route: {method} {path}"),
        }
    }

    const DECISION: (&str, &str) = ("POST", "/permission-requests/7/decision");

    #[test]
    fn web_token_decides_over_tcp() {
        let w = who(TokenKind::Web, Scope::Decide);
        let r = route(DECISION.0, DECISION.1);
        assert!(authorize(r, &w, Transport::Tcp, false, "").is_ok());
        assert!(authorize(r, &w, Transport::Unix, false, "").is_ok());
    }

    #[test]
    fn plain_web_and_external_cannot_decide() {
        let r = route(DECISION.0, DECISION.1);
        for w in [
            who(TokenKind::Web, Scope::Chat),
            who(TokenKind::External, Scope::Chat),
        ] {
            assert!(authorize(r, &w, Transport::Tcp, false, "").is_err());
            assert!(authorize(r, &w, Transport::Unix, false, "").is_err());
        }
    }

    #[test]
    fn decide_scope_is_only_for_web_kind() {
        let r = route(DECISION.0, DECISION.1);
        for kind in [TokenKind::Agent, TokenKind::Module, TokenKind::External] {
            let w = who(kind, Scope::Decide);
            assert!(
                authorize(r, &w, Transport::Tcp, false, "").is_err(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn web_decide_opens_nothing_else_admin_or_local() {
        let w = who(TokenKind::Web, Scope::Decide);
        for (m, p) in [
            ("PUT", "/permissions/global/0/shell"),
            ("POST", "/tokens"),
            ("DELETE", "/tokens/3"),
            ("POST", "/accounts"),
        ] {
            assert!(
                authorize(route(m, p), &w, Transport::Tcp, false, p).is_err(),
                "{m} {p}"
            );
        }
        // Regular `chat` routes are available: `decide` includes `chat`.
        assert!(
            authorize(
                route("POST", "/sessions/1/messages"),
                &w,
                Transport::Tcp,
                false,
                "/sessions/1/messages"
            )
            .is_ok()
        );
        assert!(
            authorize(
                route("GET", "/sessions"),
                &w,
                Transport::Tcp,
                false,
                "/sessions"
            )
            .is_ok()
        );
    }

    #[test]
    fn web_decide_patches_only_safe_settings_pages() {
        let w = who(TokenKind::Web, Scope::Decide);
        for page in ["account", "personas", "schedule", "memory", "indicators"] {
            let rel = format!("/settings/{page}");
            let r = route("PATCH", &rel);
            assert!(web_settings_grant(r, &w, &rel), "{page}");
            assert!(authorize(r, &w, Transport::Tcp, true, "").is_ok(), "{page}");
        }
        for page in [
            "models",
            "connections",
            "permissions",
            "security",
            "tunnels",
            "modules",
            "about",
        ] {
            let rel = format!("/settings/{page}");
            assert!(
                !web_settings_grant(route("PATCH", &rel), &w, &rel),
                "{page}"
            );
        }
    }

    #[test]
    fn settings_grant_needs_web_decide_and_patch() {
        let rel = "/settings/account";
        let r = route("PATCH", rel);
        for p in [
            who(TokenKind::Web, Scope::Chat),
            who(TokenKind::External, Scope::Chat),
            who(TokenKind::External, Scope::Decide),
            who(TokenKind::Agent, Scope::Chat),
        ] {
            assert!(!web_settings_grant(r, &p, rel), "{:?}", p.kind);
            assert!(
                authorize(r, &p, Transport::Tcp, false, "").is_err(),
                "{:?}",
                p.kind
            );
        }
        let w = who(TokenKind::Web, Scope::Decide);
        assert!(!web_settings_grant(route("GET", rel), &w, rel));
        // The rest of `admin` is still closed.
        assert!(authorize(route("POST", "/personas"), &w, Transport::Tcp, false, "").is_err());
    }
    fn admin() -> Principal {
        who(TokenKind::Gui, Scope::Admin)
    }

    fn reason(e: &ApiError) -> String {
        e.details["reason"].as_str().unwrap_or("").to_string()
    }

    /// Mutant `==` -> `!=` in the transport check: `local_only` is closed over TCP and open over the unix socket.
    #[test]
    fn local_only_route_is_closed_over_tcp_open_over_unix() {
        for (m, p) in [("GET", "/tokens"), ("POST", "/logout"), ("GET", "/secrets")] {
            let r = route(m, p);
            assert!(r.local_only, "{m} {p}");
            let e = authorize(r, &admin(), Transport::Tcp, false, p).unwrap_err();
            assert_eq!(e.code, "insufficient_scope", "{m} {p}");
            assert_eq!(reason(&e), "local_only", "{m} {p}");
            assert!(
                authorize(r, &admin(), Transport::Unix, false, p).is_ok(),
                "{m} {p}"
            );
        }
        // A non-local admin route is open over TCP.
        let r = route("GET", "/sessions");
        assert!(!r.local_only);
        assert!(authorize(r, &admin(), Transport::Tcp, false, "/sessions").is_ok());
    }

    #[test]
    fn scope_denial_names_scope_reason() {
        let r = route("POST", "/sessions/1/messages");
        let e = authorize(
            r,
            &who(TokenKind::External, Scope::Read),
            Transport::Unix,
            false,
            "/sessions/1/messages",
        )
        .unwrap_err();
        assert_eq!(reason(&e), "scope");
    }

    fn with_list(list: &[&str]) -> Principal {
        let mut p = who(TokenKind::Agent, Scope::Chat);
        p.allowlist = Some(list.iter().map(|s| (*s).to_string()).collect());
        p
    }

    /// Mutant "removed `!`" in the method-list check: a list entry is a template or a concrete path.
    #[test]
    fn allowlist_matches_template_or_exact_path() {
        let rel = "/sessions/3/messages";
        let r = route("POST", rel);
        let ok = |list: &[&str], rel: &str| {
            authorize(r, &with_list(list), Transport::Unix, false, rel).is_ok()
        };
        assert!(ok(&["POST /sessions/{id}/messages"], rel), "template");
        assert!(ok(&["POST /sessions/3/messages"], rel), "path");
        assert!(
            ok(
                &["POST /sessions/{id}/messages", "POST /sessions/3/messages"],
                rel
            ),
            "both"
        );
        assert!(!ok(&[], rel), "empty list");
        assert!(!ok(&["GET /sessions"], rel), "foreign entry");
        assert!(!ok(&["POST /sessions/4/messages"], rel), "foreign session");
        let e = authorize(
            r,
            &with_list(&["GET /sessions"]),
            Transport::Unix,
            false,
            rel,
        )
        .unwrap_err();
        assert_eq!(e.code, "forbidden_method");
        // Without a restriction list there are no restrictions.
        assert!(
            authorize(
                r,
                &who(TokenKind::Agent, Scope::Chat),
                Transport::Unix,
                false,
                rel
            )
            .is_ok()
        );
    }
}
