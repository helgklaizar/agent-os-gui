//! `fake-agentd` tokens and theme live in SQLite per the `0001_init.sql` schema (DB connected to the mock).

mod common;

use std::path::Path;
use std::time::Duration;

use common::{http, issue, quiet, send, sha256_hex, start};
use core_client::api::{Scope as ApiScope, Theme, TokenCreate};
use core_client::http::Request;
use fake_agentd::{Config, FakeAgentd, Kind, Scope};
use rusqlite::{Connection, OpenFlags};

fn open_ro(path: &Path) -> Connection {
    assert!(
        path.exists(),
        "mock DB file not created: {}",
        path.display()
    );
    match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(e) => panic!("DB did not open: {e}"),
    }
}

fn token_rows(path: &Path) -> Vec<(i64, String, Option<i64>, Option<i64>)> {
    let c = open_ro(path);
    let mut st = match c
        .prepare("SELECT id, token_hash, revoked_at, last_used_at FROM api_token ORDER BY id")
    {
        Ok(s) => s,
        Err(e) => panic!("no api_token table: {e}"),
    };
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)));
    match rows {
        Ok(it) => it.flatten().collect(),
        Err(e) => panic!("{e}"),
    }
}

/// The DB file and WAL journal as one byte string: the raw token must be nowhere on disk.
fn raw_disk_bytes(path: &Path) -> Vec<u8> {
    let mut all = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        if let Ok(b) = std::fs::read(Path::new(&name)) {
            all.extend(b);
        }
    }
    all
}

fn contains(hay: &[u8], needle: &str) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

async fn issue_phone(d: &FakeAgentd) -> String {
    let admin = http(d, Some(d.admin_token()));
    match admin
        .issue_token(&TokenCreate {
            name: "phone".into(),
            scope: ApiScope::Chat,
            ttl_days: Some(30),
            kind: None,
            allowlist: None,
        })
        .await
    {
        Ok(t) => t.token,
        Err(e) => panic!("token not issued: {e:?}"),
    }
}

#[tokio::test]
async fn api_token_table_stores_hash_only() {
    let d = quiet().await;
    let phone = issue_phone(&d).await;
    let rows = token_rows(d.db_path());
    let hashes: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
    assert!(rows.len() >= 2, "admin and the issued one: {rows:?}");
    assert!(
        hashes.contains(&sha256_hex(&phone).as_str()),
        "SHA-256 of the issued token in the table"
    );
    assert!(
        hashes.contains(&sha256_hex(d.admin_token()).as_str()),
        "SHA-256 of the admin token in the table"
    );
    for h in &hashes {
        assert!(
            h.len() == 64
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "64 hex only: {h}"
        );
    }
    let disk = raw_disk_bytes(d.db_path());
    assert!(!disk.is_empty(), "the DB is not empty");
    for raw in [phone.as_str(), d.admin_token()] {
        assert!(
            !contains(&disk, raw),
            "raw token found in the DB file or WAL"
        );
        assert!(
            !contains(&disk, &raw[4..]),
            "token body found in the DB file or WAL"
        );
    }
    assert_eq!(
        d.stored_token_hashes().len(),
        rows.len(),
        "the mock sees exactly the table rows"
    );
}

#[tokio::test]
async fn api_token_table_matches_api_listing() {
    let d = quiet().await;
    let _ = issue_phone(&d).await;
    let admin = http(&d, Some(d.admin_token()));
    let listed = match admin.list_tokens().await {
        Ok(l) => l,
        Err(e) => panic!("{e:?}"),
    };
    let ids: Vec<i64> = listed.iter().map(|t| t.id).collect();
    let db_ids: Vec<i64> = token_rows(d.db_path()).iter().map(|r| r.0).collect();
    assert_eq!(
        ids, db_ids,
        "GET /tokens and the api_token table describe the same rows"
    );
}

#[tokio::test]
async fn api_token_revoke_and_last_used_land_in_db() {
    let d = quiet().await;
    let phone = issue_phone(&d).await;
    let phone_id = match token_rows(d.db_path())
        .iter()
        .find(|r| r.1 == sha256_hex(&phone))
    {
        Some(r) => r.0,
        None => panic!("no row of the issued token in the DB"),
    };
    assert!(http(&d, Some(&phone)).theme().await.is_ok());
    let used = token_rows(d.db_path())
        .into_iter()
        .find(|r| r.0 == phone_id);
    assert!(
        matches!(used, Some((_, _, None, Some(_)))),
        "last_used_at written, revoked_at empty: {used:?}"
    );

    let admin = http(&d, Some(d.admin_token()));
    let r = send(&admin, Request::delete(&format!("/v1/tokens/{phone_id}"))).await;
    assert_eq!(r.status, 204);
    let revoked = token_rows(d.db_path())
        .into_iter()
        .find(|r| r.0 == phone_id);
    let Some((_, _, Some(at), _)) = revoked else {
        panic!("revoked_at not written: {revoked:?}")
    };
    d.advance(Duration::from_secs(5));
    let _ = send(&admin, Request::delete(&format!("/v1/tokens/{phone_id}"))).await;
    let again = token_rows(d.db_path())
        .into_iter()
        .find(|r| r.0 == phone_id);
    assert!(
        matches!(again, Some((_, _, Some(a), _)) if a == at),
        "a repeated revocation does not move revoked_at: {again:?}"
    );
    assert!(
        http(&d, Some(&phone)).theme().await.is_err(),
        "a revoked token is not accepted"
    );
}

#[tokio::test]
async fn settings_theme_lands_in_settings_table() {
    let d = quiet().await;
    let c = http(&d, Some(&issue(&d, Scope::Chat, Kind::Test)));
    assert!(c.set_theme(1, Theme::Dark).await.is_ok());
    let conn = open_ro(d.db_path());
    let row: Result<(String, i64), _> = conn.query_row(
        "SELECT value_json, rev FROM settings WHERE key = 'theme'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    );
    assert_eq!(
        row.ok(),
        Some(("\"dark\"".to_string(), 2)),
        "settings.theme: JSON string and rev grows"
    );
    let stale = c.set_theme(1, Theme::Light).await;
    assert!(stale.is_err(), "stale rev rejected");
    let after: Result<(String, i64), _> = conn.query_row(
        "SELECT value_json, rev FROM settings WHERE key = 'theme'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    );
    assert_eq!(
        after.ok(),
        Some(("\"dark\"".to_string(), 2)),
        "412 does not change the row"
    );
}

#[tokio::test]
async fn db_state_survives_restart_on_same_file() {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    };
    let path = dir.path().join("sessions.db");
    let (phone, theme_rev) = {
        let d = start(Config::temp().no_ticker().db(path.clone())).await;
        let phone = issue_phone(&d).await;
        let c = http(&d, Some(d.admin_token()));
        let page = c.set_theme(1, Theme::Dark).await;
        assert!(page.is_ok(), "{page:?}");
        (phone, 2)
    };
    // The DB file is held by the writer lock until the old mock closes.
    let mut second = None;
    for _ in 0..40 {
        match FakeAgentd::start(Config::temp().no_ticker().db(path.clone())).await {
            Ok(d) => {
                second = Some(d);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    let Some(d) = second else {
        panic!("the mock did not come up on the same DB file")
    };
    let c = http(&d, Some(&phone));
    let page = c.theme().await;
    assert!(
        matches!(&page, Ok(p) if p.theme == Theme::Dark && p.rev == theme_rev),
        "token and theme survived a restart: {page:?}"
    );
}
