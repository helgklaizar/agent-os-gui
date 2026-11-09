//! A3-01 (DB): migration 0004 `chat` (db.md sections 3.2, 3.6, 3.7, 3.11, section 5) through the real runner.

mod common;

use agentd_db::{MIGRATIONS, MigrateOptions, migrate, user_version};
use common::{migrated_to, names, normalized_schema, repo_root};
use rusqlite::Connection;

fn db4() -> Connection {
    let c = migrated_to(4);
    assert_eq!(user_version(&c).unwrap(), 4, "DB brought up to version 4");
    c
}

fn session(c: &Connection) -> i64 {
    c.execute(
        "INSERT INTO sessions(name, created_at, updated_at) VALUES ('s', 1, 1)",
        [],
    )
    .unwrap();
    c.last_insert_rowid()
}

fn message(c: &Connection, session_id: i64, client_id: Option<&str>) -> rusqlite::Result<usize> {
    c.execute(
        "INSERT INTO messages(session_id, client_id, role, body_md, ts) VALUES (?1, ?2, 'user', 'x', 1)",
        rusqlite::params![session_id, client_id],
    )
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn migration_0004_applies_up() {
    let mut c = migrated_to(3);
    assert_eq!(user_version(&c).unwrap(), 3);
    migrate(&mut c, &MIGRATIONS[..4], &MigrateOptions::default()).unwrap();
    assert_eq!(user_version(&c).unwrap(), 4);
    let (v, name): (i64, String) = c
        .query_row(
            "SELECT version, name FROM schema_migrations WHERE version = 4",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((v, name.as_str()), (4, "chat"));
}

#[test]
fn migration_0004_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = common::file_conn(dir.path());
    let opts = MigrateOptions::default();
    migrate(&mut c, &MIGRATIONS[..4], &opts).unwrap();
    assert_eq!(user_version(&c).unwrap(), 4);
    let stamp = |c: &Connection| -> Vec<i64> {
        c.prepare("SELECT applied_at FROM schema_migrations ORDER BY version")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let before = stamp(&c);
    let out = migrate(&mut c, &MIGRATIONS[..4], &opts).unwrap();
    assert!(out.applied.is_empty(), "a repeated start applies nothing");
    assert_eq!(user_version(&c).unwrap(), 4);
    assert_eq!(stamp(&c), before);
}

#[test]
fn migration_0004_schema() {
    let c = db4();
    let tables = names(&c, "table");
    for t in [
        "messages",
        "permissions",
        "actions_log",
        "observer_findings",
    ] {
        assert!(tables.contains(&t.to_string()), "missing table {t}");
    }
    let idx = names(&c, "index");
    for i in [
        "idx_messages_session",
        "idx_messages_client",
        "idx_actions_ts",
        "idx_actions_session",
        "idx_observer_ts",
    ] {
        assert!(idx.contains(&i.to_string()), "missing index {i}");
    }
    let trg = names(&c, "trigger");
    for t in [
        "trg_sessions_cleanup",
        "trg_personas_cleanup",
        "trg_findings_ref_actions",
    ] {
        assert!(trg.contains(&t.to_string()), "missing trigger {t}");
    }
}

#[test]
fn migration_0004_foreign_keys() {
    let c = db4();
    let s = session(&c);
    message(&c, s, Some("a")).unwrap();
    c.execute(
        "INSERT INTO actions_log(ts, actor, session_id, action) VALUES (1,'agent',?1,'shell')",
        [s],
    )
    .unwrap();
    assert_eq!(
        count(&c, "SELECT count(*) FROM pragma_foreign_key_check"),
        0
    );
}

#[test]
fn migration_0004_integrity() {
    let c = db4();
    let r: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(r, "ok");
}

/// Every DDL statement of sections 3.2, 3.6, 3.7 and 3.11 (only what belongs to 0004) appears verbatim in the migration file.
#[test]
fn migration_0004_matches_db_md() {
    let root = repo_root();
    let md = std::fs::read_to_string(root.join("docs/reference/db.md")).unwrap();
    let flat = |s: &str| -> String {
        s.lines()
            .map(|l| l.split("--").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let sql = flat(&std::fs::read_to_string(root.join("agentd/migrations/0004_chat.sql")).unwrap());
    let mut found = 0;
    for block in md.split("```sql").skip(1) {
        let block = block.split("```").next().unwrap_or("");
        for stmt in block.split(';') {
            let s = flat(stmt);
            let wanted = [
                "CREATE TABLE messages",
                "CREATE INDEX idx_messages_session",
                "CREATE UNIQUE INDEX idx_messages_client",
                "CREATE TABLE permissions",
                "CREATE TABLE actions_log",
                "CREATE INDEX idx_actions_ts",
                "CREATE INDEX idx_actions_session",
                "CREATE TABLE observer_findings",
                "CREATE INDEX idx_observer_ts",
            ];
            if wanted.iter().any(|w| s.starts_with(w)) {
                assert!(sql.contains(&s), "0004 lacks a statement from db.md: {s}");
                found += 1;
            }
        }
    }
    assert_eq!(found, 9, "db.md statements found: {found}");
    for trg in [
        "trg_sessions_cleanup",
        "trg_personas_cleanup",
        "trg_findings_ref_actions",
    ] {
        let start = md.find(&format!("CREATE TRIGGER {trg}")).unwrap();
        let body = md[start..].split("END;").next().unwrap();
        assert!(sql.contains(&flat(&format!("{body}END"))), "trigger {trg}");
    }
}

#[test]
fn schema_parity_0004_with_fake_agentd() {
    let root = repo_root();
    let fake = root.join("fake-agentd/migrations/0004_chat.sql");
    if !fake.exists() {
        // The fake-agentd catalog is out of scope for A3 (D36): the comparison turns on once 0003 and 0004 appear there (request in the report).
        eprintln!("fake-agentd/migrations/0004_chat.sql missing: comparison skipped");
        return;
    }
    let pair = |dir: &str| {
        [
            "0001_init.sql",
            "0002_bar.sql",
            "0003_sessions.sql",
            "0004_chat.sql",
        ]
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
        normalized_schema(&pair("agentd/migrations")),
        normalized_schema(&pair("fake-agentd/migrations"))
    );
}
