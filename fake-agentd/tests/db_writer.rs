//! Single writer, read pool, connection PRAGMAs (db.md section 2), file permissions.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use common::*;
use fake_agentd::{Db, DbError};

fn mode(p: &std::path::Path) -> u32 {
    std::fs::metadata(p)
        .map(|m| m.permissions().mode() & 0o777)
        .unwrap_or(0)
}

#[test]
fn db_connection_pragmas() {
    let (_d, path) = temp_db_path();
    let db = open(&path);
    let (jm, fk, bt, sy) = db
        .write(|w| {
            Ok((
                pragma_text(w, "journal_mode"),
                pragma_text(w, "foreign_keys"),
                pragma_text(w, "busy_timeout"),
                pragma_text(w, "synchronous"),
            ))
        })
        .expect("pragma");
    assert_eq!(
        (jm.as_str(), fk.as_str(), bt.as_str(), sy.as_str()),
        ("wal", "1", "5000", "1"),
        "WAL, foreign_keys ON, busy_timeout 5000, synchronous NORMAL"
    );
    let r = db.reader().unwrap();
    assert_eq!(pragma_text(&r, "foreign_keys"), "1");
    assert_eq!(pragma_text(&r, "busy_timeout"), "5000");
    assert_eq!(pragma_text(&r, "query_only"), "1");
}

#[test]
fn db_single_writer_second_open_rejected() {
    let (_d, path) = temp_db_path();
    let first = open(&path);
    let second = Db::open(&path).err();
    assert!(
        matches!(second, Some(DbError::WriterBusy)),
        "the second writer is rejected: {second:?}"
    );
    drop(first);
    assert!(
        Db::open(&path).is_ok(),
        "after the writer closes, the DB opens again"
    );
}

#[test]
fn db_reader_cannot_write() {
    let (_d, path) = temp_db_path();
    let db = open(&path);
    db.write(|w| {
        w.execute(
            "INSERT INTO settings(key, value_json, updated_at) VALUES ('a', '1', 1)",
            [],
        )
    })
    .expect("the writer writes");
    let r = db.reader().unwrap();
    let seen: i64 = r
        .query_row("SELECT count(*) FROM settings", [], |x| x.get(0))
        .unwrap_or(-1);
    assert_eq!(seen, 1, "the reader sees the writer's commit");
    let err = r.execute(
        "INSERT INTO settings(key, value_json, updated_at) VALUES ('b', '2', 1)",
        [],
    );
    assert!(err.is_err(), "the reader does not write");
    let err = r.execute("CREATE TABLE evil (x)", []);
    assert!(err.is_err(), "the reader does not change the schema");
    r.execute("PRAGMA query_only = OFF", []).ok();
    assert!(
        r.execute(
            "INSERT INTO settings(key, value_json, updated_at) VALUES ('c', '3', 1)",
            []
        )
        .is_err(),
        "and after clearing query_only: the connection is open READ_ONLY"
    );
}

#[test]
fn db_writes_are_serialised_across_threads() {
    let (_d, path) = temp_db_path();
    let db = Arc::new(open(&path));
    db.write(|w| {
        w.execute(
            "INSERT INTO settings(key, value_json, updated_at) VALUES ('n', '0', 1)",
            [],
        )
    })
    .unwrap();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let db = Arc::clone(&db);
            std::thread::spawn(move || {
                for _ in 0..50 {
                    db.write(|w| {
                        let tx = w.transaction()?;
                        let v: i64 = tx.query_row(
                            "SELECT CAST(value_json AS INTEGER) FROM settings WHERE key = 'n'",
                            [],
                            |x| x.get(0),
                        )?;
                        tx.execute(
                            "UPDATE settings SET value_json = ?1, rev = rev + 1 WHERE key = 'n'",
                            [(v + 1).to_string()],
                        )?;
                        tx.commit()
                    })
                    .expect("write");
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread");
    }
    let n: i64 = db
        .reader()
        .unwrap()
        .query_row(
            "SELECT CAST(value_json AS INTEGER) FROM settings WHERE key = 'n'",
            [],
            |x| x.get(0),
        )
        .unwrap_or(-1);
    assert_eq!(n, 400);
}

#[test]
fn db_files_permissions_0700_dir_0600_files() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join(".agentos");
    let path = home.join("sessions.db");
    let db = Db::open(&path).expect("directory is created");
    db.write(|w| {
        w.execute(
            "INSERT INTO settings(key, value_json, updated_at) VALUES ('k', '1', 1)",
            [],
        )
    })
    .unwrap();
    assert_eq!(mode(&home), 0o700, "directory ~/.agentos is 0700");
    for name in ["sessions.db", "sessions.db-wal", "sessions.db-shm"] {
        let p = home.join(name);
        assert!(p.exists(), "{name} exists while the connection is open");
        assert_eq!(mode(&p), 0o600, "{name} — 0600");
    }
    assert!(
        !db.storage_report().perms_tightened,
        "a new DB did not get its permissions \"tightened\""
    );
}

#[test]
fn db_open_tightens_loose_permissions() {
    let (d, path) = temp_db_path();
    drop(open(&path));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let db = open(&path);
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(d.path()), 0o700);
    assert!(db.storage_report().perms_tightened);
}
