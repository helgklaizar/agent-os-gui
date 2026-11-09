//! Migration 0003: cascades, `SET NULL`, effective persona, foreign key cleanliness on deletion.

mod common;
mod mig0003_common;

use fake_agentd::tree;
use mig0003_common::*;
use rusqlite::params;

#[test]
fn thread_cascade_on_session_delete() {
    let (_d, db) = fresh();
    let (keep, gone) = db
        .write(|w| {
            let keep = session(w, None, None, "Neighbor");
            let gone = session(w, None, None, "Doomed");
            for i in 0..3 {
                session(w, None, Some(gone), &format!("thread {i}"));
            }
            session(w, None, Some(keep), "neighbor thread");
            Ok((keep, gone))
        })
        .expect("fixture");
    db.write(|w| w.execute("DELETE FROM sessions WHERE id = ?1", params![gone]))
        .expect("deletion");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(
            &r,
            &format!("SELECT count(*) FROM sessions WHERE parent_id = {gone}")
        ),
        0,
        "threads deleted"
    );
    assert_eq!(
        n(&r, "SELECT count(*) FROM sessions"),
        2,
        "the rest in place"
    );
    assert_eq!(
        n(
            &r,
            &format!("SELECT count(*) FROM sessions WHERE id = {keep}")
        ),
        1
    );
}

#[test]
fn folder_delete_fk_set_null() {
    let (_d, db) = fresh();
    let f = db
        .write(|w| {
            let f = folder(w, None, "Folder", None);
            session(w, Some(f), None, "a");
            session(w, Some(f), None, "b");
            Ok(f)
        })
        .expect("fixture");
    db.write(|w| w.execute("DELETE FROM folders WHERE id = ?1", params![f]))
        .expect("DELETE");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM sessions"), 2, "rows alive");
    assert_eq!(
        n(&r, "SELECT count(*) FROM sessions WHERE folder_id IS NULL"),
        2,
        "folder_id cleared"
    );
}

#[test]
fn persona_delete_nulls_refs() {
    let (_d, db) = fresh();
    let p = db
        .write(|w| {
            let a = account(w, "cl1");
            let p = persona(w, "Work");
            w.execute(
                "INSERT INTO persona_accounts(persona_id, account_id) VALUES (?1, ?2)",
                params![p, a],
            )?;
            let f = folder(w, None, "Folder", Some(p));
            let s = session(w, Some(f), None, "s");
            w.execute(
                "UPDATE sessions SET persona_id = ?1 WHERE id = ?2",
                params![p, s],
            )?;
            Ok(p)
        })
        .expect("fixture");
    db.write(|w| w.execute("DELETE FROM personas WHERE id = ?1", params![p]))
        .expect("DELETE");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM folders WHERE persona_id IS NOT NULL"
        ),
        0
    );
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM sessions WHERE persona_id IS NOT NULL"
        ),
        0
    );
    assert_eq!(n(&r, "SELECT count(*) FROM folders"), 1, "folder alive");
    assert_eq!(n(&r, "SELECT count(*) FROM persona_accounts"), 0);
}

#[test]
fn account_delete_cleans_persona_accounts() {
    let (_d, db) = fresh();
    let a = db
        .write(|w| {
            let a = account(w, "cl1");
            let p = persona(w, "Work");
            w.execute(
                "UPDATE personas SET model_acc = ?1 WHERE id = ?2",
                params![a, p],
            )?;
            w.execute(
                "INSERT INTO persona_accounts(persona_id, account_id) VALUES (?1, ?2)",
                params![p, a],
            )?;
            Ok(a)
        })
        .expect("fixture");
    db.write(|w| w.execute("DELETE FROM accounts WHERE id = ?1", params![a]))
        .expect("DELETE");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM persona_accounts"), 0);
    assert_eq!(
        n(&r, "SELECT count(*) FROM personas WHERE model_acc IS NULL"),
        1
    );
}

#[test]
fn effective_persona_query() {
    let (_d, db) = fresh();
    let (s, own, p) = db
        .write(|w| {
            let p = persona(w, "Work");
            let top = folder(w, None, "Top", Some(p));
            let mid = folder(w, Some(top), "Middle", None);
            let s = session(w, Some(mid), None, "no persona");
            let own = persona(w, "Personal");
            let s2 = session(w, Some(mid), None, "own persona");
            w.execute(
                "UPDATE sessions SET persona_id = ?1 WHERE id = ?2",
                params![own, s2],
            )?;
            Ok((s, s2, p))
        })
        .expect("fixture");
    let r = db.reader().expect("reader");
    assert_eq!(tree::effective_persona_id(&r, s).ok().flatten(), Some(p));
    assert!(
        tree::effective_persona_id(&r, own)
            .ok()
            .flatten()
            .is_some_and(|x| x != p),
        "own persona beats the folder"
    );
    let stored: Option<i64> = r
        .query_row(
            "SELECT persona_id FROM sessions WHERE id = ?1",
            params![s],
            |x| x.get(0),
        )
        .unwrap_or(Some(-1));
    assert_eq!(stored, None, "persona_id stays NULL in the table");
}

#[test]
fn effective_persona_none_without_any() {
    let (_d, db) = fresh();
    let s = db
        .write(|w| Ok(session(w, None, None, "bare")))
        .expect("fixture");
    let r = db.reader().expect("reader");
    assert_eq!(tree::effective_persona_id(&r, s).ok().flatten(), None);
}

#[test]
fn session_delete_foreign_keys_clean() {
    let (_d, db) = fresh();
    let s = db
        .write(|w| {
            let f = folder(w, None, "Folder", None);
            let other = session(w, Some(f), None, "neighbor");
            session(w, Some(f), Some(other), "neighbor thread");
            let s = session(w, Some(f), None, "doomed");
            for i in 0..3 {
                session(w, Some(f), Some(s), &format!("thread {i}"));
            }
            Ok(s)
        })
        .expect("fixture");
    db.write(|w| w.execute("DELETE FROM sessions WHERE id = ?1", params![s]))
        .expect("deletion");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM pragma_foreign_key_check"), 0);
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM sessions t WHERE t.parent_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sessions p WHERE p.id = t.parent_id)"
        ),
        0,
        "no orphaned threads"
    );
    assert_eq!(n(&r, "SELECT count(*) FROM sessions"), 2);
    assert_eq!(n(&r, "SELECT count(*) FROM folders"), 1);
}
