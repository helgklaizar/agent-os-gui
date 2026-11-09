//! `EventStore` for `EventRepo`.

use agentd_core::{
    EventError, EventFilter, EventResult, EventStore, FieldError, NewEvent, PortFuture,
    ReadSelector,
};
use serde_json::{Value, json};

use super::{EventRepo, sql, unavailable};

const TOPIC: &str = "events";

impl EventRepo {
    fn publish_read(&self, ids: &[i64]) {
        if !ids.is_empty() {
            self.notifier.publish(TOPIC, json!({"read": ids}));
        }
    }
}

fn valid(e: &NewEvent) -> bool {
    agentd_core::KINDS.contains(&e.kind)
        && !e.sub.is_empty()
        && e.sub.chars().count() <= agentd_core::MAX_SUB_CHARS
        && !e.text.trim().is_empty()
}

impl EventStore for EventRepo {
    fn emit(&self, event: NewEvent) -> PortFuture<'_, EventResult<Option<Value>>> {
        Box::pin(async move {
            if !valid(&event) {
                return Err(EventError::Invalid(vec![FieldError::new(
                    "event",
                    "kind, sub and text are required",
                )]));
            }
            let ts = self.clock.now_unix();
            let row = self
                .db
                .write(move |tx| sql::insert(tx, ts, &event))
                .await
                .map_err(|e| unavailable(&e))?;
            // Publishing happens after commit: a rollback leaves no false frames.
            Ok(row.map(|e| {
                let v = e.to_json();
                self.notifier.publish_event(TOPIC, v.clone());
                v
            }))
        })
    }

    fn list(&self, filter: EventFilter) -> PortFuture<'_, EventResult<Value>> {
        Box::pin(async move {
            self.db
                .read(move |c| sql::list(c, &filter))
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn mark_read(&self, selector: ReadSelector) -> PortFuture<'_, EventResult<usize>> {
        Box::pin(async move {
            let ids = self
                .db
                .write(move |tx| sql::mark_read(tx, &selector))
                .await
                .map_err(|e| unavailable(&e))?;
            self.publish_read(&ids);
            Ok(ids.len())
        })
    }

    fn mark_one(&self, id: i64) -> PortFuture<'_, EventResult<()>> {
        Box::pin(async move {
            let was = self
                .db
                .write(move |tx| sql::mark_one(tx, id))
                .await
                .map_err(|e| unavailable(&e))?;
            match was {
                None => Err(EventError::NotFound),
                Some(changed) => {
                    if changed {
                        self.publish_read(&[id]);
                    }
                    Ok(())
                }
            }
        })
    }

    fn snapshot(&self) -> PortFuture<'_, EventResult<Value>> {
        self.list(EventFilter::default())
    }
}
