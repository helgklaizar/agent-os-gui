//! A3-02: secrets via the `agentd` binary and `core-client`: write-only, `0600`, not in logs or the DB, erased with the account.

mod common;

use std::os::unix::fs::PermissionsExt;

use common::{Env, fields};
use serde_json::json;

const VALUE: &str = "sk-e2e-SECRET-9f8e";

fn keys_path(e: &Env) -> std::path::PathBuf {
    e.home.join(".agentos/keys.env")
}

#[tokio::test(flavor = "multi_thread")]
async fn secret_put_write_only() {
    let e = Env::start().await;
    let (st, b) = e
        .admin
        .put("/secrets/provider.anthropic", json!({"value": VALUE}))
        .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(
        (b["ref"].as_str(), b["set"].as_bool()),
        (Some("provider.anthropic"), Some(true))
    );
    assert!(
        b["updated_at"].as_i64().unwrap() > 0 && !b.to_string().contains(VALUE),
        "the value is not returned: {b}"
    );
    let (st, list) = e.admin.get("/secrets").await;
    assert_eq!(st, 200);
    let items = list.as_array().unwrap();
    assert_eq!(items.len(), 1, "{list}");
    assert_eq!(items[0]["ref"], "provider.anthropic");
    assert_eq!(items[0]["set"], true);
    assert!(items[0]["updated_at"].as_i64().unwrap() > 0);
    assert!(!list.to_string().contains(VALUE));
    // The value cannot be read back by any method.
    let (st, _) = e.admin.get("/secrets/provider.anthropic").await;
    assert_eq!(st, 405);
}

#[tokio::test(flavor = "multi_thread")]
async fn secret_requires_admin_local() {
    let e = Env::start().await;
    for api in [&e.chat, &e.read] {
        let (st, b) = api
            .put("/secrets/provider.anthropic", json!({"value": VALUE}))
            .await;
        assert_eq!(
            (st, b["error"]["code"].as_str()),
            (403, Some("insufficient_scope"))
        );
        let (st, _) = api.get("/secrets").await;
        assert_eq!(st, 403);
        let (st, _) = api.delete("/secrets/provider.anthropic").await;
        assert_eq!(st, 403);
    }
    assert!(!keys_path(&e).exists(), "the refusal wrote nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn secret_validation() {
    let e = Env::start().await;
    for (path, body, field) in [
        ("/secrets/provider.A", json!({"value": "v"}), "ref"),
        ("/secrets/keys.env", json!({"value": "v"}), "ref"),
        ("/secrets/account.0", json!({"value": "v"}), "ref"),
        ("/secrets/provider.a", json!({"value": ""}), "value"),
        ("/secrets/provider.a", json!({"value": 5}), "value"),
        ("/secrets/provider.a", json!({}), "value"),
        (
            "/secrets/provider.a",
            json!({"value": "v", "extra": 1}),
            "extra",
        ),
    ] {
        let (st, b) = e.admin.put(path, body.clone()).await;
        assert_eq!(st, 422, "{path} {body}: {b}");
        assert!(fields(&b).contains(&field.to_string()), "{path}: {b}");
    }
    let (st, _) = e
        .admin
        .put("/secrets/provider.a", json!({"value": "x".repeat(9000)}))
        .await;
    assert_eq!(st, 422);
    assert!(!keys_path(&e).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn secret_file_mode_0600_and_delete() {
    let e = Env::start().await;
    e.admin
        .put("/secrets/provider.openai", json!({"value": VALUE}))
        .await;
    let meta = std::fs::metadata(keys_path(&e)).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let dir = std::fs::metadata(e.home.join(".agentos")).unwrap();
    assert_eq!(dir.permissions().mode() & 0o777, 0o700);
    assert!(
        std::fs::read_to_string(keys_path(&e))
            .unwrap()
            .contains(VALUE)
    );
    let (st, _) = e.admin.delete("/secrets/provider.openai").await;
    assert_eq!(st, 204);
    let (st, b) = e.admin.delete("/secrets/provider.openai").await;
    assert_eq!((st, b["error"]["code"].as_str()), (404, Some("not_found")));
    assert!(
        !std::fs::read_to_string(keys_path(&e))
            .unwrap()
            .contains(VALUE)
    );
    let (_, list) = e.admin.get("/secrets").await;
    assert_eq!(list.as_array().unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn secret_not_in_logs_or_db() {
    let mut e = Env::start_trace().await;
    e.admin
        .put("/secrets/provider.anthropic", json!({"value": VALUE}))
        .await;
    e.admin.get("/secrets").await;
    e.chat
        .put("/secrets/provider.anthropic", json!({"value": VALUE}))
        .await;
    let log = std::fs::read_to_string(&e.log).unwrap();
    assert!(
        log.contains("agentd"),
        "the log is not empty: {} bytes",
        log.len()
    );
    assert!(!log.contains(VALUE), "the secret value reached the log");
    let db = e.db.clone();
    // Stopping the core flushes the WAL into the main file; read both.
    e.inspect(|_| Ok(())).await;
    let mut raw = std::fs::read(&db).unwrap();
    raw.extend(std::fs::read(format!("{}-wal", db.display())).unwrap_or_default());
    let needle = VALUE.as_bytes();
    assert!(
        !raw.windows(needle.len()).any(|w| w == needle),
        "the secret value reached the DB"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn secret_erased_on_account_delete() {
    let mut e = Env::start().await;
    let a = e
        .admin
        .make("/accounts", json!({"type": "anthropic", "label": "k"}))
        .await;
    let id = a["id"].as_i64().unwrap();
    let r = format!("account.{id}");
    let (st, _) = e
        .admin
        .put(&format!("/secrets/{r}"), json!({"value": VALUE}))
        .await;
    assert_eq!(st, 200);
    let linked: Option<String> = e
        .inspect(move |c| {
            c.query_row("SELECT secret_ref FROM accounts WHERE id = ?1", [id], |x| {
                x.get(0)
            })
        })
        .await;
    assert_eq!(
        linked.as_deref(),
        Some(r.as_str()),
        "the secret is bound to the account"
    );
    let (st, _) = e.admin.delete(&format!("/accounts/{id}")).await;
    assert_eq!(st, 204);
    let (_, list) = e.admin.get("/secrets").await;
    assert_eq!(
        list.as_array().unwrap().len(),
        0,
        "the secret is erased: {list}"
    );
    assert!(
        !std::fs::read_to_string(keys_path(&e))
            .unwrap()
            .contains(VALUE)
    );
    let left: Option<String> = e
        .inspect(move |c| {
            c.query_row("SELECT secret_ref FROM accounts WHERE id = ?1", [id], |x| {
                x.get(0)
            })
        })
        .await;
    assert_eq!(left, None, "secret_ref cleared");
}
