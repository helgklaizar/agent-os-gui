//! A5-02 (DB): migration 0006 `tasks` (db.md section 3.8) through the real runner.

mod common;

use agentd_db::{MIGRATIONS, MigrateOptions, migrate, user_version};
use common::{file_conn, migrated_to, names, repo_root};
use rusqlite::Connection;

fn db6() -> Connection {
    let c = migrated_to(6);
    assert_eq!(user_version(&c).unwrap(), 6, "DB brought up to version 6");
    c
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn session(c: &Connection) -> i64 {
    c.execute(
        "INSERT INTO sessions(name, created_at, updated_at) VALUES ('s', 1, 1)",
        [],
    )
    .unwrap();
    c.last_insert_rowid()
}

fn task(c: &Connection, sid: Option<i64>, state: &str) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO tasks(kind, title, spec, state, session_id, created_at) VALUES ('sched','t','s',?2,?1,1)",
        rusqlite::params![sid, state],
    )?;
    Ok(c.last_insert_rowid())
}

#[test]
fn migration_0006_applies_up() {
    let mut c = migrated_to(5);
    assert_eq!(user_version(&c).unwrap(), 5);
    let set = &MIGRATIONS[..6.min(MIGRATIONS.len())];
    migrate(&mut c, set, &MigrateOptions::default()).unwrap();
    assert_eq!(user_version(&c).unwrap(), 6);
    let (v, name): (i64, String) = c
        .query_row(
            "SELECT version, name FROM schema_migrations WHERE version = 6",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((v, name.as_str()), (6, "tasks"));
}

#[test]
fn migration_0005_0006_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = file_conn(dir.path());
    let opts = MigrateOptions::default();
    let set = &MIGRATIONS[..6.min(MIGRATIONS.len())];
    migrate(&mut c, set, &opts).unwrap();
    assert_eq!(user_version(&c).unwrap(), 6);
    let out = migrate(&mut c, set, &opts).unwrap();
    assert!(out.applied.is_empty(), "a repeated start applies nothing");
    assert_eq!(user_version(&c).unwrap(), 6);
}

#[test]
fn migration_0006_schema() {
    let c = db6();
    let tables = names(&c, "table");
    for t in ["tasks", "task_runs", "triggers"] {
        assert!(tables.contains(&t.to_string()), "missing table {t}");
    }
    let idx = names(&c, "index");
    for i in ["idx_tasks_next", "idx_task_runs_task"] {
        assert!(idx.contains(&i.to_string()), "missing index {i}");
    }
    assert!(names(&c, "trigger").contains(&"trg_sessions_pause_tasks".to_string()));
}

#[test]
fn migration_0005_0006_foreign_keys_integrity() {
    let c = db6();
    let t = task(&c, None, "active").unwrap();
    c.execute(
        "INSERT INTO task_runs(task_id, started_at, status) VALUES (?1,1,'ok')",
        [t],
    )
    .unwrap();
    assert_eq!(
        count(&c, "SELECT count(*) FROM pragma_foreign_key_check"),
        0
    );
    let r: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(r, "ok");
}

#[test]
fn tasks_state_check() {
    let c = db6();
    assert!(task(&c, None, "running").is_err());
    assert!(task(&c, None, "paused").is_ok());
    assert!(
        c.execute(
            "INSERT INTO tasks(kind, title, spec, created_at) VALUES ('x','t','s',1)",
            []
        )
        .is_err(),
        "kind outside the set"
    );
}

#[test]
fn task_runs_status_check() {
    let c = db6();
    let t = task(&c, None, "active").unwrap();
    let ins = |st: &str| {
        c.execute(
            "INSERT INTO task_runs(task_id, started_at, status) VALUES (?1,1,?2)",
            rusqlite::params![t, st],
        )
    };
    assert!(ins("done").is_err());
    for ok in ["running", "ok", "error", "skipped"] {
        ins(ok).unwrap();
    }
}

#[test]
fn task_runs_cascade_on_task_delete() {
    let c = db6();
    let t = task(&c, None, "active").unwrap();
    c.execute(
        "INSERT INTO task_runs(task_id, started_at, status) VALUES (?1,1,'ok')",
        [t],
    )
    .unwrap();
    c.execute("DELETE FROM tasks WHERE id = ?1", [t]).unwrap();
    assert_eq!(count(&c, "SELECT count(*) FROM task_runs"), 0);
}

#[test]
fn session_delete_pauses_tasks() {
    let c = db6();
    let s = session(&c);
    let t = task(&c, Some(s), "active").unwrap();
    let done = task(&c, Some(s), "done").unwrap();
    c.execute(
        "INSERT INTO triggers(source, match_json, session_id) VALUES ('mail','{}',?1)",
        [s],
    )
    .unwrap();
    c.execute("DELETE FROM sessions WHERE id = ?1", [s])
        .unwrap();
    let st = |id: i64| -> (String, Option<i64>) {
        c.query_row(
            "SELECT state, session_id FROM tasks WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(st(t), ("paused".to_string(), None));
    assert_eq!(st(done).0, "done", "a completed one does not change");
    let (en, sid): (i64, Option<i64>) = c
        .query_row("SELECT enabled, session_id FROM triggers", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((en, sid), (0, None));
}

fn flat(s: &str) -> String {
    s.lines()
        .map(|l| l.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn migration_0006_matches_db_md() {
    let root = repo_root();
    let md = std::fs::read_to_string(root.join("docs/reference/db.md")).unwrap();
    let sql = flat(
        &std::fs::read_to_string(root.join("agentd/migrations/0006_tasks.sql"))
            .expect("migration 0006 file"),
    );
    let wanted = [
        "CREATE TABLE tasks",
        "CREATE INDEX idx_tasks_next",
        "CREATE TABLE task_runs",
        "CREATE INDEX idx_task_runs_task",
        "CREATE TABLE triggers",
    ];
    let mut found = 0;
    for block in md.split("```sql").skip(1) {
        let block = block.split("```").next().unwrap_or("");
        for stmt in block.split(';') {
            let s = flat(stmt);
            if wanted.iter().any(|w| s.starts_with(w)) {
                assert!(sql.contains(&s), "0006 lacks a statement from db.md: {s}");
                found += 1;
            }
        }
    }
    assert_eq!(found, 5, "db.md statements found: {found}");
    let start = md.find("CREATE TRIGGER trg_sessions_pause_tasks").unwrap();
    let body = md[start..].split("END;").next().unwrap();
    assert!(sql.contains(&flat(&format!("{body}END"))), "trigger");
}

#[test]
fn schema_parity_0006_with_fake_agentd() {
    let root = repo_root();
    let names = [
        "0001_init.sql",
        "0002_bar.sql",
        "0003_sessions.sql",
        "0004_chat.sql",
        "0005_events_gateway.sql",
        "0006_tasks.sql",
    ];
    // the mock's file, and if there is none, the `agentd/migrations` file that the mock's runner embeds itself
    let pair = |dir: &str| {
        names
            .iter()
            .map(|n| {
                std::fs::read_to_string(root.join(dir).join(n))
                    .or_else(|_| std::fs::read_to_string(root.join("agentd/migrations").join(n)))
                    .unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        common::normalized_schema(&pair("agentd/migrations")),
        common::normalized_schema(&pair("fake-agentd/migrations"))
    );
}
