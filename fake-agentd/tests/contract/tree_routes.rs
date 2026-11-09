//! Tree routes and responses against `openapi-draft.yaml` (schemas `Folder`, `Session`, `Persona`).

use fake_agentd::tree_http::ROUTES;
use serde_json::json;

use crate::openapi::Spec;
use crate::rawhttp::{Req, send};
use crate::{assert_clean, mock, status_documented};

#[test]
fn contract_tree_routes_match_openapi() {
    let spec = Spec::load();
    assert_eq!(ROUTES.len(), 12, "tree routes in the mock");
    for r in ROUTES {
        assert!(
            spec.op(r.method, r.pattern).is_some(),
            "{} {} is in the mock and missing from openapi-draft.yaml",
            r.method,
            r.pattern
        );
        assert_eq!(
            spec.scope(r.method, r.pattern),
            Some(r.scope.as_str()),
            "x-scope {} {}",
            r.method,
            r.pattern
        );
        assert_eq!(spec.local_only(r.method, r.pattern), r.local_only);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_tree_responses_match_schemas() {
    let d = mock().await;
    let spec = Spec::load();
    let (sock, admin) = (d.socket_path(), d.admin_token().to_string());

    let folders = send(sock, Req::new("GET", "/v1/folders").token(&admin)).await;
    assert_eq!(folders.status, 200);
    for (i, f) in folders.json().as_array().into_iter().flatten().enumerate() {
        assert_clean(
            "GET /folders",
            spec.validate(spec.schema("Folder"), f, &format!("Folder[{i}]")),
        );
    }
    let sessions = send(sock, Req::new("GET", "/v1/sessions").token(&admin)).await;
    assert_eq!(sessions.status, 200);
    let list = sessions.json();
    assert!(list.as_array().is_some_and(|a| !a.is_empty()));
    for (i, s) in list.as_array().into_iter().flatten().enumerate() {
        assert_clean(
            "GET /sessions",
            spec.validate(spec.schema("Session"), s, &format!("Session[{i}]")),
        );
    }
    let personas = send(sock, Req::new("GET", "/v1/personas").token(&admin)).await;
    assert_eq!(personas.status, 200);
    for (i, p) in personas.json().as_array().into_iter().flatten().enumerate() {
        assert_clean(
            "GET /personas",
            spec.validate(spec.schema("Persona"), p, &format!("Persona[{i}]")),
        );
    }
    let created = send(
        sock,
        Req::new("POST", "/v1/folders")
            .token(&admin)
            .json(json!({"name": "K"})),
    )
    .await;
    assert_eq!(created.status, 201);
    assert_clean(
        "POST /folders",
        spec.validate(spec.schema("Folder"), &created.json(), "Folder"),
    );
    assert!(status_documented(&spec, "POST", "/folders", 201));
    let s = send(
        sock,
        Req::new("POST", "/v1/sessions")
            .token(&admin)
            .json(json!({"name": "S"})),
    )
    .await;
    assert_eq!(s.status, 201);
    assert_clean(
        "POST /sessions",
        spec.validate(spec.schema("Session"), &s.json(), "Session"),
    );
    let sid = s.json()["id"].as_i64().unwrap_or(-1);
    let t = send(
        sock,
        Req::new("POST", &format!("/v1/sessions/{sid}/threads"))
            .token(&admin)
            .json(json!({"name": "T"})),
    )
    .await;
    assert_eq!(t.status, 201);
    assert_clean(
        "POST threads",
        spec.validate(spec.schema("Session"), &t.json(), "Session"),
    );
    for (m, p, st) in [
        ("PATCH", "/sessions/{id}", 200),
        ("DELETE", "/sessions/{id}", 204),
        ("POST", "/sessions/{id}/read", 204),
        ("DELETE", "/folders/{id}", 204),
        ("PATCH", "/folders/{id}", 200),
        ("GET", "/sessions/{id}", 200),
    ] {
        assert!(status_documented(&spec, m, p, st), "{m} {p} {st}");
    }
    let bad = send(
        sock,
        Req::new("POST", "/v1/sessions")
            .token(&admin)
            .json(json!({"name": ""})),
    )
    .await;
    assert!(status_documented(&spec, "POST", "/sessions", bad.status));
    let missing = send(sock, Req::new("GET", "/v1/sessions/99999").token(&admin)).await;
    assert!(status_documented(
        &spec,
        "GET",
        "/sessions/{id}",
        missing.status
    ));
    let clash = send(
        sock,
        Req::new("PATCH", "/v1/folders/1")
            .token(&admin)
            .json(json!({"parent_id": 1})),
    )
    .await;
    assert!(
        status_documented(&spec, "PATCH", "/folders/{id}", clash.status),
        "{}",
        clash.status
    );
}
