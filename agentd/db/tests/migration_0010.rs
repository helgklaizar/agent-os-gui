//! A7-fix (DB): migration 0010 `permissions_mcp`: tables `permission_requests` and `mcp_servers` (db.md section 3.14, D46.8/D46.9).
//! The DDL in the migration file is verbatim the `sql` block from db.md; the full schema gives twenty-six `sessions.db` tables per db.md
//! (twenty-five own plus the internal `schema_migrations`).

mod common;

use agentd_db::MIGRATIONS;
use common::{migrated_to, names};
use rusqlite::Connection;

fn db_md_ddl() -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/db.md");
    let text = std::fs::read_to_string(p).unwrap();
    let sec = text
        .split("### 3.14 ")
        .nth(1)
        .expect("section 3.14 in db.md")
        .split("\n## ")
        .next()
        .unwrap();
    let start = sec.find("```sql\n").expect("sql block") + "```sql\n".len();
    let end = start + sec[start..].find("```").unwrap();
    sec[start..end].trim().to_string()
}

#[test]
fn migration_0010_is_registered_after_web_token() {
    assert_eq!(MIGRATIONS.len(), 10);
    let m = MIGRATIONS[9];
    assert_eq!((m.version, m.name), (10, "permissions_mcp"));
    assert!(!m.mutates_data && !m.recreates_tables);
}

#[test]
fn migration_0010_ddl_is_verbatim_from_db_md() {
    let ddl = db_md_ddl();
    assert!(ddl.contains("CREATE TABLE permission_requests"));
    let sql = MIGRATIONS.get(9).map_or("", |m| m.sql);
    assert!(
        sql.contains(&ddl),
        "migration 0010 DDL does not match db.md section 3.14"
    );
}

#[test]
fn schema_has_exactly_26_tables() {
    let c = migrated_to(MIGRATIONS.len());
    let tables: Vec<String> = names(&c, "table")
        .into_iter()
        .filter(|t| t != "sqlite_sequence")
        .collect();
    assert_eq!(tables.len(), 26, "{tables:?}");
    for t in ["permission_requests", "mcp_servers", "schema_migrations"] {
        assert!(tables.contains(&t.to_string()), "missing {t}");
    }
    assert!(names(&c, "index").contains(&"idx_permreq_pending".to_string()));
}

fn seeded() -> Connection {
    let c = migrated_to(MIGRATIONS.len());
    c.execute(
        "INSERT INTO sessions(name, created_at, updated_at) VALUES ('s', 1, 1)",
        [],
    )
    .unwrap();
    c
}

#[test]
fn permission_requests_constraints_and_cascade() {
    let c = seeded();
    let ins = |status: &str| {
        c.execute(
            "INSERT INTO permission_requests(session_id, action, options_json, status, expires_at, created_at) VALUES (1, 'shell', '[]', ?1, 10, 1)",
            [status],
        )
    };
    assert!(ins("pending").is_ok());
    assert!(ins("bogus").is_err(), "status outside CHECK is rejected");
    let default: String = c
        .query_row("SELECT status FROM permission_requests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(default, "pending");
    c.execute("DELETE FROM sessions WHERE id = 1", []).unwrap();
    let left: i64 = c
        .query_row("SELECT count(*) FROM permission_requests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0, "session requests are deleted by cascade");
}

#[test]
fn mcp_servers_constraints() {
    let c = seeded();
    let ins = |name: &str, transport: &str| {
        c.execute(
            "INSERT INTO mcp_servers(name, transport) VALUES (?1, ?2)",
            [name, transport],
        )
    };
    assert!(ins("a", "stdio").is_ok());
    assert!(ins("a", "http").is_err(), "name is unique");
    assert!(
        ins("b", "ws").is_err(),
        "transport outside CHECK is rejected"
    );
    let enabled: i64 = c
        .query_row("SELECT enabled FROM mcp_servers WHERE name='a'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(enabled, 1);
}
