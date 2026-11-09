//! `StatusSource` port over `Db`: schema version and write queue length for `/health` and `/metrics`.

use agentd_core::{PortFuture, StatusSource};

use crate::{Db, user_version};

impl StatusSource for Db {
    fn user_version(&self) -> PortFuture<'_, Option<i64>> {
        Box::pin(async move {
            match self.read(|c| Ok(user_version(c))).await {
                Ok(Ok(v)) => Some(v),
                // A version read failure does not bring `/health` down: the field stays empty, the cause is in the log.
                Ok(Err(_)) | Err(_) => {
                    tracing::warn!(
                        scope = "migrator",
                        event = "user_version_unreadable",
                        "schema version not read"
                    );
                    None
                }
            }
        })
    }

    fn write_queue_len(&self) -> usize {
        self.queue_len()
    }
}
