//! Gateway store (`GatewayStore`, A5-05): contact and message are one writer transaction.

use std::sync::Arc;

use agentd_core::{GatewayError, GatewayStore, NewGwMessage, PortFuture, Stored};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::{Db, DbError};

pub struct GatewayRepo {
    db: Arc<Db>,
}

impl GatewayRepo {
    #[must_use]
    pub fn new(db: Arc<Db>) -> GatewayRepo {
        GatewayRepo { db }
    }
}

fn unavailable(e: &DbError) -> GatewayError {
    tracing::warn!(scope = "gateway", event = "db_unavailable", error = %e, "gateway DB unavailable");
    GatewayError::Unavailable
}

/// Contact by name: if it exists, channels and persona are updated; otherwise it is created.
fn upsert_contact(tx: &Transaction<'_>, c: &agentd_core::Contact) -> rusqlite::Result<i64> {
    let found: Option<i64> = tx
        .query_row(
            "SELECT id FROM contacts WHERE name = ?1 ORDER BY id LIMIT 1",
            [&c.name],
            |r| r.get(0),
        )
        .optional()?;
    match found {
        Some(id) => {
            tx.execute(
                "UPDATE contacts SET handles_json = ?2, persona_id = (SELECT id FROM personas WHERE id = ?3) WHERE id = ?1",
                params![id, c.handles_json, c.persona_id],
            )?;
            Ok(id)
        }
        None => {
            tx.execute(
                "INSERT INTO contacts(name, handles_json, persona_id) VALUES (?1, ?2, (SELECT id FROM personas WHERE id = ?3))",
                params![c.name, c.handles_json, c.persona_id],
            )?;
            Ok(tx.last_insert_rowid())
        }
    }
}

fn store_tx(tx: &Transaction<'_>, m: &NewGwMessage) -> rusqlite::Result<Option<Stored>> {
    let dup: i64 = tx.query_row(
        "SELECT count(*) FROM gw_messages WHERE account_id = ?1 AND ext_id = ?2",
        params![m.account_id, m.ext_id],
        |r| r.get(0),
    )?;
    if dup > 0 {
        return Ok(None);
    }
    let contact_id = m
        .contact
        .as_ref()
        .map(|c| upsert_contact(tx, c))
        .transpose()?;
    tx.execute(
        "INSERT INTO gw_messages(account_id, ext_id, direction, contact_id, body, routed_session_id, isolated, read, ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
        params![
            m.account_id,
            m.ext_id,
            m.direction.as_str(),
            contact_id,
            m.body,
            m.routed_session_id,
            i64::from(m.isolated),
            m.ts
        ],
    )?;
    Ok(Some(Stored {
        id: tx.last_insert_rowid(),
        contact_id,
    }))
}

impl GatewayStore for GatewayRepo {
    fn account_type(
        &self,
        account_id: i64,
    ) -> PortFuture<'_, Result<Option<String>, GatewayError>> {
        Box::pin(async move {
            self.db
                .read(move |c| {
                    c.query_row(
                        "SELECT type FROM accounts WHERE id = ?1 AND deleted_at IS NULL",
                        [account_id],
                        |r| r.get(0),
                    )
                    .optional()
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn store(&self, m: NewGwMessage) -> PortFuture<'_, Result<Option<Stored>, GatewayError>> {
        Box::pin(async move {
            self.db
                .write(move |tx| store_tx(tx, &m))
                .await
                .map_err(|e| unavailable(&e))
        })
    }
}
