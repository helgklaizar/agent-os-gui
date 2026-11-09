//! Migration 0005 (`events`, `contacts`, `gw_messages`; db.md sections 3.3, 3.11): applying, schema objects, foreign keys.

mod common;
mod mig0005_common;

use common::*;
use fake_agentd::{Db, MIGRATIONS};
use mig0005_common::*;

#[test]
fn migration_0005_applies_on_clean_db() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v4(&path);
    let r = db.reader().expect("reader");
    assert_eq!(applied_versions(&r), vec![1, 2, 3, 4, 5]);
    assert_eq!(user_version(&r), 5);
    let (name, at): (String, i64) = r
        .query_row(
            "SELECT name, applied_at FROM schema_migrations WHERE version = 5",
            [],
            |x| Ok((x.get(0)?, x.get(1)?)),
        )
        .unwrap_or_default();
    assert_eq!(name, "events_gateway");
    assert!(at > 1_700_000_000, "applied_at is unix seconds UTC");
}

#[test]
fn migration_0005_idempotent_by_version() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v4(&path);
    db.write(|w| {
        session(w, "s");
        Ok(())
    })
    .expect("row");
    drop(db);
    let db = Db::open_with(&path, &MIGRATIONS[..5]).expect("reopening");
    let r = db.reader().expect("reader");
    assert_eq!(user_version(&r), 5);
    assert_eq!(applied_versions(&r), vec![1, 2, 3, 4, 5]);
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM schema_migrations WHERE version = 5"
        ),
        1,
        "the migration is not applied a second time"
    );
    assert_eq!(n(&r, "SELECT count(*) FROM sessions"), 1, "data intact");
}

#[test]
fn migration_0005_indexes_and_trigger_exist() {
    let (_d, db) = fresh();
    let r = db.reader().expect("reader");
    for t in ["events", "contacts", "gw_messages"] {
        assert!(object_exists(&r, "table", t), "no table {t}");
    }
    for i in [
        "idx_events_filter",
        "idx_events_session",
        "idx_events_dedup",
        "idx_gw_unread",
    ] {
        assert!(object_exists(&r, "index", i), "no index {i}");
    }
    assert!(
        object_exists(&r, "trigger", "trg_findings_ref_gw"),
        "no trigger trg_findings_ref_gw"
    );
}

#[test]
fn migration_0005_foreign_key_check_empty() {
    let (_d, db) = fresh();
    let r = db.reader().expect("reader");
    assert!(
        object_exists(&r, "table", "events"),
        "the events table must come from the migration"
    );
    assert_eq!(n(&r, "SELECT count(*) FROM pragma_foreign_key_check"), 0);
}

#[test]
fn migration_0005_dedup_index_is_partial_unique() {
    let (_d, db) = fresh();
    let r = db.reader().expect("reader");
    let sql: String = r
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'idx_events_dedup'",
            [],
            |x| x.get(0),
        )
        .unwrap_or_default();
    assert!(sql.contains("UNIQUE"), "the index is unique: {sql}");
    assert!(
        sql.contains("dedup_key IS NOT NULL"),
        "the index is partial and does not depend on unread: {sql}"
    );
    assert!(
        !sql.contains("unread"),
        "unread is not in the uniqueness key"
    );
}
