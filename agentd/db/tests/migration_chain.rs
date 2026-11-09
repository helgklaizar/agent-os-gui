//! A7-14 (DB): migration chain from each version `0001`…`0008` to the latest: version, `foreign_key_check`, `integrity_check`
//! and the table set (db.md: twenty-six `sessions.db` tables, including the internal `schema_migrations`; not counting the internal `schema_migrations` and `sqlite_sequence`).

mod common;

use agentd_db::{MIGRATIONS, MigrateOptions, migrate, user_version};
use common::{migrated_to, names};

fn checks(c: &rusqlite::Connection, from: usize) {
    assert_eq!(
        user_version(c).unwrap(),
        MIGRATIONS.len() as i64,
        "from version {from}"
    );
    let fk: i64 = c
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        fk, 0,
        "foreign_key_check after the path from version {from}"
    );
    let ic: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        ic, "ok",
        "integrity_check after the path from version {from}"
    );
    let max: i64 = c
        .query_row("SELECT max(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(max, MIGRATIONS.len() as i64);
}

#[test]
fn migration_chain_each_version_to_10() {
    assert_eq!(MIGRATIONS.len(), 10, "migration set: 0001…0010");
    let mut shapes = Vec::new();
    for from in 1..=MIGRATIONS.len() {
        let mut c = migrated_to(from);
        migrate(&mut c, MIGRATIONS, &MigrateOptions::default()).unwrap();
        checks(&c, from);
        let mut tables = names(&c, "table");
        tables.sort();
        shapes.push(tables);
    }
    assert!(
        shapes.windows(2).all(|w| w[0] == w[1]),
        "the schema does not depend on the starting version"
    );
    let own: Vec<&str> = shapes[0]
        .iter()
        .map(String::as_str)
        .filter(|t| !matches!(*t, "schema_migrations" | "sqlite_sequence"))
        .collect();
    // db.md counts twenty-six tables: these twenty-five plus the internal `schema_migrations`.
    let expected = [
        "accounts",
        "actions_log",
        "api_token",
        "contacts",
        "events",
        "folders",
        "gw_messages",
        "indicators",
        "mcp_servers",
        "memory_facts",
        "messages",
        "modules",
        "observer_findings",
        "permission_requests",
        "permissions",
        "persona_accounts",
        "personas",
        "quota_samples",
        "quotas",
        "sessions",
        "settings",
        "task_runs",
        "tasks",
        "triggers",
        "tunnels",
    ];
    assert_eq!(own, expected);
}
