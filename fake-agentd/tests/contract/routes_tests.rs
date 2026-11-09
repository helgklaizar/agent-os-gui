//! Mock routes and successful responses against `openapi-draft.yaml`.

use fake_agentd::routes::ROUTES;
use fake_agentd::{Kind, Scope, TokenSpec};
use serde_json::json;

use crate::openapi::Spec;
use crate::rawhttp::{Req, send};
use crate::{REQUEST_ID, assert_clean, mock, status_documented, token};

#[test]
fn contract_routes_match_openapi() {
    let spec = Spec::load();
    let mut checked = 0;
    for r in ROUTES {
        if r.pattern.starts_with("/_test/") {
            assert!(
                spec.op(r.method, r.pattern).is_none(),
                "service route {} got into the contract",
                r.pattern
            );
            continue;
        }
        let Some(_) = spec.op(r.method, r.pattern) else {
            panic!(
                "{} {} is in the mock and missing from openapi-draft.yaml",
                r.method, r.pattern
            )
        };
        assert_eq!(
            spec.scope(r.method, r.pattern),
            Some(r.scope.as_str()),
            "x-scope {} {}",
            r.method,
            r.pattern
        );
        assert_eq!(
            spec.local_only(r.method, r.pattern),
            r.local_only,
            "x-local-only {} {}",
            r.method,
            r.pattern
        );
        checked += 1;
    }
    assert_eq!(
        checked, 5,
        "contract routes in the mock: theme GET and PATCH, tokens GET, POST and DELETE"
    );
}

#[test]
fn contract_health_is_the_only_anonymous_route() {
    let spec = Spec::load();
    for (method, path, op) in spec.operations() {
        let anonymous = op.get("security").is_some_and(|s| s.as_seq().is_empty());
        assert_eq!(
            anonymous,
            (method.as_str(), path.as_str()) == ("GET", "/health"),
            "{method} {path}: anonymity"
        );
    }
}

// ---------- successful responses ----------

#[tokio::test(flavor = "multi_thread")]
async fn contract_success_responses_match_schemas() {
    let d = mock().await;
    let spec = Spec::load();
    let sock = d.socket_path();
    let admin = d.admin_token().to_string();
    let read = token(&d, TokenSpec::new(Scope::Read, Kind::Test));

    let health = send(
        sock,
        Req::new("GET", "/v1/health").header("X-Request-Id", REQUEST_ID),
    )
    .await;
    assert_eq!(health.status, 200);
    assert_eq!(
        health.header("x-request-id"),
        Some(REQUEST_ID),
        "X-Request-Id is returned"
    );
    assert_clean(
        "GET /health",
        spec.validate(spec.schema("Health"), &health.json(), "Health"),
    );
    assert!(status_documented(&spec, "GET", "/health", 200));

    let theme = send(sock, Req::new("GET", "/v1/settings/theme").token(&read)).await;
    assert_eq!(theme.status, 200);
    assert_clean(
        "GET /settings/theme",
        spec.validate(spec.schema("ThemePage"), &theme.json(), "ThemePage"),
    );
    let rev = theme.json()["rev"].as_i64().unwrap_or(-1);
    assert_eq!(
        theme
            .header("etag")
            .map(|e| e.trim_matches('"').to_string()),
        Some(rev.to_string()),
        "ETag = rev"
    );
    assert!(status_documented(&spec, "GET", "/settings/theme", 200));

    let patched = send(
        sock,
        Req::new("PATCH", "/v1/settings/theme")
            .token(&admin)
            .header("If-Match", &rev.to_string())
            .json(json!({"theme": "dark"})),
    )
    .await;
    assert_eq!(patched.status, 200);
    assert_clean(
        "PATCH /settings/theme",
        spec.validate(spec.schema("ThemePage"), &patched.json(), "ThemePage"),
    );
    assert_eq!(patched.json()["rev"].as_i64(), Some(rev + 1));
    assert!(patched.header("etag").is_some(), "ETag on PATCH");
    assert!(status_documented(&spec, "PATCH", "/settings/theme", 200));

    let issued = send(
        sock,
        Req::new("POST", "/v1/tokens")
            .token(&admin)
            .json(json!({"name": "phone", "scope": "chat", "ttl_days": 30})),
    )
    .await;
    assert_eq!(issued.status, 201);
    assert_clean(
        "POST /tokens",
        spec.validate(spec.schema("TokenIssued"), &issued.json(), "TokenIssued"),
    );
    assert!(status_documented(&spec, "POST", "/tokens", 201));

    let list = send(sock, Req::new("GET", "/v1/tokens").token(&admin)).await;
    assert_eq!(list.status, 200);
    let arr = list.json();
    assert!(
        arr.as_array().is_some_and(|a| a.len() >= 3),
        "admin, read and the issued one: {arr}"
    );
    for (i, t) in arr.as_array().into_iter().flatten().enumerate() {
        assert_clean(
            "GET /tokens",
            spec.validate(spec.schema("Token"), t, &format!("Token[{i}]")),
        );
        assert!(
            t.get("token").is_none(),
            "the list does not return the value"
        );
    }
    assert!(status_documented(&spec, "GET", "/tokens", 200));

    let id = issued.json()["id"].as_i64().unwrap_or(-1);
    let gone = send(
        sock,
        Req::new("DELETE", &format!("/v1/tokens/{id}")).token(&admin),
    )
    .await;
    assert_eq!(gone.status, 204);
    assert!(gone.body.is_empty(), "204 without a body");
    assert!(status_documented(&spec, "DELETE", "/tokens/{id}", 204));
}
