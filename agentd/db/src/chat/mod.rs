//! Chat history (`ChatStore`, A3): messages in `messages`, attachments as files in `~/.agentos/attachments/`.
//! Writes go through the single writer; each operation is one transaction.

mod attach;
mod context;
mod messages;
mod store;

use std::path::PathBuf;
use std::sync::Arc;

use agentd_core::{ChatError, Clock};

use crate::{Db, DbError};

pub struct ChatRepo {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
    /// `~/.agentos`: `AGENT.md` and the `attachments/` directory.
    root: PathBuf,
    attachments: attach::Attachments,
}

impl ChatRepo {
    #[must_use]
    pub fn new(db: Arc<Db>, clock: Arc<dyn Clock>, root: impl Into<PathBuf>) -> ChatRepo {
        let root = root.into();
        ChatRepo {
            db,
            clock,
            attachments: attach::Attachments::new(root.join("attachments")),
            root,
        }
    }

    fn now(&self) -> i64 {
        self.clock.now_unix()
    }
}

pub(crate) fn unavailable(e: &DbError) -> ChatError {
    tracing::warn!(scope = "chat", event = "db_unavailable", error = %e, "chat DB unavailable");
    ChatError::Unavailable
}
