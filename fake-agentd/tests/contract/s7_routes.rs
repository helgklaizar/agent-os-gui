//! Routes and schemas of the settings pages, secrets, personas, accounts, modules, tunnels and files against
//! `openapi-draft.yaml` (`contract_settings_secrets_match_openapi`).

use fake_agentd::routes::Route;
use fake_agentd::{
    files_http, modules_http, people_http, secrets_http, settings_http, tunnels_http,
};
use serde_json::{Value, json};

use crate::openapi::Spec;
use crate::rawhttp::{Req, Resp, send};
use crate::{assert_clean, mock, status_documented};

fn tables() -> Vec<&'static Route> {
    settings_http::ROUTES
        .iter()
        .chain(secrets_http::ROUTES)
        .chain(people_http::ROUTES)
        .chain(modules_http::ROUTES)
        .chain(tunnels_http::ROUTES)
        .chain(files_http::ROUTES)
        .collect()
}

#[test]
fn contract_settings_secrets_match_openapi() {
    let spec = Spec::load();
    let routes = tables();
    assert_eq!(routes.len(), 26, "settings routes in the mock");
    for r in routes {
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
        assert_eq!(
            spec.local_only(r.method, r.pattern),
            r.local_only,
            "x-local-only {} {}",
            r.method,
            r.pattern
        );
    }
}

pub(crate) struct Ctx<'a> {
    pub(crate) spec: &'a Spec,
    pub(crate) sock: &'a std::path::Path,
    pub(crate) admin: &'a str,
}

impl Ctx<'_> {
    pub(crate) async fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        rev: Option<i64>,
    ) -> Resp {
        let mut req = Req::new(method, path).token(self.admin);
        if let Some(r) = rev {
            req = req.header("If-Match", &format!("\"{r}\""));
        }
        if let Some(b) = body {
            req = req.json(b);
        }
        send(self.sock, req).await
    }

    pub(crate) fn check(&self, what: &str, schema: &str, v: &Value) {
        assert_clean(
            what,
            self.spec.validate(self.spec.schema(schema), v, schema),
        );
    }

    pub(crate) fn documented(&self, method: &str, pattern: &str, got: &Resp) {
        assert!(
            status_documented(self.spec, method, pattern, got.status),
            "{method} {pattern}: status {} is not described in OpenAPI",
            got.status
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_settings_responses_match_schemas() {
    let d = mock().await;
    let spec = Spec::load();
    let c = Ctx {
        spec: &spec,
        sock: d.socket_path(),
        admin: d.admin_token(),
    };
    for page in [
        "account",
        "about",
        "personas",
        "models",
        "connections",
        "permissions",
        "security",
        "schedule",
        "memory",
        "tunnels",
    ] {
        let got = c
            .call("GET", &format!("/v1/settings/{page}"), None, None)
            .await;
        assert_eq!(got.status, 200, "{page}");
        assert!(got.header("etag").is_some(), "ETag {page}");
        c.check(
            &format!("GET /settings/{page}"),
            "SettingsPage",
            &got.json(),
        );
        c.documented("GET", "/settings/{page}", &got);
    }
    let patched = c
        .call(
            "PATCH",
            "/v1/settings/account",
            Some(json!({"name": "A"})),
            Some(0),
        )
        .await;
    assert_eq!(patched.status, 200);
    c.check("PATCH /settings/account", "SettingsPage", &patched.json());
    for (body, rev, status) in [
        (json!({"name": "b"}), Some(9), 412),
        (json!({"name": "b"}), None, 428),
    ] {
        let w = c
            .call("PATCH", "/v1/settings/account", Some(body), rev)
            .await;
        assert_eq!(w.status, status);
        c.documented("PATCH", "/settings/{page}", &w);
    }
    let ro = c
        .call(
            "PATCH",
            "/v1/settings/about",
            Some(json!({"x": 1})),
            Some(0),
        )
        .await;
    assert_eq!(ro.status, 403);
    c.documented("PATCH", "/settings/{page}", &ro);
    let bad = c
        .call(
            "PATCH",
            "/v1/settings/account",
            Some(json!({"language": "de"})),
            Some(1),
        )
        .await;
    assert_eq!(bad.status, 422);
    c.documented("PATCH", "/settings/{page}", &bad);
    let ind = c.call("GET", "/v1/settings/indicators", None, None).await;
    assert_eq!(ind.status, 200);
    c.check("GET /settings/indicators", "IndicatorsPage", &ind.json());
    let (i0, rev) = (ind.json()["items"].clone(), ind.json()["rev"].as_i64());
    let saved = c
        .call(
            "PATCH",
            "/v1/settings/indicators",
            Some(json!({"items": i0})),
            rev,
        )
        .await;
    assert_eq!(saved.status, 200);
    c.check(
        "PATCH /settings/indicators",
        "IndicatorsPage",
        &saved.json(),
    );
    c.documented("PATCH", "/settings/indicators", &saved);
}
