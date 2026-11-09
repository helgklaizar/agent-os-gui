//! A2-01: migration 0002 `bar` (db.md section 3.4 `accounts`, section 3.5, section 3.12 `indicators`) through the real runner.

mod common;

use agentd_db::{MIGRATIONS, MigrateOptions, migrate, user_version};
use common::{migrated_to, names, repo_root};

#[test]
fn migration_0002_applies_up() {
    let mut c = migrated_to(1);
    assert_eq!(user_version(&c).unwrap(), 1);
    let two = &MIGRATIONS[..MIGRATIONS.len().min(2)];
    migrate(&mut c, two, &MigrateOptions::default()).unwrap();
    assert_eq!(user_version(&c).unwrap(), 2, "user_version after 0002");
    let rows: Vec<(i64, String)> = c
        .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows, vec![(1, "init".to_string()), (2, "bar".to_string())]);
}

#[test]
fn migration_0002_schema() {
    let c = migrated_to(2);
    assert_eq!(user_version(&c).unwrap(), 2);
    let tables = names(&c, "table");
    for t in ["accounts", "quotas", "quota_samples", "indicators"] {
        assert!(tables.contains(&t.to_string()), "missing table {t}");
    }
    // The indicator state columns are created right away (D38).
    let cols: Vec<String> = c
        .prepare("SELECT name FROM pragma_table_info('indicators')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for col in ["last_state", "transition_seq", "last_change_at"] {
        assert!(cols.contains(&col.to_string()), "missing column {col}");
    }
    c.execute(
        "INSERT INTO accounts(slug, type, label, created_at) VALUES ('cl1','anthropic','k',1)",
        [],
    )
    .unwrap();
    assert!(
        c.execute(
            "INSERT INTO accounts(slug, type, label, status, created_at) VALUES ('x','wa','k','bad',1)",
            []
        )
        .is_err(),
        "account status CHECK"
    );
    assert!(
        c.execute(
            "INSERT INTO indicators(id, last_state) VALUES ('cpu','red')",
            []
        )
        .is_err(),
        "indicator state CHECK"
    );
}

#[test]
fn schema_parity_0002_with_fake_agentd() {
    let root = repo_root();
    // the mock keeps no copy of 0002: the runner embeds the file `agentd/migrations/0002_bar.sql`
    assert!(
        !root.join("fake-agentd/migrations/0002_bar.sql").exists(),
        "copy of 0002 in fake-agentd/migrations"
    );
    let runner = std::fs::read_to_string(root.join("fake-agentd/src/db/migrate.rs")).unwrap();
    assert!(runner.contains("agentd/migrations/0002_bar.sql"));
}
