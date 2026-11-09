//! A4-02: `GET /permissions`, `PUT /permissions/{scope}/{scope_id}/{action}`, weakening is forbidden.

mod common;

use common::{Env, mode_of, session};
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn permissions_api_get_defaults_and_filters() {
    let e = Env::start().await;
    let (st, all) = e.read.get("/permissions").await;
    assert_eq!(st, 200, "{all}");
    for a in ["send", "delete", "pay", "config_write"] {
        assert_eq!(
            mode_of(&all, "global", 0, a).as_deref(),
            Some("ask"),
            "{a}: {all}"
        );
    }
    assert_eq!(all.as_array().unwrap().len(), 4, "{all}");
    let (_, g) = e.read.get("/permissions?scope=global").await;
    assert_eq!(g.as_array().unwrap().len(), 4);
    let (_, none) = e.read.get("/permissions?scope=session&scope_id=1").await;
    assert_eq!(none, json!([]));
    for bad in [
        "/permissions?scope=planet",
        "/permissions?scope_id=x",
        "/permissions?scope=global&scope_id=-1",
    ] {
        let (st, v) = e.read.get(bad).await;
        assert_eq!(st, 400, "{bad}: {v}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn permissions_api_put_and_read_back() {
    let e = Env::start().await;
    let (st, v) = e
        .admin
        .put("/permissions/global/0/shell", json!({"mode": "deny"}))
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(
        v,
        json!({"scope": "global", "scope_id": 0, "action": "shell", "mode": "deny"})
    );
    let (_, all) = e.read.get("/permissions?scope=global").await;
    assert_eq!(mode_of(&all, "global", 0, "shell").as_deref(), Some("deny"));
    // The same `PUT` again and tightening below are allowed.
    let (st, _) = e
        .admin
        .put("/permissions/global/0/shell", json!({"mode": "deny"}))
        .await;
    assert_eq!(st, 200);
    let s = session(&e, None).await;
    let (st, v) = e
        .admin
        .put(
            &format!("/permissions/session/{s}/shell"),
            json!({"mode": "deny"}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn permissions_api_put_admin_local_only() {
    let e = Env::start().await;
    let s = session(&e, None).await;
    let paths = [
        "/permissions/global/0/shell".to_string(),
        format!("/permissions/session/{s}/shell"),
    ];
    for p in &paths {
        for (who, api) in [
            ("chat", &e.chat),
            ("read", &e.read),
            ("agent", &e.agent),
            ("module", &e.module),
        ] {
            let (st, v) = api.put(p, json!({"mode": "deny"})).await;
            assert_eq!(st, 403, "{who} {p}: {v}");
        }
    }
    let (_, all) = e.read.get("/permissions").await;
    assert_eq!(
        mode_of(&all, "global", 0, "shell"),
        None,
        "rule not created by a foreign token"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn permissions_api_validation() {
    let e = Env::start().await;
    let s = session(&e, None).await;
    let cases: [(String, Value, u16); 9] = [
        (
            "/permissions/global/0/teleport".into(),
            json!({"mode": "deny"}),
            422,
        ),
        (
            "/permissions/global/0/shell".into(),
            json!({"mode": "maybe"}),
            422,
        ),
        ("/permissions/global/0/shell".into(), json!({}), 422),
        (
            "/permissions/global/0/shell".into(),
            json!({"mode": "deny", "x": 1}),
            422,
        ),
        (
            "/permissions/global/5/shell".into(),
            json!({"mode": "deny"}),
            422,
        ),
        (
            "/permissions/planet/0/shell".into(),
            json!({"mode": "deny"}),
            422,
        ),
        (
            "/permissions/persona/999/shell".into(),
            json!({"mode": "deny"}),
            404,
        ),
        (
            format!("/permissions/session/{}/shell", s + 1000),
            json!({"mode": "deny"}),
            404,
        ),
        (
            "/permissions/session/abc/shell".into(),
            json!({"mode": "deny"}),
            400,
        ),
    ];
    for (path, body, want) in cases {
        let (st, v) = e.admin.put(&path, body.clone()).await;
        assert_eq!(st, want, "{path} {body}: {v}");
    }
    let (_, all) = e.read.get("/permissions").await;
    assert_eq!(all.as_array().unwrap().len(), 4, "nothing created: {all}");
}

#[tokio::test(flavor = "multi_thread")]
async fn permissions_api_session_cannot_weaken() {
    let e = Env::start().await;
    let s = session(&e, None).await;
    let (st, v) = e
        .admin
        .put(
            &format!("/permissions/session/{s}/send"),
            json!({"mode": "allow"}),
        )
        .await;
    assert_eq!(
        (st, v["error"]["code"].as_str()),
        (403, Some("permission_denied")),
        "{v}"
    );
    let (_, rows) = e
        .read
        .get(&format!("/permissions?scope=session&scope_id={s}"))
        .await;
    assert_eq!(rows, json!([]), "row not created");
    // Global deny: even ask on a session is a relaxation.
    let (st, _) = e
        .admin
        .put("/permissions/global/0/pay", json!({"mode": "deny"}))
        .await;
    assert_eq!(st, 200);
    let (st, _) = e
        .admin
        .put(
            &format!("/permissions/session/{s}/pay"),
            json!({"mode": "ask"}),
        )
        .await;
    assert_eq!(st, 403);
    // A persona does not relax the global rule, nor does a session.
    let p = e.admin.make("/personas", common::persona("P")).await["id"]
        .as_i64()
        .unwrap();
    let (st, _) = e
        .admin
        .put(
            &format!("/permissions/persona/{p}/send"),
            json!({"mode": "allow"}),
        )
        .await;
    assert_eq!(st, 403);
}
