//! One event per indicator state transition; state and layout survive a restart.

mod common;

use common::*;
use fake_agentd::Db;
use fake_agentd::indicators::{IndState, IndicatorStore};
use std::sync::Arc;

fn store() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Arc<Db>,
    IndicatorStore,
) {
    let (d, path) = temp_db_path();
    let db = open(&path);
    let db = Arc::new(db);
    let st = IndicatorStore::new(db.clone());
    (d, path, db, st)
}

fn events(db: &Db, id: &str) -> Vec<(String, String, i64)> {
    let Ok(r) = db.reader() else {
        return Vec::new();
    };
    let Ok(mut stmt) = r
        .prepare("SELECT dedup_key, level, unread FROM events WHERE dedup_key LIKE ?1 ORDER BY id")
    else {
        return Vec::new();
    };
    stmt.query_map([format!("{id}:%")], |x| {
        Ok((x.get(0)?, x.get(1)?, x.get(2)?))
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

fn mark_all_read(db: &Db) {
    db.write(|w| w.execute("UPDATE events SET unread = 0", []))
        .expect("read");
}

#[test]
fn hundred_red_frames_make_one_event_even_after_read() {
    let (_d, _p, db, st) = store();
    let first = st.apply("disk", IndState::Bad, 100).expect("frame");
    assert!(first.changed);
    for i in 0..99 {
        if i == 40 {
            mark_all_read(&db);
        }
        let t = st.apply("disk", IndState::Bad, 101 + i).expect("frame");
        assert!(!t.changed, "same state - not a transition");
        assert_eq!(t.event_id, None);
    }
    let ev = events(&db, "disk");
    assert_eq!(ev.len(), 1, "100 red frames - one event: {ev:?}");
    assert_eq!(
        ev[0].0, "disk:1",
        "dedup_key = <indicator>:<transition_seq>"
    );
    assert_eq!(ev[0].1, "error");
}

#[test]
fn return_to_ok_and_second_red_each_make_one_event() {
    let (_d, _p, db, st) = store();
    st.apply("disk", IndState::Bad, 1).expect("red");
    st.apply("disk", IndState::Ok, 2).expect("return");
    st.apply("disk", IndState::Ok, 3).expect("ok again");
    st.apply("disk", IndState::Bad, 4).expect("repeated red");
    let keys: Vec<String> = events(&db, "disk").into_iter().map(|e| e.0).collect();
    assert_eq!(keys, vec!["disk:1", "disk:2", "disk:3"]);
    let levels: Vec<String> = events(&db, "disk").into_iter().map(|e| e.1).collect();
    assert_eq!(levels, vec!["error", "ok", "error"]);
}

#[test]
fn red_to_yellow_is_a_transition() {
    let (_d, _p, db, st) = store();
    st.apply("ram", IndState::Bad, 1).expect("red");
    let t = st.apply("ram", IndState::Warn, 2).expect("yellow");
    assert!(t.changed && t.event_id.is_some(), "red -> yellow: an event");
    let levels: Vec<String> = events(&db, "ram").into_iter().map(|e| e.1).collect();
    assert_eq!(levels, vec!["error", "warn"]);
}

#[test]
fn red_read_red_gives_one_event_per_red() {
    let (_d, _p, db, st) = store();
    st.apply("temp", IndState::Bad, 1).expect("red");
    mark_all_read(&db);
    st.apply("temp", IndState::Bad, 2)
        .expect("red after reading");
    assert_eq!(
        events(&db, "temp").len(),
        1,
        "regression of deduplication by unread"
    );
}

#[test]
fn initial_unknown_to_ok_is_silent_but_counts() {
    let (_d, _p, db, st) = store();
    let t = st.apply("net", IndState::Ok, 1).expect("first frame");
    assert!(t.changed);
    assert_eq!(t.seq, 1);
    assert_eq!(t.event_id, None, "initialization without an event");
    assert!(events(&db, "net").is_empty());
}

#[test]
fn transition_seq_grows_exactly_per_transition() {
    let (_d, _p, _db, st) = store();
    let seqs: Vec<i64> = [
        IndState::Bad,
        IndState::Bad,
        IndState::Warn,
        IndState::Warn,
        IndState::Ok,
        IndState::Bad,
    ]
    .into_iter()
    .map(|s| st.apply("cpu", s, 1).map(|t| t.seq).unwrap_or(-1))
    .collect();
    assert_eq!(seqs, vec![1, 1, 2, 2, 3, 4]);
}

#[test]
fn state_layout_and_events_survive_restart() {
    let (_d, path, db, st) = store();
    st.apply("cpu", IndState::Bad, 5).expect("red");
    st.apply("ram", IndState::Warn, 6).expect("yellow");
    st.set_layout("ram", true, 1).expect("layout");
    st.set_layout("cpu", false, 7).expect("layout");
    drop(st);
    drop(db);

    let db = Arc::new(open(&path));
    let st = IndicatorStore::new(db.clone());
    let rows = st.rows().expect("rows");
    let by = |id: &str| rows.iter().find(|r| r.id == id).cloned();
    let cpu = by("cpu").expect("cpu");
    let ram = by("ram").expect("ram");
    assert_eq!(
        (cpu.enabled, cpu.position, cpu.state, cpu.seq),
        (false, 7, IndState::Bad, 1)
    );
    assert_eq!(
        (ram.enabled, ram.position, ram.state, ram.seq),
        (true, 1, IndState::Warn, 1)
    );
    let t = st
        .apply("cpu", IndState::Bad, 9)
        .expect("same red after restart");
    assert!(!t.changed, "the core remembers the state: no new event");
    assert_eq!(events(&db, "cpu").len(), 1);
}

#[test]
fn state_strings_roundtrip() {
    for s in [
        IndState::Ok,
        IndState::Warn,
        IndState::Bad,
        IndState::NoData,
        IndState::Unknown,
    ] {
        assert_eq!(IndState::parse(s.as_str()), Some(s));
    }
    assert_eq!(IndState::parse("red"), None, "a foreign value is rejected");
    assert_eq!(IndState::Warn.as_str(), "warn");
}
