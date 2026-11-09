//! `contacts` and `gw_messages`: `RESTRICT`, soft account deletion, watcher reference trigger, uniqueness.

mod common;
mod mig0005_common;

use mig0005_common::*;

#[test]
fn delete_account_restricted_by_gw_messages() {
    let (_d, db) = fresh();
    db.write(|w| {
        let a = account(w, "wa1");
        gw_message(w, a, "m1")?;
        assert!(
            w.execute("DELETE FROM accounts WHERE id = ?1", [a])
                .is_err(),
            "a hard DELETE of an account with gw_messages is rejected (RESTRICT)"
        );
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM accounts"), 1);
    assert_eq!(n(&r, "SELECT count(*) FROM gw_messages"), 1, "rows intact");
}

#[test]
fn account_soft_delete_keeps_gw_messages() {
    let (_d, db) = fresh();
    db.write(|w| {
        let a = account(w, "wa1");
        gw_message(w, a, "m1")?;
        w.execute("INSERT INTO contacts(name) VALUES ('Anna')", [])?;
        w.execute("UPDATE accounts SET deleted_at = 500 WHERE id = ?1", [a])?;
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT deleted_at FROM accounts"), 500);
    assert_eq!(n(&r, "SELECT count(*) FROM gw_messages"), 1);
    assert_eq!(n(&r, "SELECT count(*) FROM contacts"), 1);
}

#[test]
fn finding_ref_nulled_when_gw_message_deleted() {
    let (_d, db) = fresh();
    db.write(|w| {
        let a = account(w, "wa1");
        let m = gw_message(w, a, "m1")?;
        w.execute(
            "INSERT INTO observer_findings(ts, kind, severity, ref_table, ref_id, verdict) VALUES (1, 'injection', 'high', 'gw_messages', CAST(?1 AS TEXT), 'blocked')",
            [m],
        )?;
        w.execute("DELETE FROM gw_messages WHERE id = ?1", [m])?;
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM observer_findings WHERE ref_id IS NULL"
        ),
        1,
        "ref_id cleared by trigger trg_findings_ref_gw"
    );
    assert_eq!(n(&r, "SELECT count(*) FROM observer_findings"), 1);
}

#[test]
fn gw_messages_unique_account_ext() {
    let (_d, db) = fresh();
    db.write(|w| {
        let a = account(w, "wa1");
        let b = account(w, "wa2");
        gw_message(w, a, "m1")?;
        assert!(
            gw_message(w, a, "m1").is_err(),
            "repeat of (account_id, ext_id)"
        );
        assert!(
            gw_message(w, b, "m1").is_ok(),
            "the same ext_id for another account"
        );
        Ok(())
    })
    .expect("write");
}

#[test]
fn gw_message_survives_session_and_contact_delete() {
    let (_d, db) = fresh();
    db.write(|w| {
        let a = account(w, "wa1");
        let s = session(w, "s");
        w.execute("INSERT INTO contacts(name) VALUES ('Anna')", [])?;
        let c = w.last_insert_rowid();
        let m = gw_message(w, a, "m1")?;
        w.execute(
            "UPDATE gw_messages SET routed_session_id = ?1, contact_id = ?2 WHERE id = ?3",
            [s, c, m],
        )?;
        w.execute("DELETE FROM sessions WHERE id = ?1", [s])?;
        w.execute("DELETE FROM contacts WHERE id = ?1", [c])?;
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(
            &r,
            "SELECT count(*) FROM gw_messages WHERE routed_session_id IS NULL AND contact_id IS NULL"
        ),
        1,
        "SET NULL"
    );
}

#[test]
fn delete_persona_nulls_contact_persona() {
    let (_d, db) = fresh();
    db.write(|w| {
        w.execute(
            "INSERT INTO personas(name, color) VALUES ('p', '#aabbcc')",
            [],
        )?;
        let p = w.last_insert_rowid();
        w.execute(
            "INSERT INTO contacts(name, persona_id) VALUES ('Anna', ?1)",
            [p],
        )?;
        w.execute("DELETE FROM personas WHERE id = ?1", [p])?;
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(&r, "SELECT count(*) FROM contacts WHERE persona_id IS NULL"),
        1
    );
}
