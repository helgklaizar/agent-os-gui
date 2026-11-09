//! Free space at least twice the DB size before a migration (db.md section 6); otherwise the migration does not start,
//! the DB is untouched, an `error` event is written.

mod common;

use std::fs;
use std::path::Path;

use common::*;
use fake_agentd::{Db, DbError, Migration, migrate_with_space};
use rusqlite::Connection;

const M1: Migration = Migration {
    version: 1,
    mutates_data: false,
    name: "init",
    sql: "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL);\n\
          CREATE TABLE parent (id INTEGER PRIMARY KEY, v TEXT NOT NULL);\n\
          INSERT INTO parent(id, v) VALUES (1, 'a');",
};

const M2: Migration = Migration {
    version: 2,
    mutates_data: false,
    name: "more",
    sql: "CREATE TABLE notes (id INTEGER PRIMARY KEY);",
};

fn db_v1() -> (tempfile::TempDir, std::path::PathBuf) {
    let (dir, path) = temp_db_path();
    drop(Db::open_with(&path, &[M1]).expect("v1"));
    (dir, path)
}

fn size_of(path: &Path) -> u64 {
    fs::metadata(path).expect("DB size").len()
}

#[test]
fn low_disk_blocks_migration_and_leaves_db_untouched() {
    let (_d, path) = db_v1();
    let size = size_of(&path);
    let err = Db::open_with_space(&path, &[M1, M2], &|_| Ok(1)).err();
    assert!(
        matches!(err, Some(DbError::LowDisk { free: 1, need }) if need == size * 2),
        "exactly 2 x the DB size ({size}) is needed: {err:?}"
    );
    let raw = Connection::open(&path).expect("open");
    assert_eq!(user_version(&raw), 1, "the version did not move");
    assert!(
        !object_exists(&raw, "table", "notes"),
        "the migration did not start"
    );
}

#[test]
fn low_disk_reports_error_event_without_paths() {
    let (_d, path) = db_v1();
    assert!(Db::open_with_space(&path, &[M1, M2], &|_| Ok(1)).is_err());
    let mut name = path.as_os_str().to_owned();
    name.push(".events.jsonl");
    let text = fs::read_to_string(std::path::PathBuf::from(name)).unwrap_or_default();
    let v: serde_json::Value =
        serde_json::from_str(text.lines().last().unwrap_or_default()).unwrap_or_default();
    assert_eq!(v["type"], "error", "error event: {text:?}");
    assert_eq!(v["code"], "low_disk");
    assert!(
        !text.contains(path.to_string_lossy().as_ref()),
        "no paths in the event"
    );
}

#[test]
fn exactly_double_size_is_enough_and_one_byte_less_is_not() {
    let (_d, path) = db_v1();
    let need = match Db::open_with_space(&path, &[M1, M2], &|_| Ok(0)).err() {
        Some(DbError::LowDisk { need, .. }) => need,
        other => panic!("LowDisk expected: {other:?}"),
    };
    assert!(need > 0);
    assert!(matches!(
        Db::open_with_space(&path, &[M1, M2], &|_| Ok(need - 1)).err(),
        Some(DbError::LowDisk { .. })
    ));
    drop(Db::open_with_space(&path, &[M1, M2], &|_| Ok(need)).expect("exactly 2x is enough"));
    let raw = Connection::open(&path).expect("open");
    assert_eq!(user_version(&raw), 2);
}

#[test]
fn free_space_is_asked_for_the_db_directory() {
    let (_d, path) = db_v1();
    let seen = std::cell::RefCell::new(Vec::new());
    drop(
        Db::open_with_space(&path, &[M1, M2], &|p| {
            seen.borrow_mut().push(p.to_path_buf());
            Ok(u64::MAX)
        })
        .expect("migration"),
    );
    // sqlite returns the path with symlinks resolved (on macOS `/var` -> `/private/var`)
    let dir = fs::canonicalize(path.parent().expect("dir")).expect("canonical");
    let asked: Vec<_> = seen
        .into_inner()
        .iter()
        .map(|p| fs::canonicalize(p).expect("canonical"))
        .collect();
    assert_eq!(asked, vec![dir]);
}

#[test]
fn no_check_when_nothing_to_migrate() {
    let (_d, path) = db_v1();
    drop(Db::open_with_space(&path, &[M1], &|_| Ok(0)).expect("nothing to migrate"));
}

#[test]
fn no_check_for_in_memory_database() {
    let mut mem = Connection::open_in_memory().expect("mem");
    migrate_with_space(&mut mem, &[M1], &|_| Ok(0)).expect("in-memory DB without a check");
    assert_eq!(user_version(&mem), 1);
}

#[test]
fn free_space_error_blocks_migration() {
    let (_d, path) = db_v1();
    let err =
        Db::open_with_space(&path, &[M1, M2], &|_| Err(std::io::Error::other("statvfs"))).err();
    assert!(matches!(err, Some(DbError::Io(_))), "{err:?}");
    let raw = Connection::open(&path).expect("open");
    assert_eq!(user_version(&raw), 1);
}
