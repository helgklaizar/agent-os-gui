//! A5-01 (DB): migration 0005 `events_gateway` (db.md sections 3.3, 3.11) through the real runner.

mod common;

use agentd_db::{MIGRATIONS, MigrateOptions, migrate, user_version};
use common::{migrated_to, names, normalized_schema, repo_root};
use rusqlite::Connection;

fn db5() -> Connection {
    let c = migrated_to(5);
    assert_eq!(user_version(&c).unwrap(), 5, "DB brought up to version 5");
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

fn account(c: &Connection, slug: &str) -> i64 {
    c.execute(
        "INSERT INTO accounts(slug, type, label, created_at) VALUES (?1, 'tg', 'tg', 1)",
        [slug],
    )
    .unwrap();
    c.last_insert_rowid()
}

fn event(c: &Connection, sid: Option<i64>, dedup: Option<&str>) -> rusqlite::Result<usize> {
    c.execute(
        "INSERT OR IGNORE INTO events(kind, sub, text, session_id, dedup_key, ts) VALUES ('msg','sys','t',?1,?2,1)",
        rusqlite::params![sid, dedup],
    )
}

fn gw(c: &Connection, acc: i64, ext: &str) -> rusqlite::Result<usize> {
    c.execute(
        "INSERT INTO gw_messages(account_id, ext_id, direction, body, ts) VALUES (?1, ?2, 'in', 'b', 1)",
        rusqlite::params![acc, ext],
    )
}

#[test]
fn migration_0005_applies_up() {
    let mut c = migrated_to(4);
    assert_eq!(user_version(&c).unwrap(), 4);
    let set = &MIGRATIONS[..5.min(MIGRATIONS.len())];
    migrate(&mut c, set, &MigrateOptions::default()).unwrap();
    assert_eq!(user_version(&c).unwrap(), 5);
    let (v, name): (i64, String) = c
        .query_row(
            "SELECT version, name FROM schema_migrations WHERE version = 5",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((v, name.as_str()), (5, "events_gateway"));
}

#[test]
fn migration_0005_schema() {
    let c = db5();
    let tables = names(&c, "table");
    for t in ["events", "contacts", "gw_messages"] {
        assert!(tables.contains(&t.to_string()), "missing table {t}");
    }
    let idx = names(&c, "index");
    for i in [
        "idx_events_filter",
        "idx_events_session",
        "idx_events_dedup",
        "idx_gw_unread",
    ] {
        assert!(idx.contains(&i.to_string()), "missing index {i}");
    }
    assert!(names(&c, "trigger").contains(&"trg_findings_ref_gw".to_string()));
    assert!(
        c.execute(
            "INSERT INTO events(kind, sub, text, ts) VALUES ('x','sys','t',1)",
            []
        )
        .is_err(),
        "kind outside the set"
    );
    assert!(
        c.execute(
            "INSERT INTO events(kind, sub, level, text, ts) VALUES ('ai','agent','fatal','t',1)",
            []
        )
        .is_err(),
        "level outside the set"
    );
}

#[test]
fn migration_0005_foreign_keys_integrity() {
    let c = db5();
    let s = session(&c);
    let a = account(&c, "tg1");
    event(&c, Some(s), None).unwrap();
    gw(&c, a, "1").unwrap();
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
fn events_dedup_unique_index() {
    let c = db5();
    assert_eq!(event(&c, None, Some("cpu:1")).unwrap(), 1);
    assert_eq!(
        event(&c, None, Some("cpu:1")).unwrap(),
        0,
        "the second is ignored"
    );
    assert_eq!(event(&c, None, Some("cpu:2")).unwrap(), 1);
    assert_eq!(event(&c, None, None).unwrap(), 1);
    assert_eq!(
        event(&c, None, None).unwrap(),
        1,
        "NULL is not deduplicated"
    );
    assert_eq!(count(&c, "SELECT count(*) FROM events"), 4);
}

#[test]
fn delete_session_keeps_system_events() {
    let c = db5();
    let s = session(&c);
    event(&c, Some(s), None).unwrap();
    event(&c, None, None).unwrap();
    c.execute("DELETE FROM sessions WHERE id = ?1", [s])
        .unwrap();
    assert_eq!(count(&c, "SELECT count(*) FROM events"), 2, "events stayed");
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM events WHERE session_id IS NOT NULL"
        ),
        0
    );
}

#[test]
fn gw_messages_unique_ext_id() {
    let c = db5();
    let (a, b) = (account(&c, "a"), account(&c, "b"));
    gw(&c, a, "x").unwrap();
    assert!(gw(&c, a, "x").is_err(), "duplicate (account_id, ext_id)");
    gw(&c, b, "x").unwrap();
}

#[test]
fn gw_messages_restrict_account_delete() {
    let c = db5();
    let a = account(&c, "a");
    gw(&c, a, "x").unwrap();
    assert!(
        c.execute("DELETE FROM accounts WHERE id = ?1", [a])
            .is_err()
    );
    assert_eq!(count(&c, "SELECT count(*) FROM gw_messages"), 1);
}

#[test]
fn gw_route_nulled_on_session_and_contact_delete() {
    let c = db5();
    let (a, s) = (account(&c, "a"), session(&c));
    c.execute("INSERT INTO contacts(name) VALUES ('v')", [])
        .unwrap();
    let ct = c.last_insert_rowid();
    c.execute(
        "INSERT INTO gw_messages(account_id, ext_id, direction, contact_id, body, routed_session_id, ts) VALUES (?1,'1','in',?2,'b',?3,1)",
        rusqlite::params![a, ct, s],
    )
    .unwrap();
    c.execute("DELETE FROM sessions WHERE id = ?1", [s])
        .unwrap();
    c.execute("DELETE FROM contacts WHERE id = ?1", [ct])
        .unwrap();
    let (r, k): (Option<i64>, Option<i64>) = c
        .query_row(
            "SELECT routed_session_id, contact_id FROM gw_messages",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((r, k), (None, None));
}

#[test]
fn finding_ref_nulled_when_gw_message_deleted() {
    let c = db5();
    let a = account(&c, "a");
    gw(&c, a, "x").unwrap();
    let id = c.last_insert_rowid();
    c.execute(
        "INSERT INTO observer_findings(ts, kind, severity, ref_table, ref_id, verdict) VALUES (1,'injection','high','gw_messages',?1,'blocked')",
        [id.to_string()],
    )
    .unwrap();
    c.execute("DELETE FROM gw_messages WHERE id = ?1", [id])
        .unwrap();
    let r: Option<String> = c
        .query_row("SELECT ref_id FROM observer_findings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(r, None);
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

/// The db.md statements for sections 3.3 and 3.11 (events, contacts, gateway messages, trigger) appear verbatim in the migration file.
#[test]
fn migration_0005_matches_db_md() {
    let root = repo_root();
    let md = std::fs::read_to_string(root.join("docs/reference/db.md")).unwrap();
    let sql = flat(
        &std::fs::read_to_string(root.join("agentd/migrations/0005_events_gateway.sql"))
            .expect("migration 0005 file"),
    );
    let wanted = [
        "CREATE TABLE events",
        "CREATE INDEX idx_events_filter",
        "CREATE INDEX idx_events_session",
        "CREATE UNIQUE INDEX idx_events_dedup",
        "CREATE TABLE contacts",
        "CREATE TABLE gw_messages",
        "CREATE INDEX idx_gw_unread",
    ];
    let mut found = 0;
    for block in md.split("```sql").skip(1) {
        let block = block.split("```").next().unwrap_or("");
        for stmt in block.split(';') {
            let s = flat(stmt);
            if wanted.iter().any(|w| s.starts_with(w)) {
                assert!(sql.contains(&s), "0005 lacks a statement from db.md: {s}");
                found += 1;
            }
        }
    }
    assert_eq!(found, 7, "db.md statements found: {found}");
    let start = md.find("CREATE TRIGGER trg_findings_ref_gw").unwrap();
    let body = md[start..].split("END;").next().unwrap();
    assert!(sql.contains(&flat(&format!("{body}END"))), "trigger");
}

#[test]
fn schema_parity_0005_with_fake_agentd() {
    let root = repo_root();
    let fake = root.join("fake-agentd/migrations/0005_events_gateway.sql");
    if !fake.exists() {
        // The fake-agentd catalog is out of scope for A5 (D36): the comparison turns on once 0003…0006 appear there (request A5-14).
        eprintln!("fake-agentd/migrations/0005_events_gateway.sql missing: comparison skipped");
        return;
    }
    let names = [
        "0001_init.sql",
        "0002_bar.sql",
        "0003_sessions.sql",
        "0004_chat.sql",
        "0005_events_gateway.sql",
    ];
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
        normalized_schema(&pair("agentd/migrations")),
        normalized_schema(&pair("fake-agentd/migrations"))
    );
}
