//! Migration 0002: soft account deletion through the mock API clears quota data.

mod common;
mod mig0002_common;
#[allow(dead_code)]
#[path = "contract/rawhttp.rs"]
mod rawhttp;

use fake_agentd::{Config, FakeAgentd};
use mig0002_common::*;
use rusqlite::{Connection, params};
use serde_json::json;

fn fixtures_config() -> Config {
    Config::temp().no_ticker()
}

#[tokio::test(flavor = "multi_thread")]
async fn account_soft_delete_clears_quota_data() {
    let d = FakeAgentd::start(fixtures_config()).await.expect("mock");
    let read = Connection::open_with_flags(d.db_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("reading the mock DB");
    let id = account_id(&read, "cl1");
    assert!(id > 0, "the cl1 account fixture was not created");
    let count = |sql: &str, id: i64| -> i64 {
        read.query_row(sql, params![id], |x| x.get(0)).unwrap_or(-1)
    };
    assert_eq!(
        count("SELECT count(*) FROM quotas WHERE account_id = ?1", id),
        1
    );
    assert!(
        count(
            "SELECT count(*) FROM quota_samples WHERE account_id = ?1",
            id
        ) > 0,
        "cl1 has samples"
    );
    let secret_before: Option<String> = read
        .query_row(
            "SELECT secret_ref FROM accounts WHERE id = ?1",
            params![id],
            |x| x.get(0),
        )
        .unwrap_or(None);
    assert!(secret_before.is_some(), "cl1 has a secret reference");

    let path = format!("/v1/accounts/{id}");
    let sock = d.socket_path();
    let anon = rawhttp::send(sock, rawhttp::Req::new("DELETE", &path)).await;
    assert_eq!(anon.status, 401, "without a token");
    let reader = d
        .issue(fake_agentd::TokenSpec::new(
            fake_agentd::Scope::Read,
            fake_agentd::Kind::Test,
        ))
        .expect("read token")
        .token;
    let denied = rawhttp::send(sock, rawhttp::Req::new("DELETE", &path).token(&reader)).await;
    assert_eq!(denied.status, 403, "scope read does not delete");
    assert_eq!(
        count("SELECT count(*) FROM quotas WHERE account_id = ?1", id),
        1,
        "after the refusals the data is intact"
    );

    let done = rawhttp::send(
        sock,
        rawhttp::Req::new("DELETE", &path).token(d.admin_token()),
    )
    .await;
    assert_eq!(done.status, 204);
    let deleted_at: Option<i64> = read
        .query_row(
            "SELECT deleted_at FROM accounts WHERE id = ?1",
            params![id],
            |x| x.get(0),
        )
        .unwrap_or(None);
    assert!(
        deleted_at.is_some_and(|t| t > 1_700_000_000),
        "deleted_at is set"
    );
    assert_eq!(
        count("SELECT count(*) FROM accounts WHERE id = ?1", id),
        1,
        "the account row is alive"
    );
    assert_eq!(
        count("SELECT count(*) FROM quotas WHERE account_id = ?1", id),
        0
    );
    assert_eq!(
        count(
            "SELECT count(*) FROM quota_samples WHERE account_id = ?1",
            id
        ),
        0
    );
    let secret_after: Option<String> = read
        .query_row(
            "SELECT secret_ref FROM accounts WHERE id = ?1",
            params![id],
            |x| x.get(0),
        )
        .unwrap_or(Some("not read".into()));
    assert_eq!(secret_after, None, "secret_ref cleared");
    let other = account_id(&read, "cl2");
    assert_eq!(
        count("SELECT count(*) FROM quotas WHERE account_id = ?1", other),
        1,
        "another account untouched"
    );

    let again = rawhttp::send(
        sock,
        rawhttp::Req::new("DELETE", &path).token(d.admin_token()),
    )
    .await;
    assert_eq!(again.status, 404, "repeated deletion");
    let unknown = rawhttp::send(
        sock,
        rawhttp::Req::new("DELETE", "/v1/accounts/99999").token(d.admin_token()),
    )
    .await;
    assert_eq!(unknown.status, 404, "unknown account");
    assert_eq!(unknown.json()["error"]["code"], json!("not_found"));
}
