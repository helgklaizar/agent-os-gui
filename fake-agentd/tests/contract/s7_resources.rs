//! Settings resources (personas, accounts, modules, tunnels, files) against the `openapi-draft.yaml` schemas.

use serde_json::json;

use crate::mock;
use crate::openapi::Spec;
use crate::s7_routes::Ctx;

#[tokio::test(flavor = "multi_thread")]
async fn contract_s7_resources_match_schemas() {
    let d = mock().await;
    let spec = Spec::load();
    let c = Ctx {
        spec: &spec,
        sock: d.socket_path(),
        admin: d.admin_token(),
    };
    let acc = c
        .call(
            "POST",
            "/v1/accounts",
            Some(json!({"type": "wa", "label": "w"})),
            None,
        )
        .await;
    assert_eq!(acc.status, 201);
    c.check("POST /accounts", "Account", &acc.json());
    c.documented("POST", "/accounts", &acc);
    let id = acc.json()["id"].as_i64().unwrap_or(-1);
    let list = c.call("GET", "/v1/accounts", None, None).await;
    for a in list.json().as_array().into_iter().flatten() {
        c.check("GET /accounts", "Account", a);
    }
    let flow = c
        .call("POST", &format!("/v1/accounts/{id}/connect"), None, None)
        .await;
    c.check("POST /accounts/{id}/connect", "ConnectFlow", &flow.json());
    c.documented("POST", "/accounts/{id}/connect", &flow);
    let renamed = c
        .call(
            "PATCH",
            &format!("/v1/accounts/{id}"),
            Some(json!({"label": "n"})),
            None,
        )
        .await;
    c.check("PATCH /accounts/{id}", "Account", &renamed.json());
    let p = c
        .call(
            "POST",
            "/v1/personas",
            Some(json!({"name": "P", "account_ids": [id]})),
            None,
        )
        .await;
    assert_eq!(p.status, 201);
    c.check("POST /personas", "Persona", &p.json());
    c.documented("POST", "/personas", &p);
    let pid = p.json()["id"].as_i64().unwrap_or(-1);
    let pp = c
        .call(
            "PATCH",
            &format!("/v1/personas/{pid}"),
            Some(json!({"name": "Q"})),
            None,
        )
        .await;
    c.check("PATCH /personas/{id}", "Persona", &pp.json());
    let t = c
        .call(
            "POST",
            "/v1/tunnels",
            Some(json!({"type": "ssh", "addr": "h:1"})),
            None,
        )
        .await;
    assert_eq!(t.status, 201);
    c.check("POST /tunnels", "Tunnel", &t.json());
    c.documented("POST", "/tunnels", &t);
    let tl = c.call("GET", "/v1/tunnels", None, None).await;
    for x in tl.json().as_array().into_iter().flatten() {
        c.check("GET /tunnels", "Tunnel", x);
    }
    let mods = c.call("GET", "/v1/modules", None, None).await;
    for m in mods.json().as_array().into_iter().flatten() {
        c.check("GET /modules", "Module", m);
    }
    let inst = c
        .call(
            "POST",
            "/v1/modules",
            Some(json!({"url": "https://modules.example.test/quick-btn.json"})),
            None,
        )
        .await;
    assert_eq!(inst.status, 201);
    c.check("POST /modules", "Module", &inst.json());
    c.documented("POST", "/modules", &inst);
    let ask = c
        .call(
            "POST",
            "/v1/modules",
            Some(json!({"url": "https://modules.example.test/ask-x.json"})),
            None,
        )
        .await;
    c.documented("POST", "/modules", &ask);
    let mp = c
        .call(
            "PATCH",
            "/v1/modules/quick-btn",
            Some(json!({"enabled": false})),
            None,
        )
        .await;
    c.check("PATCH /modules/{id}", "Module", &mp.json());
    let put = c
        .call(
            "PUT",
            "/v1/secrets/provider.x",
            Some(json!({"value": "v"})),
            None,
        )
        .await;
    c.check("PUT /secrets/{ref}", "SecretInfo", &put.json());
    c.documented("PUT", "/secrets/{ref}", &put);
    let sl = c.call("GET", "/v1/secrets", None, None).await;
    for s in sl.json().as_array().into_iter().flatten() {
        c.check("GET /secrets", "SecretInfo", s);
    }
    let files = c.call("GET", "/v1/files", None, None).await;
    for f in files.json().as_array().into_iter().flatten() {
        c.check("GET /files", "FileInfo", f);
    }
    let denied = c
        .call("GET", "/v1/files/content?path=keys.env", None, None)
        .await;
    assert_eq!(denied.status, 403);
    c.documented("GET", "/files/content", &denied);
    for (m, path, pat, want) in [
        (
            "DELETE",
            format!("/v1/tunnels/{}", t.json()["id"]),
            "/tunnels/{id}",
            204,
        ),
        (
            "DELETE",
            format!("/v1/personas/{pid}"),
            "/personas/{id}",
            204,
        ),
        (
            "DELETE",
            "/v1/secrets/provider.x".to_string(),
            "/secrets/{ref}",
            204,
        ),
        (
            "DELETE",
            "/v1/modules/quick-btn".to_string(),
            "/modules/{module_id}",
            204,
        ),
        (
            "DELETE",
            format!("/v1/accounts/{id}/connect"),
            "/accounts/{id}/connect",
            204,
        ),
    ] {
        let w = c.call(m, &path, None, None).await;
        assert_eq!(w.status, want, "{m} {path}");
        c.documented(m, pat, &w);
    }
}
