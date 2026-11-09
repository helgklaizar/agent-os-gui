//! Migration 0006 (`tasks`, `task_runs`, `triggers`; db.md section 3.8): applying, indexes, foreign keys, CHECK.

mod common;

use common::*;
use fake_agentd::{Db, MIGRATIONS};
use rusqlite::{Connection, params};

/// A DB at version 5, then the live `Db::open` (the "up from version 5" path).
fn upgraded_from_v5(path: &std::path::Path) -> Db {
    let old = Db::open_with(path, &MIGRATIONS[..5]).expect("DB version 5");
    drop(old);
    Db::open_with(path, &MIGRATIONS[..6]).expect("DB version 6")
}

fn fresh() -> (tempfile::TempDir, Db) {
    let (d, path) = temp_db_path();
    let db = upgraded_from_v5(&path);
    (d, db)
}

fn n(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1)
}

fn session(w: &Connection, name: &str) -> i64 {
    w.execute(
        "INSERT INTO sessions(name, created_at, updated_at) VALUES (?1, 100, 100)",
        params![name],
    )
    .expect("session");
    w.last_insert_rowid()
}

fn task(w: &Connection, session: Option<i64>) -> rusqlite::Result<i64> {
    w.execute(
        "INSERT INTO tasks(kind, title, spec, session_id, created_at) VALUES ('sched', 't', 's', ?1, 100)",
        params![session],
    )?;
    Ok(w.last_insert_rowid())
}

#[test]
fn migration_tasks_applies_on_clean_db() {
    let (_d, db) = fresh();
    let r = db.reader().expect("reader");
    assert_eq!(applied_versions(&r), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(user_version(&r), 6);
    let (name, at): (String, i64) = r
        .query_row(
            "SELECT name, applied_at FROM schema_migrations WHERE version = 6",
            [],
            |x| Ok((x.get(0)?, x.get(1)?)),
        )
        .unwrap_or_default();
    assert_eq!(name, "tasks");
    assert!(at > 1_700_000_000, "applied_at is unix seconds UTC");
    for t in ["tasks", "task_runs", "triggers"] {
        assert!(object_exists(&r, "table", t), "table {t}");
    }
}

#[test]
fn migration_tasks_idempotent_by_version() {
    let (_d, path) = temp_db_path();
    let db = upgraded_from_v5(&path);
    db.write(|w| {
        session(w, "s");
        Ok(())
    })
    .expect("row");
    drop(db);
    let db = Db::open_with(&path, &MIGRATIONS[..6]).expect("reopening");
    let r = db.reader().expect("reader");
    assert_eq!(user_version(&r), 6);
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM schema_migrations WHERE version = 6"
        ),
        1,
        "the migration is not applied a second time"
    );
    assert_eq!(n(&r, "SELECT count(*) FROM sessions"), 1, "data intact");
}

#[test]
fn migration_tasks_indexes_exist() {
    let (_d, db) = fresh();
    let r = db.reader().expect("reader");
    assert!(object_exists(&r, "index", "idx_tasks_next"));
    assert!(object_exists(&r, "index", "idx_task_runs_task"));
}

#[test]
fn migration_tasks_foreign_key_check_empty() {
    let (_d, db) = fresh();
    db.write(|w| {
        let s = session(w, "s");
        let t = task(w, Some(s))?;
        w.execute(
            "INSERT INTO task_runs(task_id, started_at, status) VALUES (?1, 1, 'ok')",
            [t],
        )?;
        w.execute(
            "INSERT INTO triggers(source, match_json, session_id) VALUES ('mail', '{}', ?1)",
            [s],
        )?;
        Ok(())
    })
    .expect("rows");
    let r = db.reader().expect("reader");
    let bad: i64 = r
        .prepare("PRAGMA foreign_key_check")
        .and_then(|mut s| s.query_map([], |_| Ok(())).map(|rows| rows.count() as i64))
        .unwrap_or(-1);
    assert_eq!(bad, 0, "foreign_key_check is empty");
    assert_eq!(pragma_text(&r, "foreign_keys"), "1");
}

#[test]
fn delete_task_cascades_runs() {
    let (_d, db) = fresh();
    db.write(|w| {
        let t = task(w, None)?;
        for status in ["ok", "error"] {
            w.execute(
                "INSERT INTO task_runs(task_id, started_at, status) VALUES (?1, 1, ?2)",
                params![t, status],
            )?;
        }
        w.execute("DELETE FROM tasks WHERE id = ?1", [t])?;
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM task_runs"), 0);
}

#[test]
fn delete_session_nulls_task_and_drops_triggers() {
    let (_d, db) = fresh();
    db.write(|w| {
        let s = session(w, "s");
        task(w, Some(s))?;
        w.execute(
            "INSERT INTO triggers(source, match_json, session_id) VALUES ('mail', '{}', ?1)",
            [s],
        )?;
        w.execute("DELETE FROM sessions WHERE id = ?1", [s])?;
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(&r, "SELECT count(*) FROM tasks WHERE session_id IS NULL"),
        1,
        "the task is alive without a session"
    );
    assert_eq!(
        n(&r, "SELECT count(*) FROM tasks WHERE state = 'paused'"),
        1,
        "and is paused (trigger trg_sessions_pause_tasks)"
    );
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM triggers WHERE session_id IS NULL AND enabled = 0"
        ),
        1,
        "rule disabled, binding cleared"
    );
}

#[test]
fn delete_persona_nulls_task_persona() {
    let (_d, db) = fresh();
    db.write(|w| {
        w.execute(
            "INSERT INTO personas(name, color) VALUES ('p', '#aabbcc')",
            [],
        )?;
        let p = w.last_insert_rowid();
        w.execute(
            "INSERT INTO tasks(kind, title, spec, persona_id, created_at) VALUES ('deleg', 't', 's', ?1, 1)",
            [p],
        )?;
        w.execute("DELETE FROM personas WHERE id = ?1", [p])?;
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(&r, "SELECT count(*) FROM tasks WHERE persona_id IS NULL"),
        1
    );
}

#[test]
fn tasks_check_constraints() {
    let (_d, db) = fresh();
    db.write(|w| {
        let bad_kind = w.execute(
            "INSERT INTO tasks(kind, title, spec, created_at) VALUES ('x', 't', 's', 1)",
            [],
        );
        assert!(bad_kind.is_err(), "kind outside sched|deleg");
        let bad_state = w.execute(
            "INSERT INTO tasks(kind, title, spec, state, created_at) VALUES ('sched', 't', 's', 'gone', 1)",
            [],
        );
        assert!(bad_state.is_err(), "state outside active|paused|done");
        let t = task(w, None)?;
        let bad_status = w.execute(
            "INSERT INTO task_runs(task_id, started_at, status) VALUES (?1, 1, 'weird')",
            [t],
        );
        assert!(bad_status.is_err(), "status outside running|ok|error|skipped");
        let tz: String = w.query_row("SELECT tz FROM tasks WHERE id = ?1", [t], |r| r.get(0))?;
        assert_eq!(tz, "Asia/Jerusalem", "default time zone");
        Ok(())
    })
    .expect("write");
}
