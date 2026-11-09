//! Event feed (`EventStore`, A5-03): a write in one writer transaction, publication to the `events` topic after commit.

mod sql;
mod store;

use std::sync::Arc;

use agentd_core::{Clock, EventError, Notifier};

use crate::{Db, DbError};

pub(crate) use sql::{insert as insert_event, mark_session_events};

pub struct EventRepo {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
    notifier: Arc<dyn Notifier>,
}

impl EventRepo {
    #[must_use]
    pub fn new(db: Arc<Db>, clock: Arc<dyn Clock>, notifier: Arc<dyn Notifier>) -> EventRepo {
        EventRepo {
            db,
            clock,
            notifier,
        }
    }
}

pub(crate) fn unavailable(e: &DbError) -> EventError {
    tracing::warn!(scope = "events", event = "db_unavailable", error = %e, "events DB unavailable");
    EventError::Unavailable
}
