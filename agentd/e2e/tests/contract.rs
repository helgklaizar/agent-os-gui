//! `contract_a2_routes`: A2 route responses against the `openapi-draft.yaml` schemas (fields, required fields, types)
//! and response codes against those declared for the operation.

mod common;

use common::Env;
use serde_json::json;

mod openapi;

use openapi::{check, declared, schemas};

#[tokio::test(flavor = "multi_thread")]
async fn contract_a2_routes() {
    let all = schemas();
    for name in ["Folder", "Session", "SessionConfig", "Persona", "Account"] {
        assert!(!all[name].props.is_empty(), "schema {name} was not parsed");
    }
    let e = Env::start().await;
    let mut seen: Vec<(&str, &str, u16)> = Vec::new();
    let mut rec = |method: &'static str, path: &'static str, st: u16| seen.push((method, path, st));

    let (st, acc) = e
        .admin
        .post("/accounts", json!({"type": "anthropic", "label": "k"}))
        .await;
    rec("post", "/accounts", st);
    check("Account", &acc, &all);
    let (st, p) = e
        .admin
        .post(
            "/personas",
            json!({"name": "p", "color": "#fff", "account_ids": [acc["id"]]}),
        )
        .await;
    rec("post", "/personas", st);
    check("Persona", &p, &all);
    let (st, f) = e
        .chat
        .post("/folders", json!({"name": "f", "persona_id": p["id"]}))
        .await;
    rec("post", "/folders", st);
    check("Folder", &f, &all);
    let body =
        json!({"name": "s", "folder_id": f["id"], "path": e.home.join("p").to_string_lossy()});
    let (st, s) = e.chat.post("/sessions", body).await;
    rec("post", "/sessions", st);
    check("Session", &s, &all);
    let sid = s["id"].as_i64().unwrap();
    let (st, t) = e
        .chat
        .post(&format!("/sessions/{sid}/threads"), json!({"name": "t"}))
        .await;
    rec("post", "/sessions/{id}/threads", st);
    check("Session", &t, &all);
    check("SessionConfig", &t["cfg"], &all);

    for (path, schema) in [
        ("/folders", "Folder"),
        ("/personas", "Persona"),
        ("/accounts", "Account"),
    ] {
        let (st, list) = e.read.get(path).await;
        rec("get", path, st);
        list.as_array()
            .unwrap()
            .iter()
            .for_each(|i| check(schema, i, &all));
    }
    let (st, list) = e.read.get("/sessions").await;
    rec("get", "/sessions", st);
    list.as_array()
        .unwrap()
        .iter()
        .for_each(|i| check("Session", i, &all));
    let (st, one) = e.read.get(&format!("/sessions/{sid}")).await;
    rec("get", "/sessions/{id}", st);
    check("Session", &one, &all);

    let (st, v) = e
        .chat
        .patch(
            &format!("/sessions/{sid}"),
            json!({"name": "n", "cfg": {"mem": true}, "archived": false}),
        )
        .await;
    rec("patch", "/sessions/{id}", st);
    check("Session", &v, &all);
    check("SessionConfig", &v["cfg"], &all);
    let (st, v) = e
        .chat
        .patch(&format!("/folders/{}", f["id"]), json!({"open": false}))
        .await;
    rec("patch", "/folders/{id}", st);
    check("Folder", &v, &all);
    let (st, v) = e
        .admin
        .patch(&format!("/personas/{}", p["id"]), json!({"name": "q"}))
        .await;
    rec("patch", "/personas/{id}", st);
    check("Persona", &v, &all);
    let (st, v) = e
        .admin
        .patch(&format!("/accounts/{}", acc["id"]), json!({"label": "z"}))
        .await;
    rec("patch", "/accounts/{id}", st);
    check("Account", &v, &all);
    rec(
        "post",
        "/sessions/{id}/read",
        e.chat
            .post(&format!("/sessions/{sid}/read"), json!({}))
            .await
            .0,
    );
    // Errors: 404, 409, 422 with a body in the api.md section 3 format.
    let (st, err) = e.chat.patch("/folders/999", json!({"name": "x"})).await;
    rec("patch", "/folders/{id}", st);
    assert_eq!(err["error"]["code"], "not_found");
    let (st, err) = e
        .chat
        .patch(
            &format!("/folders/{}", f["id"]),
            json!({"parent_id": f["id"]}),
        )
        .await;
    rec("patch", "/folders/{id}", st);
    assert_eq!(err["error"]["code"], "conflict");
    let (st, err) = e.chat.post("/folders", json!({"name": ""})).await;
    rec("post", "/folders", st);
    assert_eq!(err["error"]["details"]["fields"][0]["field"], "name");
    rec(
        "delete",
        "/sessions/{id}",
        e.chat.delete(&format!("/sessions/{sid}")).await.0,
    );
    rec(
        "delete",
        "/folders/{id}",
        e.chat.delete(&format!("/folders/{}", f["id"])).await.0,
    );
    rec(
        "delete",
        "/personas/{id}",
        e.admin.delete(&format!("/personas/{}", p["id"])).await.0,
    );
    rec(
        "delete",
        "/accounts/{id}",
        e.admin.delete(&format!("/accounts/{}", acc["id"])).await.0,
    );

    assert!(seen.len() >= 22, "{} calls", seen.len());
    for (method, path, st) in &seen {
        let ok = declared(path, method);
        // Error codes common to all operations are not listed on every operation in the spec.
        assert!(
            ok.contains(st) || matches!(st, 404 | 409 | 422),
            "{method} {path} -> {st}, declared {ok:?}"
        );
        assert!(!ok.is_empty(), "{method} {path}: operation not in the spec");
    }
    // The effective identity is in the schema and filled in from the folder.
    assert_eq!(one["effective_persona_id"], p["id"]);
}
