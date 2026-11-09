//! Migration 0008 `memory_facts` (db.md section 3.10).

mod common;

use common::*;
use fake_agentd::{Db, MIGRATIONS};
use rusqlite::ErrorCode;

fn fresh() -> (tempfile::TempDir, std::path::PathBuf) {
    temp_db_path()
}

/// A DB "on top of schema 7": migrations 1-7 applied, then the runner brings it to the latest.
fn over_s7() -> (tempfile::TempDir, std::path::PathBuf) {
    let (d, path) = temp_db_path();
    let db = Db::open_with(&path, &MIGRATIONS[..7]).expect("schema 7");
    drop(db);
    (d, path)
}

fn constraint_failure(e: &fake_agentd::DbError) -> bool {
    matches!(e, fake_agentd::DbError::Sqlite(rusqlite::Error::SqliteFailure(f, _)) if f.code == ErrorCode::ConstraintViolation)
}

#[test]
fn migration_0008_applies_from_clean_db() {
    let (_d, path) = fresh();
    let db = open(&path);
    let r = db.reader().expect("reader");
    assert_eq!(applied_versions(&r), code_versions());
    assert_eq!(user_version(&r), latest_version());
    assert!(object_exists(&r, "table", "memory_facts"));
    assert!(object_exists(&r, "index", "idx_memory_persona"));
    let name: String = r
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 8",
            [],
            |x| x.get(0),
        )
        .unwrap_or_default();
    assert_eq!(name, "memory_facts");
}

#[test]
fn migration_0008_applies_over_s7_schema_and_keeps_user_version_equal_to_schema() {
    let (_d, path) = over_s7();
    let db = open(&path);
    let r = db.reader().expect("reader");
    let max: i64 = r
        .query_row("SELECT max(version) FROM schema_migrations", [], |x| {
            x.get(0)
        })
        .unwrap_or(-1);
    assert_eq!(max, latest_version());
    assert_eq!(
        user_version(&r),
        max,
        "user_version = max(schema_migrations)"
    );
    assert!(object_exists(&r, "table", "memory_facts"));
    let violations: i64 = r
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |x| {
            x.get(0)
        })
        .unwrap_or(-1);
    assert_eq!(violations, 0);
}

#[test]
fn migration_0008_blocks_hard_delete_of_persona_with_facts() {
    let (_d, path) = over_s7();
    let db = open(&path);
    fixture::seed_search(&db);
    db.write(|w| {
        w.execute(
            "INSERT INTO memory_facts(persona_id, text, ts) VALUES (1, 'personal fact', 5)",
            [],
        )
    })
    .expect("persona fact");
    let err = db
        .write(|w| w.execute("DELETE FROM personas WHERE id = 1", []))
        .err();
    assert!(
        err.as_ref().is_some_and(constraint_failure),
        "ON DELETE RESTRICT: a hard persona deletion with facts is rejected, got {err:?}"
    );
    db.write(|w| w.execute("UPDATE personas SET deleted_at = 9 WHERE id = 1", []))
        .expect("soft deletion");
    let facts: i64 = db
        .reader()
        .and_then(|r| {
            Ok(r.query_row(
                "SELECT count(*) FROM memory_facts WHERE persona_id = 1",
                [],
                |x| x.get(0),
            )?)
        })
        .unwrap_or(-1);
    assert_eq!(
        facts, 1,
        "facts of a soft-deleted persona stay as an archive"
    );
}

#[test]
fn migration_0008_session_delete_sets_fact_session_null() {
    let (_d, path) = over_s7();
    let db = open(&path);
    fixture::seed_search(&db);
    db.write(|w| {
        w.execute(
            "INSERT INTO memory_facts(persona_id, session_id, text, ts) VALUES (NULL, 1, 'from session', 6)",
            [],
        )
    })
    .expect("fact");
    db.write(|w| w.execute("DELETE FROM sessions WHERE id = 1", []))
        .expect("session deletion");
    let (sid, n): (Option<i64>, i64) = db
        .reader()
        .and_then(|r| {
            Ok(r.query_row(
                "SELECT session_id, count(*) FROM memory_facts WHERE text = 'from session'",
                [],
                |x| Ok((x.get(0)?, x.get(1)?)),
            )?)
        })
        .unwrap_or((Some(-1), -1));
    assert_eq!(
        (sid, n),
        (None, 1),
        "SET NULL: the fact is alive, session_id cleared"
    );
}

#[test]
fn migration_0008_source_check_and_default() {
    let (_d, path) = over_s7();
    let db = open(&path);
    db.write(|w| w.execute("INSERT INTO memory_facts(text, ts) VALUES ('t', 1)", []))
        .expect("default source");
    let src: String = db
        .reader()
        .and_then(|r| Ok(r.query_row("SELECT source FROM memory_facts", [], |x| x.get(0))?))
        .unwrap_or_default();
    assert_eq!(src, "agent");
    let err = db
        .write(|w| {
            w.execute(
                "INSERT INTO memory_facts(text, source, ts) VALUES ('t', 'robot', 1)",
                [],
            )
        })
        .err();
    assert!(
        err.as_ref().is_some_and(constraint_failure),
        "CHECK source: {err:?}"
    );
}
