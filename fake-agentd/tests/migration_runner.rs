//! Migration runner (db.md section 6): versions, version rollback, atomicity.

mod common;

use common::*;
use fake_agentd::{DbError, MIGRATIONS, Migration, migrate};
use rusqlite::Connection;

const M1: Migration = Migration {
    version: 1,
    mutates_data: false,
    name: "init",
    sql: "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL);",
};

fn mem(fk: bool) -> Connection {
    let c = Connection::open_in_memory().expect("memory");
    c.pragma_update(None, "foreign_keys", fk).expect("fk");
    c
}

#[test]
fn migration_runner_rejects_unknown_version() {
    let (_d, path) = temp_db_path();
    drop(open(&path));
    let raw = Connection::open(&path).unwrap();
    raw.pragma_update(None, "user_version", latest_version() + 1)
        .unwrap();
    drop(raw);

    let err = fake_agentd::Db::open(&path).err();
    assert!(
        matches!(err, Some(DbError::UnknownVersion { db, known }) if db == latest_version() + 1 && known == latest_version()),
        "DB is newer than the code (version rollback): {err:?}"
    );
    let raw = Connection::open(&path).unwrap();
    assert_eq!(
        user_version(&raw),
        latest_version() + 1,
        "the refusal changes nothing"
    );
    assert_eq!(applied_versions(&raw), code_versions());

    let mut empty = mem(true);
    empty.pragma_update(None, "user_version", 99).unwrap();
    assert!(matches!(
        migrate(&mut empty, MIGRATIONS),
        Err(DbError::UnknownVersion { db: 99, .. })
    ));
    assert!(
        !object_exists(&empty, "table", "schema_migrations"),
        "nothing created on a new DB"
    );
}

#[test]
fn migration_runner_rejects_missing_file() {
    let (_d, path) = temp_db_path();
    drop(open(&path));
    let raw = Connection::open(&path).unwrap();
    raw.execute(
        "INSERT INTO schema_migrations(version, name, applied_at) VALUES (?1, 'ghost', 1)",
        [latest_version() + 1],
    )
    .unwrap();
    drop(raw);
    let err = fake_agentd::Db::open(&path).err();
    assert!(
        matches!(err, Some(DbError::MissingMigrationFile { version }) if version == latest_version() + 1),
        "{err:?}"
    );
}

#[test]
fn migration_runner_rejects_name_mismatch() {
    let (_d, path) = temp_db_path();
    drop(open(&path));
    let raw = Connection::open(&path).unwrap();
    raw.execute(
        "UPDATE schema_migrations SET name = 'other' WHERE version = 1",
        [],
    )
    .unwrap();
    drop(raw);
    let err = fake_agentd::Db::open(&path).err();
    assert!(
        matches!(err, Some(DbError::MigrationMismatch { version: 1 })),
        "{err:?}"
    );
}

#[test]
fn migration_runner_rejects_user_version_without_schema_row() {
    let mut c = mem(true);
    c.pragma_update(None, "user_version", 1).unwrap();
    let err = migrate(&mut c, &[M1]).err();
    assert!(
        matches!(
            err,
            Some(DbError::VersionMismatch {
                user_version: 1,
                schema_max: 0
            })
        ),
        "{err:?}"
    );
}

#[test]
fn migration_runner_heals_lagging_user_version() {
    let (_d, path) = temp_db_path();
    let db = open(&path);
    db.write(|w| {
        w.execute(
            "INSERT INTO settings(key, value_json, updated_at) VALUES ('k', '1', 1)",
            [],
        )
    })
    .unwrap();
    drop(db);
    let raw = Connection::open(&path).unwrap();
    raw.pragma_update(None, "user_version", 0).unwrap(); // a break between COMMIT and the user_version write
    drop(raw);

    let db = open(&path);
    let r = db.reader().unwrap();
    assert_eq!(user_version(&r), latest_version());
    assert_eq!(applied_versions(&r), code_versions());
    let n: i64 = r
        .query_row("SELECT count(*) FROM settings", [], |x| x.get(0))
        .unwrap_or(-1);
    assert_eq!(n, 1, "the migration was not applied again");
}

#[test]
fn migration_runner_applies_incrementally() {
    let m2 = Migration {
        version: 2,
        mutates_data: false,
        name: "more",
        sql: "CREATE TABLE t2 (x INTEGER);",
    };
    let (_d, path) = temp_db_path();
    drop(fake_agentd::Db::open_with(&path, &[MIGRATIONS[0]]).expect("v1"));
    let db = fake_agentd::Db::open_with(&path, &[MIGRATIONS[0], m2]).expect("v2");
    let r = db.reader().unwrap();
    assert_eq!(applied_versions(&r), vec![1, 2]);
    assert_eq!(user_version(&r), 2);
    assert!(object_exists(&r, "table", "t2"));
    drop(db);
    // rolling the code back to an old set: the DB is newer than the code
    let err = fake_agentd::Db::open_with(&path, &[MIGRATIONS[0]]).err();
    assert!(
        matches!(err, Some(DbError::UnknownVersion { db: 2, known: 1 })),
        "{err:?}"
    );
}

#[test]
fn migration_failure_rolls_back_and_restores_foreign_keys() {
    let mut c = mem(true);
    migrate(&mut c, &[M1]).expect("v1");
    let bad = Migration {
        version: 2,
        mutates_data: false,
        name: "bad",
        sql: "CREATE TABLE half (x); CREATE TABLE half (x);",
    };
    assert!(matches!(
        migrate(&mut c, &[M1, bad]),
        Err(DbError::Sqlite(_))
    ));
    assert_eq!(applied_versions(&c), vec![1]);
    assert_eq!(user_version(&c), 1);
    assert!(
        !object_exists(&c, "table", "half"),
        "partial migration rolled back"
    );
    assert_eq!(
        pragma_text(&c, "foreign_keys"),
        "1",
        "foreign_keys ON again after a failure"
    );
}

#[test]
fn migration_foreign_key_check_failure_rolls_back() {
    let mut c = mem(true);
    migrate(&mut c, &[M1]).expect("v1");
    let m2 = Migration {
        version: 2,
        mutates_data: false,
        name: "orphan",
        sql: "CREATE TABLE p (id INTEGER PRIMARY KEY); CREATE TABLE ch (pid INTEGER REFERENCES p(id)); INSERT INTO ch VALUES (5);",
    };
    let err = migrate(&mut c, &[M1, m2]).err();
    assert!(
        matches!(err, Some(DbError::ForeignKeyViolation { version: 2 })),
        "{err:?}"
    );
    assert_eq!(applied_versions(&c), vec![1]);
    assert!(!object_exists(&c, "table", "ch"));
    assert_eq!(pragma_text(&c, "foreign_keys"), "1");
}

#[test]
fn migration_runner_rejects_unordered_set() {
    let mut c = mem(true);
    let m2 = Migration {
        version: 2,
        mutates_data: false,
        name: "two",
        sql: "SELECT 1;",
    };
    assert!(matches!(
        migrate(&mut c, &[m2, M1]),
        Err(DbError::InvalidMigrationSet)
    ));
    assert!(matches!(
        migrate(&mut c, &[M1, M1]),
        Err(DbError::InvalidMigrationSet)
    ));
}
