//! Migration 0003: integrity triggers: cycles, folder and thread depth, id reuse, persona mode checks.

mod common;
mod mig0003_common;

use mig0003_common::*;
use rusqlite::params;

#[test]
fn folder_cycle_rejected() {
    let (_d, db) = fresh();
    let (a, b) = db
        .write(|w| {
            let a = folder(w, None, "A", None);
            let b = folder(w, Some(a), "B", None);
            Ok((a, b))
        })
        .expect("fixture");
    let err = db
        .write(|w| {
            w.execute(
                "UPDATE folders SET parent_id = ?1 WHERE id = ?2",
                params![b, a],
            )
        })
        .expect_err("cycle accepted");
    assert!(err.to_string().contains("folder cycle"), "{err}");
}

#[test]
fn folder_self_parent_rejected() {
    let (_d, db) = fresh();
    let a = db
        .write(|w| Ok(folder(w, None, "A", None)))
        .expect("folder");
    let err = db
        .write(|w| {
            w.execute(
                "UPDATE folders SET parent_id = ?1 WHERE id = ?1",
                params![a],
            )
        })
        .expect_err("parent_id = id accepted");
    assert!(
        !err.to_string().is_empty(),
        "rejected (CHECK or cycle trigger)"
    );
    let ins = db.write(|w| {
        w.execute(
            "INSERT INTO folders(id, parent_id, name, created_at) VALUES (50, 50, 'x', 1)",
            [],
        )
    });
    assert!(
        ins.is_err(),
        "inserting parent_id = id is rejected by CHECK"
    );
}

#[test]
fn folder_depth_limit_8() {
    let (_d, db) = fresh();
    let mut parent = None;
    for level in 1..=8 {
        let id = db
            .write(|w| {
                w.execute(
                    "INSERT INTO folders(parent_id, name, created_at) VALUES (?1, ?2, 1)",
                    params![parent, format!("L{level}")],
                )?;
                Ok(w.last_insert_rowid())
            })
            .unwrap_or_else(|e| panic!("level {level} must be accepted: {e}"));
        parent = Some(id);
    }
    let err = db
        .write(|w| {
            w.execute(
                "INSERT INTO folders(parent_id, name, created_at) VALUES (?1, 'L9', 1)",
                params![parent],
            )
        })
        .expect_err("9th level accepted");
    assert!(err.to_string().contains("folder depth"), "{err}");
}

#[test]
fn folder_depth_limit_on_update() {
    let (_d, db) = fresh();
    let (deep, loose) = db
        .write(|w| {
            let mut parent = None;
            for i in 0..8 {
                parent = Some(folder(w, parent, &format!("L{i}"), None));
            }
            Ok((parent.unwrap_or(0), folder(w, None, "free", None)))
        })
        .expect("fixture");
    let err = db
        .write(|w| {
            w.execute(
                "UPDATE folders SET parent_id = ?1 WHERE id = ?2",
                params![deep, loose],
            )
        })
        .expect_err("move to the 9th level accepted");
    assert!(err.to_string().contains("folder depth"), "{err}");
}

#[test]
fn thread_depth_one_level() {
    let (_d, db) = fresh();
    let (s, t) = db
        .write(|w| {
            let s = session(w, None, None, "session");
            Ok((s, session(w, None, Some(s), "thread")))
        })
        .expect("fixture");
    let err = db
        .write(|w| {
            w.execute(
                "INSERT INTO sessions(parent_id, name, created_at, updated_at) VALUES (?1, 'thread of thread', 1, 1)",
                params![t],
            )
        })
        .expect_err("thread of a thread accepted");
    assert!(err.to_string().contains("thread depth"), "{err}");
    let upd = db
        .write(|w| {
            let other = session(w, None, None, "other");
            w.execute(
                "UPDATE sessions SET parent_id = ?1 WHERE id = ?2",
                params![t, other],
            )
        })
        .expect_err("move under a thread accepted");
    assert!(upd.to_string().contains("thread depth"), "{upd}");
    assert!(
        db.write(|w| w.execute(
            "INSERT INTO sessions(parent_id, name, created_at, updated_at) VALUES (?1, 'another thread', 1, 1)",
            params![s]
        ))
        .is_ok(),
        "a second thread of a session is allowed"
    );
}

#[test]
fn ids_not_reused_after_delete() {
    let (_d, db) = fresh();
    let old = db.write(|w| Ok(session(w, None, None, "last"))).expect("s");
    db.write(|w| w.execute("DELETE FROM sessions WHERE id = ?1", params![old]))
        .expect("deletion");
    let new = db.write(|w| Ok(session(w, None, None, "new"))).expect("s2");
    assert!(new > old, "id {new} is not greater than the deleted {old}");
}

#[test]
fn session_persona_check_modes() {
    let (_d, db) = fresh();
    for good in ["own", "shared", "none"] {
        let ok = db.write(|w| {
            w.execute(
                "INSERT INTO personas(name, color, mem_mode) VALUES ('p', '#000', ?1)",
                params![good],
            )
        });
        assert!(ok.is_ok(), "{good} must be accepted");
    }
    assert!(
        db.write(|w| w.execute(
            "INSERT INTO personas(name, color, mem_mode) VALUES ('p', '#000', 'all')",
            []
        ))
        .is_err(),
        "mem_mode outside own/shared/none"
    );
    for good in ["all", "important", "digest", "off"] {
        let ok = db.write(|w| {
            w.execute(
                "INSERT INTO personas(name, color, notif_mode) VALUES ('p', '#000', ?1)",
                params![good],
            )
        });
        assert!(ok.is_ok(), "{good} must be accepted");
    }
    assert!(
        db.write(|w| w.execute(
            "INSERT INTO personas(name, color, notif_mode) VALUES ('p', '#000', 'loud')",
            []
        ))
        .is_err(),
        "notif_mode outside all/important/digest/off"
    );
}
