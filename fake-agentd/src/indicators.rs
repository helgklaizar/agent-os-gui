//! Bar indicator state in the core DB (db.md sections 3.3 and 3.12).
//!
//! The core stores `indicators.last_state` and `indicators.transition_seq`. A state change in one transaction increments the counter
//! and inserts an event with `dedup_key = <indicator>:<transition_seq>` (`INSERT OR IGNORE`). A frame in the same state writes
//! nothing, so the read status of the previous event does not affect deduplication. Metric values do not reach the DB.

use std::sync::Arc;

use rusqlite::params;

use crate::db::Db;
use crate::store::{StoreError, db_err, map_db};

/// Indicator state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndState {
    Ok,
    Warn,
    Bad,
    NoData,
    Unknown,
}

impl IndState {
    pub fn as_str(self) -> &'static str {
        match self {
            IndState::Ok => "ok",
            IndState::Warn => "warn",
            IndState::Bad => "bad",
            IndState::NoData => "nodata",
            IndState::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<IndState> {
        [
            IndState::Ok,
            IndState::Warn,
            IndState::Bad,
            IndState::NoData,
            IndState::Unknown,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }

    /// `events.level` for the transition event.
    fn level(self) -> &'static str {
        match self {
            IndState::Bad => "error",
            IndState::Warn => "warn",
            IndState::Ok => "ok",
            IndState::NoData | IndState::Unknown => "info",
        }
    }

    fn label(self) -> &'static str {
        match self {
            IndState::Ok => "normal",
            IndState::Warn => "warning",
            IndState::Bad => "failure",
            IndState::NoData => "no data",
            IndState::Unknown => "unknown",
        }
    }
}

/// Result of applying a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Transition {
    /// The state changed (the counter grew).
    pub changed: bool,
    /// Current `transition_seq`.
    pub seq: i64,
    /// The created event; `None` - there was no transition or this is the initial setup.
    pub event_id: Option<i64>,
}

/// Row of the `indicators` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndicatorRow {
    pub id: String,
    pub enabled: bool,
    pub position: i64,
    pub state: IndState,
    pub seq: i64,
}

pub struct IndicatorStore {
    db: Arc<Db>,
}

impl IndicatorStore {
    pub fn new(db: Arc<Db>) -> IndicatorStore {
        IndicatorStore { db }
    }

    /// Applies a frame. The first frame after `unknown` into `ok` or `nodata` counts as initialization: the counter grows, no event
    /// (otherwise at startup every indicator would clutter the feed with a "normal" entry).
    pub fn apply(&self, id: &str, state: IndState, now: i64) -> Result<Transition, StoreError> {
        self.db
            .write(|c| {
                let tx = c.transaction()?;
                tx.execute("INSERT OR IGNORE INTO indicators(id) VALUES (?1)", [id])?;
                let (last, seq): (String, i64) = tx.query_row(
                    "SELECT last_state, transition_seq FROM indicators WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                let last = IndState::parse(&last).unwrap_or(IndState::Unknown);
                if last == state {
                    return Ok(Transition {
                        changed: false,
                        seq,
                        event_id: None,
                    });
                }
                let seq = seq + 1;
                tx.execute(
                    "UPDATE indicators SET last_state = ?1, transition_seq = ?2, last_change_at = ?3 WHERE id = ?4",
                    params![state.as_str(), seq, now, id],
                )?;
                let silent = last == IndState::Unknown
                    && matches!(state, IndState::Ok | IndState::NoData);
                let mut event_id = None;
                if !silent {
                    let n = tx.execute(
                        "INSERT OR IGNORE INTO events(kind, sub, level, text, source, dedup_key, ts)
                         VALUES ('msg', 'sys', ?1, ?2, 'agentd', ?3, ?4)",
                        params![
                            state.level(),
                            format!("Indicator {id}: {}", state.label()),
                            format!("{id}:{seq}"),
                            now
                        ],
                    )?;
                    if n == 1 {
                        event_id = Some(tx.last_insert_rowid());
                    }
                }
                tx.commit()?;
                Ok(Transition {
                    changed: true,
                    seq,
                    event_id,
                })
            })
            .map_err(map_db)
    }

    /// All indicators in `position` order, then `id`.
    pub fn rows(&self) -> Result<Vec<IndicatorRow>, StoreError> {
        let conn = self.db.reader().map_err(db_err)?;
        let mut stmt = conn
            .prepare(
                "SELECT id, enabled, position, last_state, transition_seq FROM indicators ORDER BY position, id",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(IndicatorRow {
                    id: r.get(0)?,
                    enabled: r.get::<_, i64>(1)? != 0,
                    position: r.get(2)?,
                    state: IndState::parse(&r.get::<_, String>(3)?).unwrap_or(IndState::Unknown),
                    seq: r.get(4)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<Result<_, _>>().map_err(db_err)
    }

    /// Enable flag and order (the settings page is handled elsewhere; here it is a write to `indicators`).
    pub fn set_layout(&self, id: &str, enabled: bool, position: i64) -> Result<(), StoreError> {
        self.db
            .write(|c| {
                c.execute("INSERT OR IGNORE INTO indicators(id) VALUES (?1)", [id])?;
                c.execute(
                    "UPDATE indicators SET enabled = ?1, position = ?2 WHERE id = ?3",
                    params![i64::from(enabled), position, id],
                )?;
                Ok(())
            })
            .map_err(map_db)
    }
}
