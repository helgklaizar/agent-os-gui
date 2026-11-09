//! Search (`SearchStore`, A6-08): index `~/.agentos/index.db` (FTS5), filled by syncing with `sessions.db` (read only)
//! and the `AGENT.md` and `memory/` files. The index gets session names, ready `user` and `agent` messages, and files; tool
//! results, system messages, `meta_json`, `args_json` and secrets are not indexed. Sync runs on a query.

mod index;
mod sync;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agentd_core::{PortFuture, SearchError, SearchHit, SearchStore};
use rusqlite::Connection;

use crate::Db;

pub struct SearchRepo {
    db: Arc<Db>,
    /// `~/.agentos`: `AGENT.md`, `memory/`, `index.db`.
    root: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

impl SearchRepo {
    /// Opens (creates) the index; an index file error refuses to start search, not the core.
    pub fn open(db: Arc<Db>, root: impl Into<PathBuf>) -> Result<SearchRepo, crate::DbError> {
        let root = root.into();
        let conn = index::open(&root.join("index.db"))?;
        Ok(SearchRepo {
            db,
            root,
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

fn unavailable(what: &str) -> SearchError {
    tracing::warn!(
        scope = "search",
        event = "unavailable",
        what,
        "search unavailable"
    );
    SearchError::Unavailable
}

impl SearchStore for SearchRepo {
    fn search(
        &self,
        query: String,
        limit: usize,
    ) -> PortFuture<'_, Result<Vec<SearchHit>, SearchError>> {
        Box::pin(async move {
            self.sync().await.map_err(|()| unavailable("sync"))?;
            let conn = self.conn.clone();
            tokio::task::spawn_blocking(move || {
                let c = conn
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                index::query(&c, &query, limit)
            })
            .await
            .map_err(|_| unavailable("join"))?
            .map_err(|_| unavailable("query"))
        })
    }
}
