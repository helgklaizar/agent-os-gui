//! The `events` table: cascades, indicator transition deduplication, `sub` as core data, checks.

mod common;
mod mig0005_common;

use fake_agentd::events_db;
use mig0005_common::*;

#[test]
fn delete_session_keeps_system_events() {
    let (_d, db) = fresh();
    let (sid, system) = db
        .write(|w| {
            let sid = session(w, "s");
            event(w, "ai", "agent", Some(sid), None)?;
            event(w, "msg", "sys", None, None)?;
            let rows = w.execute("DELETE FROM sessions WHERE id = ?1", [sid])?;
            assert_eq!(rows, 1);
            Ok((sid, w.last_insert_rowid()))
        })
        .expect("deletion");
    let _ = system;
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM events"), 2, "events stayed");
    assert_eq!(
        n(&r, "SELECT count(*) FROM events WHERE session_id IS NULL"),
        2,
        "session_id cleared"
    );
    assert_eq!(
        n(
            &r,
            &format!("SELECT count(*) FROM events WHERE session_id = {sid}")
        ),
        0
    );
}

#[test]
fn dedup_key_unique_even_after_read() {
    let (_d, db) = fresh();
    db.write(|w| {
        assert_eq!(event(w, "msg", "sys", None, Some("cpu:1"))?, 1);
        w.execute("UPDATE events SET unread = 0", [])?;
        assert_eq!(
            event(w, "msg", "sys", None, Some("cpu:1"))?,
            0,
            "a repeated dedup_key is rejected even after reading"
        );
        assert!(
            w.execute(
                "INSERT INTO events(kind, sub, text, dedup_key, ts) VALUES ('msg','sys','y','cpu:1',1)",
                []
            )
            .is_err(),
            "a plain insert with the same key violates uniqueness"
        );
        event(w, "msg", "sys", None, None)?;
        event(w, "msg", "sys", None, None)?;
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(
        n(&r, "SELECT count(*) FROM events"),
        3,
        "NULL keys do not conflict"
    );
}

#[test]
fn transition_red_read_red_one_event() {
    let (_d, db) = fresh();
    db.write(|w| {
        indicator(w, "cpu", "ok");
        assert!(
            events_db::transition(w, "cpu", "bad", 10)?.is_some(),
            "ok->bad: event 1"
        );
        w.execute("UPDATE events SET unread = 0", [])?;
        assert!(
            events_db::transition(w, "cpu", "bad", 11)?.is_none(),
            "a repeated bad frame: no events"
        );
        assert!(
            events_db::transition(w, "cpu", "warn", 12)?.is_some(),
            "bad->warn: event 2"
        );
        assert!(
            events_db::transition(w, "cpu", "bad", 13)?.is_some(),
            "warn->bad: event 3"
        );
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM events"), 3);
    let keys: Vec<String> = r
        .prepare("SELECT dedup_key FROM events ORDER BY id")
        .expect("query")
        .query_map([], |x| x.get(0))
        .expect("rows")
        .flatten()
        .collect();
    assert_eq!(keys, vec!["cpu:1", "cpu:2", "cpu:3"]);
    assert_eq!(
        n(&r, "SELECT transition_seq FROM indicators WHERE id = 'cpu'"),
        3
    );
}

#[test]
fn transition_nodata_is_a_transition() {
    let (_d, db) = fresh();
    db.write(|w| {
        indicator(w, "whapi", "ok");
        assert!(events_db::transition(w, "whapi", "nodata", 10)?.is_some());
        assert!(events_db::transition(w, "whapi", "ok", 11)?.is_some());
        Ok(())
    })
    .expect("write");
    let r = db.reader().expect("reader");
    assert_eq!(n(&r, "SELECT count(*) FROM events"), 2);
    assert_eq!(
        n(&r, "SELECT count(DISTINCT dedup_key) FROM events"),
        2,
        "different transition_seq"
    );
}

#[test]
fn transition_unknown_indicator_is_error() {
    let (_d, db) = fresh();
    let res = db.write(|w| Ok(events_db::transition(w, "no-such", "bad", 1)));
    assert!(res.expect("write").is_err(), "a nonexistent indicator");
}

#[test]
fn events_sub_is_free_text() {
    let (_d, db) = fresh();
    db.write(|w| {
        assert_eq!(
            event(w, "ai", "new-subtype", None, None)?,
            1,
            "sub is not an enum"
        );
        assert!(
            w.execute(
                "INSERT INTO events(kind, sub, text, ts) VALUES ('bogus','agent','x',1)",
                []
            )
            .is_err(),
            "kind outside ai/task/msg (INSERT OR IGNORE would silently skip a CHECK violation)"
        );
        Ok(())
    })
    .expect("write");
}

#[test]
fn events_check_constraints() {
    let (_d, db) = fresh();
    db.write(|w| {
        assert!(
            w.execute("INSERT INTO events(kind, sub, level, text, ts) VALUES ('ai','agent','fatal','x',1)", [])
                .is_err(),
            "level outside info/warn/error/ok"
        );
        assert!(
            w.execute("INSERT INTO events(kind, sub, text, ts) VALUES ('nope','agent','x',1)", [])
                .is_err(),
            "kind outside ai/task/msg"
        );
        w.execute("INSERT INTO events(kind, sub, text, ts) VALUES ('ai','agent','x',1)", [])?;
        let (level, unread): (String, i64) = w.query_row(
            "SELECT level, unread FROM events",
            [],
            |x| Ok((x.get(0)?, x.get(1)?)),
        )?;
        assert_eq!((level.as_str(), unread), ("info", 1), "defaults");
        Ok(())
    })
    .expect("write");
}
