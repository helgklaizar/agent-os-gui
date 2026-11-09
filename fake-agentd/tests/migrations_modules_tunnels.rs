//! Migration 0007 (`modules`, `tunnels`; db.md section 3.9): applying, version, foreign keys, CHECK, account cascade.

mod common;

use common::*;
use fake_agentd::{Db, MIGRATIONS};
use rusqlite::{Connection, params};

/// A DB at version 6, then the live `Db::open` (the "up from version 6" path).
fn upgraded_from_v6(path: &std::path::Path) -> Db {
    let old = Db::open_with(path, &MIGRATIONS[..6]).expect("DB version 6");
    drop(old);
    open(path)
}

fn fresh() -> (tempfile::TempDir, Db) {
    let (d, path) = temp_db_path();
    let db = upgraded_from_v6(&path);
    (d, db)
}

fn n(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1)
}

fn module(w: &Connection, id: &str, placement: &str) -> rusqlite::Result<usize> {
    w.execute(
        "INSERT INTO modules(id, name, manifest_json, placement) VALUES (?1, 'm', '{}', ?2)",
        params![id, placement],
    )
}

fn tunnel(w: &Connection, kind: &str) -> rusqlite::Result<usize> {
    w.execute(
        "INSERT INTO tunnels(type, addr) VALUES (?1, 'host:1')",
        params![kind],
    )
}

fn account(w: &Connection, slug: &str) -> i64 {
    w.execute(
        "INSERT INTO accounts(slug, type, label, created_at) VALUES (?1, 'wa', 'x', 100)",
        params![slug],
    )
    .expect("account");
    w.last_insert_rowid()
}

#[test]
fn migration_modules_tunnels_applies_on_clean_db() {
    let (_d, db) = fresh();
    let r = db.reader().expect("reader");
    assert_eq!(applied_versions(&r), code_versions());
    assert_eq!(user_version(&r), latest_version());
    let name: String = r
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 7",
            [],
            |x| x.get(0),
        )
        .unwrap_or_default();
    assert_eq!(name, "modules_tunnels");
    for t in ["modules", "tunnels"] {
        assert!(object_exists(&r, "table", t), "table {t}");
    }
    assert!(object_exists(&r, "trigger", "trg_findings_ref_modules"));
}

#[test]
fn migration_modules_tunnels_idempotent_by_version() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v6(&path);
    db.write(|w| module(w, "a", "panel").map(|_| ()))
        .expect("row");
    drop(db);
    let db = Db::open(&path).expect("reopening");
    let r = db.reader().expect("reader");
    assert_eq!(user_version(&r), latest_version());
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM schema_migrations WHERE version = 7"
        ),
        1,
        "the migration is not applied a second time"
    );
    assert_eq!(n(&r, "SELECT count(*) FROM modules"), 1, "data intact");
}

#[test]
fn migration_modules_tunnels_foreign_key_check_empty() {
    let (_d, db) = fresh();
    let r = db.reader().expect("reader");
    assert!(
        object_exists(&r, "table", "modules"),
        "no table - nothing to check"
    );
    let mut stmt = r.prepare("PRAGMA foreign_key_check").expect("pragma");
    let rows = stmt.query_map([], |_| Ok(())).expect("rows").count();
    assert_eq!(rows, 0);
}

#[test]
fn modules_check_placement() {
    let (_d, db) = fresh();
    db.write(|w| {
        for (i, p) in ["panel", "widget", "button"].iter().enumerate() {
            assert_eq!(module(w, &format!("ok{i}"), p), Ok(1), "{p} is valid");
        }
        assert!(module(w, "bad", "sidebar").is_err(), "invalid placement");
        assert!(module(w, "bad2", "").is_err(), "empty placement");
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM modules"), 3);
}

#[test]
fn tunnels_check_type() {
    let (_d, db) = fresh();
    db.write(|w| {
        for t in ["wireguard", "tailscale", "ssh", "cloudflare", "ngrok"] {
            assert_eq!(tunnel(w, t), Ok(1), "{t} is valid");
        }
        assert!(tunnel(w, "openvpn").is_err(), "invalid type");
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM tunnels"), 5);
    assert_eq!(
        n(&r, "SELECT min(enabled) + max(enabled) FROM tunnels"),
        0,
        "disabled by default"
    );
}

#[test]
fn delete_account_cascades_dependents() {
    let (_d, db) = fresh();
    db.write(|w| {
        let a = account(w, "a1");
        let b = account(w, "a2");
        let p = chat_rows::persona(w, "P");
        for acc in [a, b] {
            w.execute(
                "INSERT INTO persona_accounts VALUES (?1, ?2)",
                params![p, acc],
            )?;
            w.execute(
                "INSERT INTO quotas(account_id, updated_at) VALUES (?1, 1)",
                params![acc],
            )?;
            w.execute(
                "INSERT INTO quota_samples(account_id, ts) VALUES (?1, 1)",
                params![acc],
            )?;
        }
        w.execute(
            "INSERT INTO gw_messages(account_id, ext_id, direction, body, ts) VALUES (?1, 'e', 'in', 'b', 1)",
            params![b],
        )?;
        assert_eq!(
            w.execute("DELETE FROM accounts WHERE id = ?1", params![a]),
            Ok(1)
        );
        assert!(
            w.execute("DELETE FROM accounts WHERE id = ?1", params![b])
                .is_err(),
            "with a gw_messages archive a hard deletion is blocked (RESTRICT)"
        );
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert!(
        object_exists(&r, "table", "modules"),
        "migration 0007 applied"
    );
    assert_eq!(n(&r, "SELECT count(*) FROM persona_accounts"), 1);
    assert_eq!(n(&r, "SELECT count(*) FROM quotas"), 1);
    assert_eq!(n(&r, "SELECT count(*) FROM quota_samples"), 1);
    assert_eq!(n(&r, "SELECT count(*) FROM accounts"), 1);
}
