//! `/triggers`, `POST /modules/{module_id}/token`, `GET /metrics` against `openapi-draft.yaml` and `api.md`.

use fake_agentd::{Kind, Scope, TokenSpec};
use serde_json::{Value, json};

use crate::openapi::Spec;
use crate::rawhttp::{Req, Resp, send};
use crate::{assert_clean, mock, status_documented, token};

async fn first_session(d: &fake_agentd::FakeAgentd) -> i64 {
    let r = send(
        d.socket_path(),
        Req::new("GET", "/v1/sessions").token(d.admin_token()),
    )
    .await;
    r.json()[0]["id"].as_i64().unwrap_or(-1)
}

async fn call(d: &fake_agentd::FakeAgentd, method: &str, path: &str, body: Option<Value>) -> Resp {
    let mut req = Req::new(method, path).token(d.admin_token());
    if let Some(b) = body {
        req = req.json(b);
    }
    send(d.socket_path(), req).await
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_triggers_crud_matches_schema() {
    let d = mock().await;
    let spec = Spec::load();
    let sid = first_session(&d).await;
    assert!(sid > 0);

    let made = call(
        &d,
        "POST",
        "/v1/triggers",
        Some(json!({"source": "tg", "match": {"from": "Bob"}, "session_id": sid})),
    )
    .await;
    assert_eq!(made.status, 201, "{}", made.json());
    assert_clean(
        "POST /triggers",
        spec.validate(spec.schema("Trigger"), &made.json(), "Trigger"),
    );
    assert!(status_documented(&spec, "POST", "/triggers", 201));
    assert_eq!(made.json()["enabled"], true, "enabled by default");
    let id = made.json()["id"].as_i64().unwrap_or(-1);

    let list = call(&d, "GET", "/v1/triggers", None).await;
    assert_eq!(list.status, 200);
    let items = list.json();
    assert_eq!(items.as_array().map(Vec::len), Some(1));
    for (i, t) in items.as_array().into_iter().flatten().enumerate() {
        assert_clean(
            "GET /triggers",
            spec.validate(spec.schema("Trigger"), t, &format!("Trigger[{i}]")),
        );
    }

    let off = call(
        &d,
        "PATCH",
        &format!("/v1/triggers/{id}"),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(off.status, 200, "{}", off.json());
    assert_eq!(off.json()["enabled"], false);
    assert_eq!(off.json()["source"], "tg", "other fields kept");
    assert_clean(
        "PATCH /triggers/{id}",
        spec.validate(spec.schema("Trigger"), &off.json(), "Trigger"),
    );

    let gone = call(&d, "DELETE", &format!("/v1/triggers/{id}"), None).await;
    assert_eq!(gone.status, 204);
    assert!(gone.body.is_empty());
    assert!(status_documented(&spec, "DELETE", "/triggers/{id}", 204));
    let again = call(&d, "DELETE", &format!("/v1/triggers/{id}"), None).await;
    assert_eq!(again.status, 404);
    assert_eq!(again.json()["error"]["code"], "not_found");
    let list = call(&d, "GET", "/v1/triggers", None).await;
    assert_eq!(list.json(), json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_triggers_reject_bad_input_and_foreign_scope() {
    let d = mock().await;
    let sid = first_session(&d).await;
    let bad_source = call(
        &d,
        "POST",
        "/v1/triggers",
        Some(json!({"source": "Not A Channel", "match": {}, "session_id": sid})),
    )
    .await;
    assert_eq!(bad_source.status, 422, "{}", bad_source.json());
    assert_eq!(bad_source.json()["error"]["code"], "validation_failed");
    let no_session = call(
        &d,
        "POST",
        "/v1/triggers",
        Some(json!({"source": "tg", "match": {}, "session_id": 999_999})),
    )
    .await;
    assert_eq!(no_session.status, 404, "{}", no_session.json());
    let missing = call(
        &d,
        "PATCH",
        "/v1/triggers/999999",
        Some(json!({"enabled": true})),
    )
    .await;
    assert_eq!(missing.status, 404);

    let read = token(&d, TokenSpec::new(Scope::Read, Kind::Test));
    let list = send(
        d.socket_path(),
        Req::new("GET", "/v1/triggers").token(&read),
    )
    .await;
    assert_eq!(list.status, 200, "read can read the list");
    let create = send(
        d.socket_path(),
        Req::new("POST", "/v1/triggers")
            .token(&read)
            .json(json!({"source": "tg", "match": {}, "session_id": sid})),
    )
    .await;
    assert_eq!(create.status, 403, "read does not create triggers");
    assert_eq!(create.json()["error"]["code"], "insufficient_scope");
    let anon = send(d.socket_path(), Req::new("GET", "/v1/triggers")).await;
    assert_eq!(anon.status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_module_token_is_issued_once_for_enabled_module_only() {
    let d = mock().await;
    let spec = Spec::load();
    let none = call(&d, "POST", "/v1/modules/weather-card/token", None).await;
    assert_eq!(none.status, 404, "module not installed: {}", none.json());
    assert!(status_documented(
        &spec,
        "POST",
        "/modules/{module_id}/token",
        404
    ));

    let inst = call(
        &d,
        "POST",
        "/v1/modules",
        Some(json!({"catalog_id": "weather-card"})),
    )
    .await;
    assert_eq!(inst.status, 201, "{}", inst.json());
    let issued = call(&d, "POST", "/v1/modules/weather-card/token", None).await;
    assert_eq!(issued.status, 201, "{}", issued.json());
    let body = issued.json();
    let value = body["token"].as_str().unwrap_or("").to_string();
    assert!(value.len() >= 32, "token value issued: {body}");
    assert!(body["expires_at"].as_i64().is_some_and(|t| t > 0), "{body}");
    assert!(
        d.stored_token_hashes().iter().all(|h| *h != value),
        "the store does not hold the value itself"
    );

    // panel permissions are only the manifest methods (`GET /quotas`), everything else is forbidden by the allowlist
    let ok = send(d.socket_path(), Req::new("GET", "/v1/quotas").token(&value)).await;
    assert_eq!(
        ok.status,
        200,
        "a manifest method is allowed: {}",
        ok.json()
    );
    let other = send(
        d.socket_path(),
        Req::new("GET", "/v1/sessions").token(&value),
    )
    .await;
    assert_eq!(
        other.status,
        403,
        "a method outside the manifest: {}",
        other.json()
    );
    assert_eq!(other.json()["error"]["code"], "forbidden_method");
    let adm = send(d.socket_path(), Req::new("GET", "/v1/tokens").token(&value)).await;
    assert_eq!(adm.status, 403, "scope module is not admin");

    let off = call(
        &d,
        "PATCH",
        "/v1/modules/weather-card",
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(off.status, 200, "{}", off.json());
    let disabled = call(&d, "POST", "/v1/modules/weather-card/token", None).await;
    assert_eq!(
        disabled.status,
        409,
        "no token is issued for a disabled module: {}",
        disabled.json()
    );
    assert!(status_documented(
        &spec,
        "POST",
        "/modules/{module_id}/token",
        409
    ));

    let read = token(&d, TokenSpec::new(Scope::Read, Kind::Test));
    let denied = send(
        d.socket_path(),
        Req::new("POST", "/v1/modules/weather-card/token").token(&read),
    )
    .await;
    assert_eq!(denied.status, 403, "issuing is admin only");
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_metrics_count_routes_without_user_data() {
    let d = mock().await;
    for _ in 0..3 {
        let r = call(&d, "GET", "/v1/sessions", None).await;
        assert_eq!(r.status, 200);
    }
    let _ = call(&d, "GET", "/v1/sessions/1", None).await;
    let m = call(&d, "GET", "/v1/metrics", None).await;
    assert_eq!(m.status, 200, "{}", m.json());
    assert!(
        m.header("content-type")
            .is_some_and(|c| c.starts_with("application/json")),
        "{:?}",
        m.header("content-type")
    );
    let body = m.json();
    let rows = body["http_requests_total"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let sessions = rows
        .iter()
        .find(|r| r["route"] == "/sessions" && r["status"] == 200)
        .unwrap_or_else(|| panic!("no /sessions 200 line: {body}"));
    assert!(
        sessions["count"].as_i64().is_some_and(|n| n >= 3),
        "{sessions}"
    );
    assert!(
        rows.iter().any(|r| r["route"] == "/sessions/{id}"),
        "the route is a template, not the actual path: {body}"
    );
    assert!(body["ws_connections"].is_number(), "{body}");
    let text = body.to_string();
    assert!(
        !text.contains(d.admin_token()),
        "token value in the metrics"
    );
    assert!(
        !text.contains("/sessions/1\""),
        "actual path in the metrics"
    );

    let read = token(&d, TokenSpec::new(Scope::Read, Kind::Test));
    let denied = send(d.socket_path(), Req::new("GET", "/v1/metrics").token(&read)).await;
    assert_eq!(denied.status, 403, "metrics are admin");
}
