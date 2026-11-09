//! `TunnelStore` (A6-05): tunnels in `tunnels` (docs/reference/db.md section 3.9).

use std::sync::Arc;

use agentd_core::{PortFuture, Tunnel, TunnelError, TunnelStore};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::Db;

const COLS: &str = "id, type, addr, descr, enabled";

pub struct TunnelRepo {
    db: Arc<Db>,
}

impl TunnelRepo {
    #[must_use]
    pub fn new(db: Arc<Db>) -> TunnelRepo {
        TunnelRepo { db }
    }
}

fn row(r: &Row<'_>) -> rusqlite::Result<Tunnel> {
    Ok(Tunnel {
        id: r.get(0)?,
        kind: r.get(1)?,
        addr: r.get(2)?,
        descr: r.get(3)?,
        enabled: r.get::<_, i64>(4)? != 0,
    })
}

fn get(c: &Connection, id: i64) -> rusqlite::Result<Option<Tunnel>> {
    c.query_row(
        &format!("SELECT {COLS} FROM tunnels WHERE id = ?1"),
        [id],
        row,
    )
    .optional()
}

fn unavailable(e: &crate::DbError) -> TunnelError {
    tracing::warn!(scope = "tunnels", event = "db_unavailable", error = %e, "tunnels DB unavailable");
    TunnelError::Unavailable
}

impl TunnelStore for TunnelRepo {
    fn create(&self, t: Tunnel) -> PortFuture<'_, Result<Tunnel, TunnelError>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    tx.execute(
                        "INSERT INTO tunnels(type, addr, descr, enabled) VALUES (?1, ?2, ?3, ?4)",
                        params![t.kind, t.addr, t.descr, i64::from(t.enabled)],
                    )?;
                    get(tx, tx.last_insert_rowid())
                })
                .await
                .map_err(|e| unavailable(&e))?
                .ok_or(TunnelError::NotFound)
        })
    }

    fn get(&self, id: i64) -> PortFuture<'_, Result<Tunnel, TunnelError>> {
        Box::pin(async move {
            self.db
                .read(move |c| get(c, id))
                .await
                .map_err(|e| unavailable(&e))?
                .ok_or(TunnelError::NotFound)
        })
    }

    fn list(&self) -> PortFuture<'_, Result<Vec<Tunnel>, TunnelError>> {
        Box::pin(async move {
            self.db
                .read(|c| {
                    c.prepare(&format!("SELECT {COLS} FROM tunnels ORDER BY id"))?
                        .query_map([], row)?
                        .collect()
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn update(&self, t: Tunnel) -> PortFuture<'_, Result<Tunnel, TunnelError>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    let n = tx.execute(
                        "UPDATE tunnels SET type = ?1, addr = ?2, descr = ?3, enabled = ?4 WHERE id = ?5",
                        params![t.kind, t.addr, t.descr, i64::from(t.enabled), t.id],
                    )?;
                    if n == 0 { Ok(None) } else { get(tx, t.id) }
                })
                .await
                .map_err(|e| unavailable(&e))?
                .ok_or(TunnelError::NotFound)
        })
    }

    fn delete(&self, id: i64) -> PortFuture<'_, Result<(), TunnelError>> {
        Box::pin(async move {
            let n = self
                .db
                .write(move |tx| tx.execute("DELETE FROM tunnels WHERE id = ?1", [id]))
                .await
                .map_err(|e| unavailable(&e))?;
            if n == 0 {
                Err(TunnelError::NotFound)
            } else {
                Ok(())
            }
        })
    }
}
