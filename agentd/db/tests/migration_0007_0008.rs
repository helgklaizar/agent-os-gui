//! A6-01, A6-02 (DB): migrations 0007 `modules_tunnels` and 0008 `memory_facts` (db.md sections 3.9, 3.10) through the real runner.

mod common;

use agentd_db::{MIGRATIONS, MigrateOptions, migrate, user_version};
use common::{file_conn, migrated_to, names};
use rusqlite::Connection;

fn db8() -> Connection {
    let c = migrated_to(8);
    assert_eq!(user_version(&c).unwrap(), 8);
    c
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn migration_0007_0008_apply_up_and_are_idempotent() {
    let mut c = migrated_to(6);
    let set = &MIGRATIONS[..8];
    migrate(&mut c, set, &MigrateOptions::default()).unwrap();
    assert_eq!(user_version(&c).unwrap(), 8);
    let rows: Vec<(i64, String)> = c
        .prepare(
            "SELECT version, name FROM schema_migrations WHERE version IN (7, 8) ORDER BY version",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        [
            (7, "modules_tunnels".to_string()),
            (8, "memory_facts".to_string())
        ]
    );
    let dir = tempfile::tempdir().unwrap();
    let mut f = file_conn(dir.path());
    migrate(&mut f, set, &MigrateOptions::default()).unwrap();
    let again = migrate(&mut f, set, &MigrateOptions::default()).unwrap();
    assert!(again.applied.is_empty(), "a repeated start applies nothing");
}

#[test]
fn migration_0007_0008_schema() {
    let c = db8();
    let tables = names(&c, "table");
    for t in ["modules", "tunnels", "memory_facts"] {
        assert!(tables.contains(&t.to_string()), "missing table {t}");
    }
    assert!(names(&c, "index").contains(&"idx_memory_persona".to_string()));
    assert!(names(&c, "trigger").contains(&"trg_findings_ref_modules".to_string()));
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
fn modules_and_tunnels_checks() {
    let c = db8();
    let module = |placement: &str| {
        c.execute(
            "INSERT INTO modules(id, name, manifest_json, placement) VALUES (?1, 'm', '{}', ?2)",
            rusqlite::params![format!("m-{placement}"), placement],
        )
    };
    assert!(module("window").is_err(), "placement outside the set");
    for ok in ["panel", "widget", "button"] {
        module(ok).unwrap();
    }
    let tunnel = |t: &str| c.execute("INSERT INTO tunnels(type, addr) VALUES (?1, 'host')", [t]);
    assert!(tunnel("openvpn").is_err());
    for ok in ["wireguard", "tailscale", "ssh", "cloudflare", "ngrok"] {
        tunnel(ok).unwrap();
    }
    let enabled: i64 = c
        .query_row("SELECT enabled FROM tunnels LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(enabled, 0, "tunnel is off by default");
}

#[test]
fn module_delete_nulls_finding_ref() {
    let c = db8();
    c.execute(
        "INSERT INTO modules(id, name, manifest_json) VALUES ('weather', 'w', '{}')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO observer_findings(ts, kind, severity, ref_table, ref_id, verdict)
         VALUES (1, 'module', 'high', 'modules', 'weather', 'blocked'), (1, 'module', 'low', 'modules', 'other', 'asked')",
        [],
    )
    .unwrap();
    c.execute("DELETE FROM modules WHERE id = 'weather'", [])
        .unwrap();
    let refs: Vec<Option<String>> = c
        .prepare("SELECT ref_id FROM observer_findings ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        refs,
        [None, Some("other".to_string())],
        "only the reference to the deleted module is nulled"
    );
}

fn persona(c: &Connection) -> i64 {
    c.execute("INSERT INTO personas(name, color) VALUES ('P', '#fff')", [])
        .unwrap();
    c.last_insert_rowid()
}

#[test]
fn persona_hard_delete_blocked_by_facts() {
    let c = db8();
    let p = persona(&c);
    c.execute(
        "INSERT INTO memory_facts(persona_id, text, ts) VALUES (?1, 'fact', 1)",
        [p],
    )
    .unwrap();
    assert!(
        c.execute("DELETE FROM personas WHERE id = ?1", [p])
            .is_err(),
        "RESTRICT: a persona with facts is not deleted"
    );
    c.execute("DELETE FROM memory_facts WHERE persona_id = ?1", [p])
        .unwrap();
    c.execute("DELETE FROM personas WHERE id = ?1", [p])
        .unwrap();
}

#[test]
fn memory_facts_session_delete_nulls_origin_and_source_is_checked() {
    let c = db8();
    c.execute(
        "INSERT INTO sessions(name, created_at, updated_at) VALUES ('s', 1, 1)",
        [],
    )
    .unwrap();
    let s = c.last_insert_rowid();
    c.execute(
        "INSERT INTO memory_facts(session_id, text, ts) VALUES (?1, 'shared fact', 1)",
        [s],
    )
    .unwrap();
    assert!(
        c.execute(
            "INSERT INTO memory_facts(text, source, ts) VALUES ('x', 'robot', 1)",
            []
        )
        .is_err(),
        "source outside the set"
    );
    c.execute("DELETE FROM sessions WHERE id = ?1", [s])
        .unwrap();
    let (sid, src): (Option<i64>, String) = c
        .query_row("SELECT session_id, source FROM memory_facts", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(
        (sid, src.as_str()),
        (None, "agent"),
        "the fact stayed, its origin is nulled"
    );
}
