//! A4-05 (end-to-end): token issue and revoke are logged without the token value and without its hash.

mod common;

use common::Env;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn audit_token_events_without_value() {
    let mut e = Env::start().await;
    let (st, issued) = e
        .admin
        .post(
            "/tokens",
            json!({"name": "phone", "scope": "chat", "ttl_days": 30}),
        )
        .await;
    assert_eq!(st, 201, "{issued}");
    let value = issued["token"].as_str().unwrap().to_string();
    let id = issued["id"].as_i64().unwrap();
    let (st, _) = e.admin.delete(&format!("/tokens/{id}")).await;
    assert_eq!(st, 204);
    let (rows, hash) = e
        .inspect(move |c| {
            let rows: Vec<(String, String, Option<String>, Option<String>)> = c
                .prepare("SELECT actor, action, target, args_json FROM actions_log ORDER BY id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<Result<_, _>>()?;
            let hash: String = c.query_row(
                "SELECT token_hash FROM api_token WHERE id = ?1",
                [id],
                |r| r.get(0),
            )?;
            Ok((rows, hash))
        })
        .await;
    let target = format!("token:{id}");
    let kinds: Vec<&str> = rows
        .iter()
        .filter(|r| r.2.as_deref() == Some(&target))
        .map(|r| r.1.as_str())
        .collect();
    assert_eq!(kinds, ["token_issue", "token_revoke"], "{rows:?}");
    assert!(rows.iter().all(|r| r.0 == "user"));
    let dump = format!("{rows:?}");
    assert!(!dump.contains(&value), "token value in the log");
    assert!(!dump.contains(&hash), "token hash in the log");
    let log = std::fs::read_to_string(&e.log).unwrap_or_default();
    assert!(!log.contains(&value));
}
