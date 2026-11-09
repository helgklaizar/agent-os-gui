//! W-101: `GET /v1/actions` on a real journal: entry shape, page boundary (`limit`/`next_cursor`), route exists.
//! Kills the `item`, `list`, `>`, `route` mutants from `agentd/api/src/actions.rs` inside the crate itself (W-100).

mod common;

use std::sync::Arc;

use agentd_core::{Actor, Audit, Decision, NewAction, SystemClock};
use common::{TOKEN_READ, config, db_services, start_with, tcp_req};
use serde_json::{Value, json};

async fn rig(rows: usize) -> (tempfile::TempDir, agentd_api::Server, std::net::SocketAddr) {
    let dir = tempfile::tempdir().unwrap();
    let (db, mut services) = db_services(dir.path());
    let repo = Arc::new(agentd_db::PermRepo::new(db));
    let audit = Audit::new(repo, Arc::new(SystemClock));
    for n in 0..rows {
        audit
            .record(NewAction {
                actor: Actor::User,
                session_id: None,
                persona_id: None,
                action: format!("act{n}"),
                target: Some(format!("t{n}")),
                args: Some(json!({"secret": "not returned"})),
                decision: Some(Decision::Allow),
                result: Some("ok".into()),
            })
            .await
            .unwrap();
    }
    services.audit = Some(audit);
    let server = start_with(config(dir.path()), services).await;
    server.tcp().set_enabled(true);
    for _ in 0..300 {
        if let Some(a) = server.tcp().local_addr() {
            return (dir, server, a);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("TCP did not come up");
}

async fn get(a: std::net::SocketAddr, q: &str) -> (u16, Value) {
    let r = tcp_req(a, "GET", &format!("/v1/actions{q}"), Some(TOKEN_READ)).await;
    let v = if r.body.is_empty() {
        Value::Null
    } else {
        r.json()
    };
    (r.status, v)
}

#[tokio::test(flavor = "multi_thread")]
async fn item_has_the_public_fields_and_no_args() {
    let (_d, _s, a) = rig(1).await;
    let (st, v) = get(a, "").await;
    assert_eq!(st, 200, "{v}");
    let items = v["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{v}");
    let i = &items[0];
    assert_eq!(i["actor"], "user");
    assert!(i["session_id"].is_null() && i["persona_id"].is_null());
    assert_eq!(i["action"], "act0");
    assert_eq!(i["target"], "t0");
    assert_eq!(i["decision"], "allow");
    assert_eq!(i["result"], "ok");
    assert!(i["id"].as_i64().unwrap() > 0 && i["ts"].as_i64().unwrap() > 0);
    assert!(i.get("args_json").is_none() && i.get("undo_json").is_none());
    assert!(!v.to_string().contains("not returned"));
    assert!(v["next_cursor"].is_null(), "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cursor_appears_only_when_more_rows_exist() {
    let (_d, _s, a) = rig(3).await;
    // Exactly `limit` entries: no next page (mutants `>` → `>=`, `==`, `<`).
    let (_, v) = get(a, "?limit=3").await;
    assert_eq!(v["items"].as_array().unwrap().len(), 3, "{v}");
    assert!(v["next_cursor"].is_null(), "{v}");
    // More than `limit`: page truncated, cursor is the `id` of the last returned entry.
    let (_, v) = get(a, "?limit=2").await;
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{v}");
    let last = items[1]["id"].as_i64().unwrap();
    assert_eq!(v["next_cursor"], last.to_string(), "{v}");
    let (_, rest) = get(a, &format!("?limit=2&cursor={last}")).await;
    assert_eq!(rest["items"].as_array().unwrap().len(), 1, "{rest}");
    assert!(rest["next_cursor"].is_null());
    // Newest first.
    assert!(items[0]["id"].as_i64().unwrap() > last);
}

#[tokio::test(flavor = "multi_thread")]
async fn route_exists_and_rejects_bad_query() {
    let (_d, _s, a) = rig(0).await;
    let (st, v) = get(a, "").await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["items"], json!([]));
    let (st, _) = get(a, "?limit=0").await;
    assert_eq!(st, 422);
}
