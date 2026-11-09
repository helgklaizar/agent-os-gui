//! Indicator transitions (`IndicatorStore`, A5-04). The state, `transition_seq`, `last_change_at` and the transition event
//! are written in one transaction; repeated frames in the same state change nothing in the DB (D5).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agentd_core::{
    Clock, EventLevel, IndicatorError, IndicatorState, IndicatorStore, NewEvent, Notifier,
    Observed, PortFuture, Reading,
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};

use crate::{Db, DbError};

pub struct IndicatorRepo {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
    notifier: Arc<dyn Notifier>,
    /// State by `id` known to the writer: a repeated frame without a state change does not go to the DB.
    states: Mutex<HashMap<String, IndicatorState>>,
    /// The last indicator frame (live `value` and `text` are not stored in the DB).
    frames: Mutex<HashMap<String, Value>>,
}

impl IndicatorRepo {
    #[must_use]
    pub fn new(db: Arc<Db>, clock: Arc<dyn Clock>, notifier: Arc<dyn Notifier>) -> IndicatorRepo {
        IndicatorRepo {
            db,
            clock,
            notifier,
            states: Mutex::new(HashMap::new()),
            frames: Mutex::new(HashMap::new()),
        }
    }
}

fn unavailable(e: &DbError) -> IndicatorError {
    tracing::warn!(scope = "indicators", event = "db_unavailable", error = %e, "indicators DB unavailable");
    IndicatorError::Unavailable
}

fn frame(id: &str, r: &Reading, ts: i64) -> Value {
    json!({"id": id, "state": r.state.as_str(), "value": r.value, "text": r.text, "ts": ts})
}

/// Transition event level; the first frame (`unknown` → `ok`/`nodata`) is initialization without an event.
fn event_for(id: &str, from: IndicatorState, r: &Reading, seq: i64) -> Option<NewEvent> {
    use IndicatorState::{Bad, Nodata, Ok, Unknown, Warn};
    if from == Unknown && matches!(r.state, Ok | Nodata) {
        return None;
    }
    let level = match r.state {
        Ok => EventLevel::Ok,
        Warn => EventLevel::Warn,
        Bad => EventLevel::Error,
        Nodata | Unknown => EventLevel::Info,
    };
    let what = r
        .text
        .clone()
        .unwrap_or_else(|| r.state.as_str().to_string());
    Some(NewEvent::new("msg", "sys", level, format!("{id}: {what}")).dedup(format!("{id}:{seq}")))
}

/// Transition transaction: `(new state written?, event)`.
fn transit(
    tx: &Transaction<'_>,
    id: &str,
    r: &Reading,
    ts: i64,
) -> rusqlite::Result<(IndicatorState, bool, Option<agentd_core::Event>)> {
    let row: Option<(String, i64)> = tx
        .query_row(
            "SELECT last_state, transition_seq FROM indicators WHERE id = ?1",
            [id],
            |x| Ok((x.get(0)?, x.get(1)?)),
        )
        .optional()?;
    let (from, seq) = match &row {
        Some((s, n)) => (
            IndicatorState::parse(s).unwrap_or(IndicatorState::Unknown),
            *n,
        ),
        None => (IndicatorState::Unknown, 0),
    };
    if row.is_none() {
        tx.execute("INSERT INTO indicators(id) VALUES (?1)", [id])?;
    }
    if from == r.state {
        return Ok((from, false, None));
    }
    let next = seq + 1;
    tx.execute(
        "UPDATE indicators SET last_state = ?2, transition_seq = ?3, last_change_at = ?4 WHERE id = ?1",
        params![id, r.state.as_str(), next, ts],
    )?;
    let event = match event_for(id, from, r, next) {
        Some(e) => crate::events::insert_event(tx, ts, &e)?,
        None => None,
    };
    Ok((from, true, event))
}

impl IndicatorStore for IndicatorRepo {
    fn observe(
        &self,
        id: String,
        reading: Reading,
    ) -> PortFuture<'_, Result<Observed, IndicatorError>> {
        Box::pin(async move {
            let ts = self.clock.now_unix();
            let out = frame(&id, &reading, ts);
            let known = self
                .states
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&id)
                .copied();
            let mut transitioned = false;
            if known != Some(reading.state) {
                let (key, r) = (id.clone(), reading.clone());
                let (_, changed, event) = self
                    .db
                    .write(move |tx| transit(tx, &key, &r, ts))
                    .await
                    .map_err(|e| unavailable(&e))?;
                transitioned = changed;
                self.states
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.clone(), reading.state);
                if let Some(e) = event {
                    self.notifier.publish_event("events", e.to_json());
                }
            }
            self.frames
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id, out.clone());
            self.notifier.publish("bar.indicators", out.clone());
            Ok(Observed {
                frame: out,
                transitioned,
            })
        })
    }

    fn snapshot(&self) -> PortFuture<'_, Result<Value, IndicatorError>> {
        Box::pin(async move {
            let rows: Vec<(String, String, Option<i64>)> = self
                .db
                .read(|c| {
                    c.prepare("SELECT id, last_state, last_change_at FROM indicators WHERE last_state <> 'unknown' OR transition_seq > 0 ORDER BY id")?
                        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                        .collect()
                })
                .await
                .map_err(|e| unavailable(&e))?;
            let frames = self.frames.lock().unwrap_or_else(|e| e.into_inner());
            let items: Vec<Value> = rows
                .into_iter()
                .map(|(id, state, at)| {
                    frames.get(&id).cloned().unwrap_or_else(|| {
                        json!({"id": id, "state": state, "value": null, "text": null, "ts": at.unwrap_or(0)})
                    })
                })
                .collect();
            Ok(json!({ "items": items }))
        })
    }
}
