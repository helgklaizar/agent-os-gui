//! A2-03: accounts without secrets (`/accounts`), `connections` topic.

mod common;

use std::time::Duration;

use common::{Env, fields};
use serde_json::{Value, json};

fn secret_account(c: &agentd_db::Connection) -> agentd_db::SqlResult<()> {
    c.execute_batch(
        "INSERT INTO accounts(slug, type, label, secret_ref, status, created_at)
         VALUES ('cl1', 'anthropic', 'Claude · key-1', 'ANTHROPIC_KEY_PLACEHOLDER', 'connected', 100);",
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn account_create_list_patch() {
    let e = Env::start().await;
    let a = e
        .admin
        .make(
            "/accounts",
            json!({"type": "anthropic", "label": "Claude · key-1"}),
        )
        .await;
    let keys: Vec<&str> = {
        let mut k: Vec<&str> = a.as_object().unwrap().keys().map(String::as_str).collect();
        k.sort_unstable();
        k
    };
    assert_eq!(keys, ["created_at", "id", "label", "status", "type"], "{a}");
    assert_eq!(a["status"], "disconnected");
    let id = a["id"].as_i64().unwrap();
    let (st, l) = e.read.get("/accounts").await;
    assert_eq!((st, l.as_array().unwrap().len()), (200, 1));
    let (st, p) = e
        .admin
        .patch(&format!("/accounts/{id}"), json!({"label": "Renamed"}))
        .await;
    assert_eq!(
        (st, p["label"].as_str(), p["type"].as_str()),
        (200, Some("Renamed"), Some("anthropic"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn account_validation_and_scope() {
    let e = Env::start().await;
    for (body, field) in [
        (json!({"type": "", "label": "x"}), "type"),
        (json!({"type": "Bad Type!", "label": "x"}), "type"),
        (json!({"type": "wa", "label": ""}), "label"),
        (json!({"type": "wa", "label": "x".repeat(101)}), "label"),
        (json!({"type": "wa", "label": "a\0b"}), "label"),
        (json!({"type": "wa"}), "label"),
        (json!({"label": "x"}), "type"),
        (
            json!({"type": "wa", "label": "x", "secret_ref": "K"}),
            "secret_ref",
        ),
        (
            json!({"type": "wa", "label": "x", "status": "connected"}),
            "status",
        ),
    ] {
        let (st, v) = e.admin.post("/accounts", body.clone()).await;
        assert_eq!(st, 422, "{body}: {v}");
        assert!(
            fields(&v).contains(&field.to_string()),
            "{body}: {:?}",
            fields(&v)
        );
    }
    let a = e
        .admin
        .make("/accounts", json!({"type": "wa", "label": "x"}))
        .await;
    let path = format!("/accounts/{}", a["id"]);
    for (st, v) in [
        e.chat
            .post("/accounts", json!({"type": "wa", "label": "y"}))
            .await,
        e.chat.patch(&path, json!({"label": "z"})).await,
        e.chat.delete(&path).await,
    ] {
        assert_eq!(
            (st, v["error"]["code"].as_str()),
            (403, Some("insufficient_scope"))
        );
    }
    assert_eq!(e.read.get("/accounts").await.0, 200);
    assert_eq!(
        e.admin
            .patch("/accounts/999", json!({"label": "z"}))
            .await
            .0,
        404
    );
    assert_eq!(e.admin.delete("/accounts/999").await.0, 404);
    let (st, v) = e
        .admin
        .patch(&path, json!({"label": "z", "type": "tg"}))
        .await;
    assert_eq!(
        (st, fields(&v)),
        (422, vec!["type".to_string()]),
        "type is immutable"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn account_hides_secret_ref() {
    let e = Env::start_with(secret_account).await;
    let (st, list) = e.read.get("/accounts").await;
    assert_eq!(st, 200);
    let text = list.to_string();
    assert_eq!(
        list.as_array().unwrap().len(),
        1,
        "account from the DB is visible"
    );
    for bad in ["secret_ref", "ANTHROPIC_KEY_PLACEHOLDER", "deleted_at"] {
        assert!(!text.contains(bad), "{bad} in response: {text}");
    }
    let mut ws = e.ws();
    let snap = ws.subscribe("connections");
    assert_eq!(snap["items"].as_array().unwrap().len(), 1, "{snap}");
    assert!(!snap.to_string().contains("secret_ref") && !snap.to_string().contains("PLACEHOLDER"));
    let a = e
        .admin
        .make("/accounts", json!({"type": "wa", "label": "x"}))
        .await;
    e.admin
        .patch(&format!("/accounts/{}", a["id"]), json!({"label": "y"}))
        .await;
    for f in ws.drain(Duration::from_millis(500)) {
        assert!(!f.data.to_string().contains("secret_ref"), "{:?}", f);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn account_slug_unique_stable() {
    let mut e = Env::start().await;
    let a = e
        .admin
        .make("/accounts", json!({"type": "anthropic", "label": "a"}))
        .await;
    let b = e
        .admin
        .make("/accounts", json!({"type": "anthropic", "label": "b"}))
        .await;
    let w = e
        .admin
        .make("/accounts", json!({"type": "wa", "label": "w"}))
        .await;
    assert_eq!(
        e.admin.delete(&format!("/accounts/{}", b["id"])).await.0,
        204
    );
    let c = e
        .admin
        .make("/accounts", json!({"type": "anthropic", "label": "c"}))
        .await;
    let ids = [
        a["id"].as_i64().unwrap(),
        b["id"].as_i64().unwrap(),
        w["id"].as_i64().unwrap(),
        c["id"].as_i64().unwrap(),
    ];
    let slugs: Vec<String> = e
        .inspect(move |c| {
            ids.iter()
                .map(|id| {
                    c.query_row("SELECT slug FROM accounts WHERE id = ?1", [id], |r| {
                        r.get(0)
                    })
                })
                .collect()
        })
        .await;
    assert_eq!(
        slugs,
        ["cl1", "cl2", "wa1", "cl3"],
        "slug is not reused after deletion"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn account_soft_delete_clears_refs() {
    let mut e = Env::start_with(|c| {
        secret_account(c)?;
        c.execute_batch(
            "INSERT INTO quotas(account_id, updated_at) VALUES (1, 1);
             INSERT INTO quota_samples(account_id, ts) VALUES (1, 1);
             INSERT INTO personas(name, color, model_acc) VALUES ('p', '#fff', 1);
             INSERT INTO persona_accounts VALUES (1, 1);",
        )
    })
    .await;
    assert_eq!(e.admin.delete("/accounts/1").await.0, 204);
    assert_eq!(
        e.admin.get("/accounts").await.1,
        json!([]),
        "deleted account is not returned"
    );
    assert_eq!(e.admin.delete("/accounts/1").await.0, 404);
    let (_, p) = e.admin.get("/personas").await;
    assert_eq!(p[0]["model_acc"], Value::Null);
    assert_eq!(p[0]["account_ids"], json!([]));
    let row: (bool, bool, i64, i64, i64, i64) = e
        .inspect(|c| {
            let n = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
            Ok((
                c.query_row(
                    "SELECT deleted_at IS NOT NULL FROM accounts WHERE id = 1",
                    [],
                    |r| r.get(0),
                )?,
                c.query_row(
                    "SELECT secret_ref IS NULL FROM accounts WHERE id = 1",
                    [],
                    |r| r.get(0),
                )?,
                n("SELECT count(*) FROM quotas")?,
                n("SELECT count(*) FROM quota_samples")?,
                n("SELECT count(*) FROM persona_accounts")?,
                n("SELECT count(*) FROM pragma_foreign_key_check")?,
            ))
        })
        .await;
    assert_eq!(row, (true, true, 0, 0, 0, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_topic_snapshot_and_status_delta() {
    let e = Env::start().await;
    let mut ws = e.ws();
    assert_eq!(ws.subscribe("connections"), json!({"items": []}));
    let a = e
        .admin
        .make("/accounts", json!({"type": "wa", "label": "x"}))
        .await;
    let frames = ws.drain(Duration::from_millis(500));
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(
        frames[0].data,
        json!({"account_id": a["id"], "status": "disconnected"})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn evil_account_label_stored_verbatim() {
    let e = Env::start().await;
    let evil = "<script>alert(1)</script> \"quoted\" 'x'";
    let a = e
        .admin
        .make("/accounts", json!({"type": "wa", "label": evil}))
        .await;
    assert_eq!(a["label"], evil);
}
