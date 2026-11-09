//! Mock errors against the `Error` schema of `openapi-draft.yaml` and the `api.md` section 3 table.

use fake_agentd::{FakeAgentd, Kind, Scope, TokenSpec};
use serde_json::json;

use crate::openapi::Spec;
use crate::rawhttp::{Req, Resp, send};
use crate::yaml;
use crate::{
    KNOWN_CODE_DRIFT, REQUEST_ID, api_error_table, assert_clean, mock, status_documented, token,
};

struct Case {
    label: &'static str,
    method: &'static str,
    route: &'static str,
    status: u16,
    code: &'static str,
    resp: Resp,
}

async fn error_cases(d: &FakeAgentd) -> Vec<Case> {
    let sock = d.socket_path();
    let admin = d.admin_token().to_string();
    let read = token(d, TokenSpec::new(Scope::Read, Kind::Test));
    let module = token(
        d,
        TokenSpec::new(Scope::Chat, Kind::Module).allowlist(&["GET /settings/theme"]),
    );
    let mut out = Vec::new();
    let mut add = |label, method, route, status, code, resp| {
        out.push(Case {
            label,
            method,
            route,
            status,
            code,
            resp,
        })
    };
    let rid = |r: Req<'static>| r.header("X-Request-Id", REQUEST_ID);

    add(
        "no token",
        "GET",
        "/tokens",
        401,
        "unauthorized",
        send(sock, rid(Req::new("GET", "/v1/tokens"))).await,
    );
    add(
        "token in the URL",
        "GET",
        "/settings/theme",
        401,
        "unauthorized",
        send(sock, rid(Req::new("GET", "/v1/settings/theme?token=agt_x"))).await,
    );
    add(
        "read on an admin route",
        "GET",
        "/tokens",
        403,
        "insufficient_scope",
        send(sock, rid(Req::new("GET", "/v1/tokens")).token(&read)).await,
    );
    add(
        "method outside the allowlist",
        "PATCH",
        "/settings/theme",
        403,
        "forbidden_method",
        send(
            sock,
            rid(Req::new("PATCH", "/v1/settings/theme"))
                .token(&module)
                .header("If-Match", "1")
                .json(json!({"theme": "dark"})),
        )
        .await,
    );
    add(
        "no token with this id",
        "DELETE",
        "/tokens/{id}",
        404,
        "not_found",
        send(
            sock,
            rid(Req::new("DELETE", "/v1/tokens/987654")).token(&admin),
        )
        .await,
    );
    add(
        "no route",
        "GET",
        "/nope",
        404,
        "not_found",
        send(sock, rid(Req::new("GET", "/v1/nope")).token(&admin)).await,
    );
    add(
        "stale If-Match",
        "PATCH",
        "/settings/theme",
        412,
        "precondition_failed",
        send(
            sock,
            rid(Req::new("PATCH", "/v1/settings/theme"))
                .token(&admin)
                .header("If-Match", "99")
                .json(json!({"theme": "dark"})),
        )
        .await,
    );
    add(
        "no If-Match",
        "PATCH",
        "/settings/theme",
        428,
        "precondition_required",
        send(
            sock,
            rid(Req::new("PATCH", "/v1/settings/theme"))
                .token(&admin)
                .json(json!({"theme": "dark"})),
        )
        .await,
    );
    add(
        "bad theme value",
        "PATCH",
        "/settings/theme",
        422,
        "validation_failed",
        send(
            sock,
            rid(Req::new("PATCH", "/v1/settings/theme"))
                .token(&admin)
                .header("If-Match", "1")
                .json(json!({"theme": "pink"})),
        )
        .await,
    );
    add(
        "bad token scope",
        "POST",
        "/tokens",
        422,
        "validation_failed",
        send(
            sock,
            rid(Req::new("POST", "/v1/tokens"))
                .token(&admin)
                .json(json!({"name": "x", "scope": "root"})),
        )
        .await,
    );
    add(
        "garbage in If-Match",
        "PATCH",
        "/settings/theme",
        400,
        "bad_request",
        send(
            sock,
            rid(Req::new("PATCH", "/v1/settings/theme"))
                .token(&admin)
                .header("If-Match", "abc")
                .json(json!({"theme": "dark"})),
        )
        .await,
    );
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_error_bodies_match_schema_and_api_table() {
    let d = mock().await;
    let spec = Spec::load();
    let table = api_error_table();
    assert!(
        table.contains(&(401, "unauthorized".to_string())),
        "the api.md section 3 error table was not read: {table:?}"
    );
    let enum_codes: Vec<&str> = spec
        .schema("Error")
        .path(&["properties", "error", "properties", "code", "enum"])
        .map(yaml::Y::strs)
        .unwrap_or_default();
    assert!(
        enum_codes.contains(&"unauthorized"),
        "the code enum was not read"
    );

    for c in error_cases(&d).await {
        let at = format!("«{}» ({} {})", c.label, c.method, c.route);
        assert_eq!(c.resp.status, c.status, "{at}: status");
        let body = c.resp.json();
        let errs = spec
            .validate(spec.schema("Error"), &body, "Error")
            .into_iter()
            .filter(|e| {
                !KNOWN_CODE_DRIFT
                    .iter()
                    .any(|(code, _)| e.contains(&format!("value {code:?} outside enum")))
            })
            .collect();
        assert_clean(&at, errs);
        assert_eq!(body["error"]["code"].as_str(), Some(c.code), "{at}: code");
        assert_eq!(
            body["error"]["request_id"].as_str(),
            Some(REQUEST_ID),
            "{at}: request_id from X-Request-Id"
        );
        assert_eq!(
            c.resp.header("x-request-id"),
            Some(REQUEST_ID),
            "{at}: X-Request-Id header"
        );
        let drift = KNOWN_CODE_DRIFT.iter().any(|(code, _)| *code == c.code);
        assert!(
            enum_codes.contains(&c.code) || drift,
            "{at}: code {} is not in the Error enum of openapi-draft.yaml",
            c.code
        );
        assert!(
            table.contains(&(c.status, c.code.to_string())) || drift,
            "{at}: the pair {} {} is not in the api.md section 3 table",
            c.status,
            c.code
        );
        if matches!(c.status, 404 | 412 | 422 | 428) && c.route != "/nope" {
            assert!(
                status_documented(&spec, c.method, c.route, c.status),
                "{at}: status {} is not described in the OpenAPI operation",
                c.status
            );
        }
    }
}
