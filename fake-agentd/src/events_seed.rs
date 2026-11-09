//! Event fixtures: one unread event of each of the 11 subtypes and two read ones, some with sessions from the tree fixtures, some system.
//! Created at mock start; leaves existing data alone.

use rusqlite::{Connection, OptionalExtension, params};

use crate::events_db::{self, NewEvent};

struct Fixture {
    kind: &'static str,
    sub: &'static str,
    level: &'static str,
    text: &'static str,
    session: Option<&'static str>,
    source: &'static str,
    unread: bool,
}

#[rustfmt::skip]
const FIXTURES: [Fixture; 13] = [
    f("ai", "agent", "ok", "Reply ready: release 1.2 build finished", Some("Release 1.2"), "agentd", true),
    f("ai", "watch", "warn", "Watcher blocked a dangerous command", Some("Review"), "agent-watch", true),
    f("ai", "claude", "warn", "Claude: window limit almost exhausted", None, "agentd", true),
    f("ai", "local", "ok", "Local model loaded", None, "agentd", true),
    f("task", "sched", "info", "Scheduled run: nightly backup", None, "agent-sched", true),
    f("task", "deleg", "info", "Delegated: client email reconciliation", Some("Client emails"), "agentd", true),
    f("task", "done", "ok", "Done: weekly report", Some("Release 1.2"), "agent-sched", true),
    f("msg", "sys", "warn", "Battery 15%", None, "agent-sys", true),
    f("msg", "wa", "info", "Anna: when is the release?", Some("Client emails"), "agent-gw", true),
    f("msg", "tg", "info", "Boris: call on Friday", None, "agent-gw", true),
    f("msg", "mail", "info", "Supplier invoice for payment", None, "agent-gw", true),
    f("ai", "agent", "info", "Reply ready: log analysis", Some("Release 1.2"), "agentd", false),
    f("msg", "wa", "info", "Anna: thanks", Some("Client emails"), "agent-gw", false),
];

const fn f(
    kind: &'static str,
    sub: &'static str,
    level: &'static str,
    text: &'static str,
    session: Option<&'static str>,
    source: &'static str,
    unread: bool,
) -> Fixture {
    Fixture {
        kind,
        sub,
        level,
        text,
        session,
        source,
        unread,
    }
}

fn session_id(conn: &Connection, name: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM sessions WHERE name = ?1",
        params![name],
        |r| r.get(0),
    )
    .optional()
}

/// Older events get an earlier `ts`: the feed goes from newest to oldest.
pub fn seed_rows(w: &mut Connection, now: i64) -> rusqlite::Result<()> {
    let tx = w.transaction()?;
    let empty: i64 = tx.query_row("SELECT count(*) FROM events", [], |r| r.get(0))?;
    if empty > 0 {
        return tx.commit();
    }
    for (i, fx) in FIXTURES.iter().enumerate() {
        let session = match fx.session {
            Some(name) => session_id(&tx, name)?,
            None => None,
        };
        let id = events_db::insert(
            &tx,
            &NewEvent {
                kind: fx.kind,
                sub: fx.sub,
                level: Some(fx.level),
                text: fx.text,
                session_id: session,
                source: Some(fx.source),
                dedup_key: None,
                ts: now - 60 * (FIXTURES.len() - i) as i64,
            },
        )?;
        if let (Some(id), false) = (id, fx.unread) {
            tx.execute("UPDATE events SET unread = 0 WHERE id = ?1", params![id])?;
        }
    }
    tx.commit()
}

/// Fixtures at mock start (after the tree fixtures: events refer to its sessions).
pub fn start(shared: &crate::state::Shared, db: &crate::db::Db) -> Result<(), crate::db::DbError> {
    let now = shared.clock.now_unix();
    db.write(|w| seed_rows(w, now))
}
