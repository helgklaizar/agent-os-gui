//! A1-05 (rest): scope and `forbidden_method` per the route table, source: `openapi-draft.yaml`.

mod common;

use std::time::Duration;

use common::{
    TOKEN_AGENT, TOKEN_CHAT, TOKEN_CHAT_ONLY, TOKEN_MODULE, TOKEN_READ, concrete, config,
    openapi_routes, start, tcp_req, unix_req,
};

fn token_for(scope: &str) -> &'static str {
    match scope {
        "read" => TOKEN_READ,
        "chat" => TOKEN_CHAT_ONLY,
        _ => TOKEN_CHAT,
    }
}

fn code(r: &common::Resp) -> String {
    r.json()["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

async fn tcp_server(dir: &std::path::Path) -> (agentd_api::Server, std::net::SocketAddr) {
    let mut cfg = config(dir);
    cfg.tcp_enabled = true;
    let server = start(cfg).await;
    for _ in 0..100 {
        if let Some(a) = server.tcp().local_addr() {
            return (server, a);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("TCP did not come up");
}

#[test]
fn openapi_has_routes_with_scopes() {
    let routes = openapi_routes();
    assert!(routes.len() >= 80, "routes parsed: {}", routes.len());
    assert!(routes.iter().any(|r| r.local_only));
    assert!(
        routes
            .iter()
            .filter(|r| r.anonymous)
            .all(|r| r.path == "/health")
    );
}

#[tokio::test]
async fn scope_enforced_per_route() {
    let dir = tempfile::tempdir().unwrap();
    let server = start(config(dir.path())).await;
    let mut checked = 0;
    for r in openapi_routes() {
        let Some(need) = r.scope.as_deref() else {
            continue;
        };
        let route = format!("/v1{}", concrete(&r.path));
        let lower: &[&str] = match need {
            "admin" => &["read", "chat"],
            "chat" => &["read"],
            _ => &[],
        };
        for have in lower {
            let resp = unix_req(
                server.socket_path(),
                &r.method,
                &route,
                Some(token_for(have)),
            )
            .await;
            assert_eq!(resp.status, 403, "{} {route} with scope {have}", r.method);
            assert_eq!(code(&resp), "insufficient_scope", "{} {route}", r.method);
        }
        let ok = unix_req(
            server.socket_path(),
            &r.method,
            &route,
            Some(token_for(need)),
        )
        .await;
        assert!(
            ![401, 403, 405].contains(&ok.status),
            "{} {route} with sufficient scope {need}: {}",
            r.method,
            ok.status
        );
        checked += 1;
    }
    assert!(checked >= 79, "routes checked: {checked}");
    server.shutdown().await;
}

#[tokio::test]
async fn local_only_routes_rejected_over_tcp() {
    let dir = tempfile::tempdir().unwrap();
    let (server, addr) = tcp_server(dir.path()).await;
    let mut local = 0;
    let mut remote = 0;
    for r in openapi_routes() {
        let Some(need) = r.scope.as_deref() else {
            continue;
        };
        let route = format!("/v1{}", concrete(&r.path));
        // Admin token: a local-only route is rejected over TCP regardless of the token.
        let resp = tcp_req(addr, &r.method, &route, Some(TOKEN_CHAT)).await;
        if r.local_only {
            assert_eq!(resp.status, 403, "{} {route} over TCP", r.method);
            assert_eq!(code(&resp), "insufficient_scope", "{} {route}", r.method);
            let unix = unix_req(server.socket_path(), &r.method, &route, Some(TOKEN_CHAT)).await;
            assert_ne!(unix.status, 403, "{} {route} over the socket", r.method);
            local += 1;
        } else {
            assert!(
                ![401, 403, 405].contains(&resp.status),
                "{} {route} ({need}) over TCP: {}",
                r.method,
                resp.status
            );
            remote += 1;
        }
    }
    assert!(
        local == 13 && remote == 68,
        "local {local}, remote {remote}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn no_anonymous_route_except_health() {
    let dir = tempfile::tempdir().unwrap();
    let server = start(config(dir.path())).await;
    for r in openapi_routes() {
        let route = format!("/v1{}", concrete(&r.path));
        let resp = unix_req(server.socket_path(), &r.method, &route, None).await;
        if r.anonymous {
            assert_eq!(resp.status, 200, "{} {route}", r.method);
        } else {
            assert_eq!(resp.status, 401, "{} {route} without a token", r.method);
            assert_eq!(code(&resp), "unauthorized");
        }
    }
    for p in ["/v1/nope", "/v1/sessions/1/nope", "/v1/_test/advance", "/"] {
        let resp = unix_req(server.socket_path(), "GET", p, None).await;
        assert_eq!(resp.status, 401, "{p}");
    }
    server.shutdown().await;
}

#[tokio::test]
async fn unknown_route_and_method_with_token() {
    let dir = tempfile::tempdir().unwrap();
    let server = start(config(dir.path())).await;
    let p = server.socket_path();
    let nf = unix_req(p, "GET", "/v1/no-such-route", Some(TOKEN_CHAT)).await;
    assert_eq!((nf.status, code(&nf).as_str()), (404, "not_found"));
    let nf = unix_req(p, "GET", "/no-prefix", Some(TOKEN_CHAT)).await;
    assert_eq!(nf.status, 404);
    let bad = unix_req(p, "PUT", "/v1/sessions", Some(TOKEN_CHAT)).await;
    assert_eq!(
        (bad.status, code(&bad).as_str()),
        (405, "method_not_allowed")
    );
    let allow = bad.header("allow").expect("Allow header");
    assert!(allow.contains("GET") && allow.contains("POST"), "{allow}");
    // All table routes are implemented; the `not_implemented` response is checked by the unit test `routes::tests`.
    server.shutdown().await;
}

#[tokio::test]
async fn module_token_allowlist() {
    let dir = tempfile::tempdir().unwrap();
    let server = start(config(dir.path())).await;
    let p = server.socket_path();
    // In the list: passes through (no handler route yet, so not 403).
    let ok = unix_req(p, "GET", "/v1/sessions", Some(TOKEN_MODULE)).await;
    assert_ne!(ok.status, 403, "GET /sessions in the list");
    // Substitutions are matched by template: the actual id does not affect the decision.
    let denied = unix_req(p, "POST", "/v1/sessions/42/messages", Some(TOKEN_MODULE)).await;
    assert_eq!(denied.status, 403);
    assert_eq!(code(&denied), "forbidden_method");
    let denied = unix_req(p, "GET", "/v1/sessions/42", Some(TOKEN_MODULE)).await;
    assert_eq!(code(&denied), "forbidden_method");
    // Insufficient scope: scope decides (insufficient_scope), not the list.
    let scope = unix_req(p, "GET", "/v1/tokens", Some(TOKEN_MODULE)).await;
    assert_eq!(code(&scope), "insufficient_scope");
    // A token without a list (not agent/module) has no method restrictions.
    let free = unix_req(p, "POST", "/v1/sessions/42/messages", Some(TOKEN_CHAT_ONLY)).await;
    assert_ne!(free.status, 403);
    server.shutdown().await;
}

#[tokio::test]
async fn agent_token_scope_chat() {
    let dir = tempfile::tempdir().unwrap();
    let server = start(config(dir.path())).await;
    let p = server.socket_path();
    // Even if an admin route is in the allowlist, an agent (scope not above chat) cannot decide permissions.
    for (m, route) in [
        ("POST", "/v1/permission-requests/7/decision"),
        ("PUT", "/v1/permissions/global/0/shell"),
        ("POST", "/v1/logout"),
        ("POST", "/v1/tokens"),
    ] {
        let resp = unix_req(p, m, route, Some(TOKEN_AGENT)).await;
        assert_eq!(resp.status, 403, "{m} {route}");
        assert_eq!(code(&resp), "insufficient_scope", "{m} {route}");
    }
    let ok = unix_req(p, "GET", "/v1/sessions", Some(TOKEN_AGENT)).await;
    assert_ne!(ok.status, 403);
    server.shutdown().await;
}
