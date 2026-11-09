//! A1-02: runner behavior on synthetic migrations and on the fixture.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use agentd_db::{DbError, MigrateOptions, Migration, RunnerEvent, migrate, user_version};
use common::{data_migration, file_conn, mem, with_extra};

const M2: Migration = data_migration(
    2,
    "add_notes",
    "CREATE TABLE notes(id INTEGER PRIMARY KEY, t TEXT); INSERT INTO notes(t) VALUES ('a');",
);
const M3: Migration = data_migration(
    3,
    "add_notes2",
    "CREATE TABLE notes2(id INTEGER PRIMARY KEY);",
);
const M4: Migration = data_migration(
    4,
    "add_notes3",
    "CREATE TABLE notes3(id INTEGER PRIMARY KEY);",
);

fn opts_with_backups(dir: &std::path::Path) -> MigrateOptions {
    MigrateOptions {
        backup_dir: Some(dir.join("migration-backups")),
        date_stamp: Arc::new(|| "20261102".to_string()),
        ..MigrateOptions::default()
    }
}

#[test]
fn db_newer_than_code_refuses() {
    let mut c = mem();
    migrate(&mut c, common::first(), &MigrateOptions::default()).unwrap();
    c.pragma_update(None, "user_version", 7).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let opts = MigrateOptions {
        on_event: Some(Arc::new(move |e: &RunnerEvent| {
            sink.lock().unwrap().push(e.clone())
        })),
        ..MigrateOptions::default()
    };
    let err = migrate(&mut c, common::first(), &opts).unwrap_err();
    assert!(
        matches!(err, DbError::NewerThanCode { db: 7, known: 1 }),
        "{err:?}"
    );
    assert!(events.lock().unwrap().contains(&RunnerEvent::Error {
        code: "db_newer_than_code"
    }));
    assert_eq!(user_version(&c).unwrap(), 7, "DB untouched");
}

/// A `user_version` write: `pragma_update(None, "user_version", ..)` or `PRAGMA user_version = ..` (spaces do not matter).
fn writes_user_version(text: &str) -> bool {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    t.contains("None,\"user_version\"") || t.contains("PRAGMAuser_version=")
}

#[test]
fn writes_user_version_detector_catches_bad_samples() {
    assert!(writes_user_version(
        "c.pragma_update(\n None,\n \"user_version\", 2)"
    ));
    assert!(writes_user_version(
        "c.execute_batch(\"PRAGMA user_version = 3\")"
    ));
    assert!(writes_user_version(
        "c.pragma_update_and_check(None, \"user_version\", 1, |r| r.get(0))"
    ));
    assert!(!writes_user_version(
        "#[error(\"user_version {db}\")] query_row(\"PRAGMA user_version\")"
    ));
}

#[test]
fn user_version_set_only_by_runner() {
    // Static check of the core sources: mentioning user_version is allowed only in the runner.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut offenders = Vec::new();
    for krate in std::fs::read_dir(&root).unwrap().map(Result::unwrap) {
        let src = krate.path().join("src");
        if !src.is_dir() {
            continue;
        }
        let mut stack = vec![src];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().map(Result::unwrap) {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs")
                    && p.file_name().is_some_and(|n| n != "migrate.rs")
                    && writes_user_version(&std::fs::read_to_string(&p).unwrap())
                {
                    offenders.push(p);
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "user_version outside the runner: {offenders:?}"
    );
}

#[test]
fn snapshot_before_data_migration() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = file_conn(dir.path());
    migrate(&mut c, common::first(), &MigrateOptions::default()).unwrap();
    c.execute(
        "INSERT INTO settings(key, value_json, updated_at) VALUES ('theme', '\"dark\"', 1)",
        [],
    )
    .unwrap();
    let opts = opts_with_backups(dir.path());
    let out = migrate(&mut c, &with_extra(&[M2]), &opts).unwrap();
    assert_eq!(out.applied, vec![2]);
    let snap = dir.path().join("migration-backups/pre-migration-2.db");
    assert_eq!(out.snapshots, vec![snap.clone()]);
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&snap), 0o600);
    assert_eq!(mode(snap.parent().unwrap()), 0o700);
    let s = rusqlite::Connection::open(&snap).unwrap();
    let n: i64 = s
        .query_row("SELECT count(*) FROM settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1, "snapshot taken before the migration, data in place");
    let has_notes: i64 = s
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='notes'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_notes, 0, "snapshot before migration 2 is applied");
}

#[test]
fn snapshot_retention_two_latest() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = file_conn(dir.path());
    migrate(&mut c, common::first(), &MigrateOptions::default()).unwrap();
    migrate(
        &mut c,
        &with_extra(&[M2, M3, M4]),
        &opts_with_backups(dir.path()),
    )
    .unwrap();
    let mut names: Vec<String> = std::fs::read_dir(dir.path().join("migration-backups"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "pre-migration-2.db.bak-20261102",
            "pre-migration-3.db",
            "pre-migration-4.db"
        ],
        "the two latest snapshots stay, the old one is moved, not deleted"
    );
}

#[test]
fn migration_refuses_low_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = file_conn(dir.path());
    migrate(&mut c, common::first(), &MigrateOptions::default()).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let opts = MigrateOptions {
        free_space: Arc::new(|_| Ok(1)),
        on_event: Some(Arc::new(move |e: &RunnerEvent| {
            sink.lock().unwrap().push(e.clone())
        })),
        ..opts_with_backups(dir.path())
    };
    let err = migrate(&mut c, &with_extra(&[M2]), &opts).unwrap_err();
    assert!(matches!(err, DbError::LowDisk { free: 1, .. }), "{err:?}");
    assert!(
        events
            .lock()
            .unwrap()
            .contains(&RunnerEvent::Error { code: "low_disk" })
    );
    assert_eq!(user_version(&c).unwrap(), 1);
    let t: i64 = c
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='notes'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(t, 0, "migration did not start");
}

#[test]
fn table_recreate_checks_foreign_keys() {
    let mut c = mem();
    migrate(&mut c, common::first(), &MigrateOptions::default()).unwrap();
    let bad = Migration {
        version: 2,
        name: "bad_recreate",
        sql: "CREATE TABLE p(id INTEGER PRIMARY KEY); CREATE TABLE ch(pid INTEGER REFERENCES p(id)); INSERT INTO ch VALUES (5);",
        mutates_data: false,
        recreates_tables: true,
    };
    let err = migrate(&mut c, &with_extra(&[bad]), &MigrateOptions::default()).unwrap_err();
    assert!(
        matches!(err, DbError::ForeignKeyViolation { version: 2 }),
        "{err:?}"
    );
    assert_eq!(user_version(&c).unwrap(), 1);
    let t: i64 = c
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('p','ch')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(t, 0, "rollback: no tables");
    let fk_on: i64 = c
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fk_on, 1, "foreign_keys is on again");
}

#[test]
fn failed_migration_rolls_back_and_keeps_version() {
    let mut c = mem();
    migrate(&mut c, common::first(), &MigrateOptions::default()).unwrap();
    let bad = data_migration(
        2,
        "broken",
        "CREATE TABLE half(id INTEGER); THIS IS NOT SQL;",
    );
    assert!(migrate(&mut c, &with_extra(&[bad]), &MigrateOptions::default()).is_err());
    assert_eq!(user_version(&c).unwrap(), 1);
    let t: i64 = c
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='half'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(t, 0);
}

#[test]
fn applied_migration_with_other_name_is_rejected() {
    let mut c = mem();
    migrate(&mut c, common::first(), &MigrateOptions::default()).unwrap();
    c.execute(
        "UPDATE schema_migrations SET name = 'other' WHERE version = 1",
        [],
    )
    .unwrap();
    let err = migrate(&mut c, common::first(), &MigrateOptions::default()).unwrap_err();
    assert!(
        matches!(err, DbError::MigrationMismatch { version: 1 }),
        "{err:?}"
    );
}

#[test]
fn unordered_migration_set_is_rejected() {
    let mut c = mem();
    let set = [M3, M2];
    let err = migrate(&mut c, &set, &MigrateOptions::default()).unwrap_err();
    assert!(matches!(err, DbError::InvalidMigrationSet), "{err:?}");
    assert_eq!(user_version(&c).unwrap(), 0);
}

#[test]
fn progress_events_reported_in_order() {
    let mut c = mem();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let opts = MigrateOptions {
        on_event: Some(Arc::new(move |e: &RunnerEvent| {
            sink.lock().unwrap().push(e.clone())
        })),
        ..MigrateOptions::default()
    };
    migrate(&mut c, &with_extra(&[M2]), &opts).unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            RunnerEvent::Started { from: 0, to: 2 },
            RunnerEvent::Applied {
                version: 1,
                name: "init".into()
            },
            RunnerEvent::Applied {
                version: 2,
                name: "add_notes".into()
            },
            RunnerEvent::Finished { version: 2 },
        ]
    );
}

#[test]
fn schema_parity_with_fake_agentd() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fake = root.join("fake-agentd/migrations/0001_init.sql");
    if !fake.exists() {
        return; // no copy alongside: nothing to compare (A1-02)
    }
    let ours = std::fs::read_to_string(root.join("agentd/migrations/0001_init.sql")).unwrap();
    let theirs = std::fs::read_to_string(&fake).unwrap();
    assert_eq!(
        normalized_schema(&ours),
        normalized_schema(&theirs),
        "agentd and fake-agentd schemas diverged"
    );
}

fn normalized_schema(sql: &str) -> Vec<String> {
    let c = mem();
    c.execute_batch(sql).unwrap();
    let mut stmt = c
        .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name")
        .unwrap();
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows.into_iter()
        .map(|s| {
            s.lines()
                .map(|l| {
                    l.split("--")
                        .next()
                        .unwrap_or("")
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[test]
fn schema_parity_detects_difference() {
    let a = "CREATE TABLE t(a INTEGER); -- c";
    let b = "CREATE TABLE t(a INTEGER, b TEXT);";
    assert_ne!(normalized_schema(a), normalized_schema(b));
    assert_eq!(
        normalized_schema(a),
        normalized_schema("CREATE TABLE t(a   INTEGER);")
    );
}
