//! A7-15 (M7, load): migration to the latest version on a DB with data. From each version `0004`…`0007` (which already has `sessions` and
//! `messages`) 5000 messages are inserted, the migration brings the schema to the latest: data intact, `foreign_key_check` and
//! `integrity_check` clean. The time is printed (`--nocapture`) and written to the report.

mod common;

use std::time::Instant;

use agentd_db::{MIGRATIONS, MigrateOptions, migrate};
use common::migrated_to;

#[test]
fn migration_with_5000_messages_keeps_data_from_each_version() {
    for from in 4..MIGRATIONS.len() {
        let mut c = migrated_to(from);
        c.execute(
            "INSERT INTO sessions(name, created_at, updated_at) VALUES ('load', 1, 1)",
            [],
        )
        .unwrap();
        let tx = c.transaction().unwrap();
        for i in 0..5000 {
            tx.execute(
                "INSERT INTO messages(session_id, role, body_md, status, ts) VALUES (1, ?1, ?2, 'done', ?3)",
                rusqlite::params![if i % 2 == 0 { "user" } else { "agent" }, format!("message {i}"), i],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        let started = Instant::now();
        migrate(&mut c, MIGRATIONS, &MigrateOptions::default()).unwrap();
        eprintln!(
            "migration from version {from} to {} on 5000 messages: {} ms",
            MIGRATIONS.len(),
            started.elapsed().as_millis()
        );
        let n: i64 = c
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 5000, "from version {from}: messages intact");
        let last: String = c
            .query_row(
                "SELECT body_md FROM messages ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(last, "message 4999");
        let fk: i64 = c
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(fk, 0, "from version {from}");
        let ic: String = c
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ic, "ok", "from version {from}");
    }
}
