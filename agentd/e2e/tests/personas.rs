//! A2-02: personas (`/personas`), soft delete, the `settings` topic.

mod common;

use std::time::Duration;

use common::{Env, fields, persona};
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn persona_create_list_patch_roundtrip() {
    let e = Env::start().await;
    let acc = e
        .admin
        .make("/accounts", json!({"type": "anthropic", "label": "k"}))
        .await;
    let body = json!({
        "name": "Work", "color": "#7aa2f7", "style": "brief", "lang": "en", "signature": "— Me",
        "mem_mode": "shared", "notif_mode": "important", "work_hours": "9-18", "quiet_hours": "22-7",
        "model_acc": acc["id"], "account_ids": [acc["id"]],
    });
    let p = e.admin.make("/personas", body).await;
    assert!(p["id"].as_i64().unwrap() > 0);
    for (k, v) in [
        ("name", json!("Work")),
        ("mem_mode", json!("shared")),
        ("model_acc", acc["id"].clone()),
        ("account_ids", json!([acc["id"]])),
    ] {
        assert_eq!(p[k], v, "field {k}: {p}");
    }
    assert!(
        p.get("deleted_at").is_none(),
        "deleted_at is not returned: {p}"
    );
    let (st, list) = e.read.get("/personas").await;
    assert_eq!(st, 200);
    assert_eq!(list.as_array().unwrap().len(), 1);
    // Defaults from docs/reference/db.md section 3.4.
    let d = e.admin.make("/personas", persona("Home")).await;
    assert_eq!(
        (
            d["mem_mode"].as_str(),
            d["notif_mode"].as_str(),
            d["lang"].as_str()
        ),
        (Some("own"), Some("all"), Some("ru"))
    );
    assert_eq!(d["account_ids"], json!([]));
    assert_eq!(d["model_acc"], Value::Null);
    // PATCH: only the sent fields; account_ids is replaced as a whole.
    let id = p["id"].as_i64().unwrap();
    let (st, q) = e
        .admin
        .patch(
            &format!("/personas/{id}"),
            json!({"name": "Work 2", "account_ids": []}),
        )
        .await;
    assert_eq!(st, 200, "{q}");
    assert_eq!(
        (
            q["name"].as_str(),
            q["style"].as_str(),
            q["account_ids"].clone()
        ),
        (Some("Work 2"), Some("brief"), json!([]))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn persona_fields_validation() {
    let e = Env::start().await;
    let cases = [
        (json!({"name": "", "color": "#fff"}), "name"),
        (json!({"name": "  ", "color": "#fff"}), "name"),
        (json!({"name": "x".repeat(101), "color": "#fff"}), "name"),
        (json!({"name": "a\0b", "color": "#fff"}), "name"),
        (json!({"name": "p", "color": ""}), "color"),
        (
            json!({"name": "p", "color": "#fff", "mem_mode": "everything"}),
            "mem_mode",
        ),
        (
            json!({"name": "p", "color": "#fff", "notif_mode": "loud"}),
            "notif_mode",
        ),
        (
            json!({"name": "p", "color": "#fff", "model_acc": 999}),
            "model_acc",
        ),
        (
            json!({"name": "p", "color": "#fff", "account_ids": [999]}),
            "account_ids",
        ),
        (
            json!({"name": "p", "color": "#fff", "account_ids": "1"}),
            "account_ids",
        ),
        (
            json!({"name": "p", "color": "#fff", "unknown": 1}),
            "unknown",
        ),
        (json!({"color": "#fff"}), "name"),
    ];
    for (body, field) in cases {
        let (st, v) = e.admin.post("/personas", body.clone()).await;
        assert_eq!(st, 422, "{body}: {v}");
        assert_eq!(v["error"]["code"], "validation_failed");
        assert!(
            fields(&v).contains(&field.to_string()),
            "{body}: fields {:?}",
            fields(&v)
        );
    }
    // Several errors at once — one entry per field.
    let (_, v) = e
        .admin
        .post(
            "/personas",
            json!({"name": "", "color": "", "mem_mode": "x"}),
        )
        .await;
    assert_eq!(fields(&v).len(), 3, "{v}");
    // PATCH is validated the same way and creates nothing foreign.
    let p = e.admin.make("/personas", persona("a")).await;
    let (st, v) = e
        .admin
        .patch(&format!("/personas/{}", p["id"]), json!({"mem_mode": "x"}))
        .await;
    assert_eq!((st, fields(&v)), (422, vec!["mem_mode".to_string()]));
    let (st, _) = e.admin.patch("/personas/999", json!({"name": "x"})).await;
    assert_eq!(st, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn persona_requires_admin() {
    let e = Env::start().await;
    let p = e.admin.make("/personas", persona("a")).await;
    let path = format!("/personas/{}", p["id"]);
    for (st, v) in [
        e.chat.post("/personas", persona("b")).await,
        e.chat.patch(&path, json!({"name": "z"})).await,
        e.chat.delete(&path).await,
        e.read.post("/personas", persona("b")).await,
    ] {
        assert_eq!(
            (st, v["error"]["code"].as_str()),
            (403, Some("insufficient_scope")),
            "{v}"
        );
    }
    assert_eq!(e.chat.get("/personas").await.0, 200, "chat can read");
    assert_eq!(e.read.get("/personas").await.0, 200, "read can read");
    let (_, list) = e.read.get("/personas").await;
    assert_eq!(list[0]["name"], "a", "nothing changed");
}

#[tokio::test(flavor = "multi_thread")]
async fn persona_delete_is_soft_and_hidden() {
    let mut e = Env::start().await;
    let p = e.admin.make("/personas", persona("a")).await;
    let id = p["id"].as_i64().unwrap();
    assert_eq!(e.admin.delete(&format!("/personas/{id}")).await.0, 204);
    assert_eq!(
        e.admin.get("/personas").await.1,
        json!([]),
        "the deleted one is not returned"
    );
    assert_eq!(
        e.admin.delete(&format!("/personas/{id}")).await.0,
        404,
        "repeated delete"
    );
    assert_eq!(
        e.admin
            .patch(&format!("/personas/{id}"), json!({"name": "x"}))
            .await
            .0,
        404
    );
    let n: Option<i64> = e
        .inspect(move |c| {
            c.query_row("SELECT deleted_at FROM personas WHERE id = ?1", [id], |r| {
                r.get(0)
            })
        })
        .await;
    assert!(n.is_some(), "the row remains, deleted_at is set");
}

#[tokio::test(flavor = "multi_thread")]
async fn persona_change_publishes_settings_rev() {
    let e = Env::start().await;
    let mut ws = e.ws();
    ws.subscribe("settings");
    let p = e.admin.make("/personas", persona("a")).await;
    let frames = ws.drain(Duration::from_millis(500));
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(frames[0].data["page"], "personas");
    let rev = frames[0].data["rev"].as_i64().unwrap();
    let (_, page) = e.read.get("/settings/personas").await;
    assert_eq!(page["rev"], rev, "rev in the delta equals the page rev");
    e.admin
        .patch(&format!("/personas/{}", p["id"]), json!({"name": "b"}))
        .await;
    e.admin.delete(&format!("/personas/{}", p["id"])).await;
    let frames = ws.drain(Duration::from_millis(500));
    let revs: Vec<i64> = frames
        .iter()
        .map(|f| f.data["rev"].as_i64().unwrap())
        .collect();
    assert_eq!(revs, vec![rev + 1, rev + 2], "one delta per change");
}

#[tokio::test(flavor = "multi_thread")]
async fn evil_persona_names_stored_verbatim() {
    let e = Env::start().await;
    let evil = "<b>x</b> {{y}} <img src=x onerror=alert(1)>";
    let p = e
        .admin
        .make(
            "/personas",
            json!({"name": evil, "color": "#fff", "signature": evil}),
        )
        .await;
    assert_eq!(
        (p["name"].as_str(), p["signature"].as_str()),
        (Some(evil), Some(evil))
    );
    let (st, _) = e
        .admin
        .post(
            "/personas",
            json!({"name": "x".repeat(10_000), "color": "#fff"}),
        )
        .await;
    assert_eq!(st, 422);
}
