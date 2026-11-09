//! Event-feed fixtures for the A5-03 end-to-end tests: events written to the DB before the core starts.
#![allow(dead_code)]

use serde_json::Value;

use super::Env;

/// An event written directly to the DB before the core starts: `(kind, sub, level, session_id, unread, ts)`.
pub type Fx = (
    &'static str,
    &'static str,
    &'static str,
    Option<i64>,
    i64,
    i64,
);

pub fn seed(
    rows: Vec<Fx>,
) -> impl FnOnce(&agentd_db::Connection) -> agentd_db::SqlResult<()> + Send {
    move |c| {
        c.execute(
            "INSERT INTO sessions(id, name, created_at, updated_at) VALUES (1,'a',1,1),(2,'b',1,1)",
            [],
        )?;
        for (kind, sub, level, sid, unread, ts) in rows {
            c.execute(
                "INSERT INTO events(kind, sub, level, text, session_id, unread, ts) VALUES (?1,?2,?3,'t',?4,?5,?6)",
                agentd_db::params![kind, sub, level, sid, unread, ts],
            )?;
        }
        Ok(())
    }
}

pub fn mixed() -> Vec<Fx> {
    vec![
        ("ai", "agent", "info", Some(1), 1, 10),
        ("ai", "watch", "warn", None, 1, 11),
        ("task", "sched", "ok", Some(2), 0, 12),
        ("msg", "tg", "info", None, 1, 13),
        ("msg", "wa", "error", Some(1), 1, 14),
    ]
}

pub async fn list(e: &Env, q: &str) -> Value {
    let (st, v) = e.read.get(&format!("/events{q}")).await;
    assert_eq!(st, 200, "{v}");
    v
}

pub fn ids(v: &Value) -> Vec<i64> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_i64().unwrap())
        .collect()
}
