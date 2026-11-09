//! Settings pages over the `settings` table (docs/reference/db.md section 3.12): `key` is the page name, `value_json` is the whole document,
//! `rev` grows on every write. A page not yet in the DB corresponds to the default document and `rev = 0`.

use std::collections::BTreeMap;
use std::sync::Arc;

use agentd_core::{
    Clock, FieldError, PortFuture, SettingsDoc, SettingsError, SettingsStore, default_doc,
    is_known_page, is_read_only, merge_patch, validate_settings_patch,
};
use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::{Db, DbError};

pub struct SettingsRepo {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
}

/// Outcome of a write inside a transaction: refusals are returned as values so the transaction rolls back without a DB error.
enum Outcome {
    Saved(Value, i64),
    Stale(i64),
    Invalid(Vec<FieldError>),
    Corrupt,
}

impl SettingsRepo {
    #[must_use]
    pub fn new(db: Arc<Db>, clock: Arc<dyn Clock>) -> SettingsRepo {
        SettingsRepo { db, clock }
    }
}

fn unavailable(e: &DbError) -> SettingsError {
    tracing::warn!(scope = "settings", event = "db_unavailable", error = %e, "settings DB unavailable");
    SettingsError::Unavailable
}

/// `rev` is a service field of the response; it is absent from the document written to the DB.
fn without_rev(mut doc: Value) -> Value {
    if let Value::Object(o) = &mut doc {
        o.remove("rev");
    }
    doc
}

impl SettingsStore for SettingsRepo {
    fn get<'a>(&'a self, page: &'a str) -> PortFuture<'a, Result<SettingsDoc, SettingsError>> {
        Box::pin(async move {
            if !is_known_page(page) {
                return Err(SettingsError::UnknownPage);
            }
            let key = page.to_string();
            let row: Option<(String, i64)> = self
                .db
                .read(move |c| {
                    c.query_row(
                        "SELECT value_json, rev FROM settings WHERE key = ?1",
                        [key],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                })
                .await
                .map_err(|e| unavailable(&e))?;
            match row {
                None => Ok(SettingsDoc {
                    doc: default_doc(page),
                    rev: 0,
                }),
                Some((text, rev)) => match serde_json::from_str(&text) {
                    Ok(doc) => Ok(SettingsDoc { doc, rev }),
                    Err(_) => {
                        tracing::error!(
                            scope = "settings",
                            event = "row_corrupt",
                            "page document is not JSON"
                        );
                        Err(SettingsError::Unavailable)
                    }
                },
            }
        })
    }

    fn patch<'a>(
        &'a self,
        page: &'a str,
        patch: Value,
        if_match: i64,
    ) -> PortFuture<'a, Result<SettingsDoc, SettingsError>> {
        Box::pin(async move {
            if !is_known_page(page) {
                return Err(SettingsError::UnknownPage);
            }
            if is_read_only(page) {
                return Err(SettingsError::ReadOnly);
            }
            let (key, patch, now) = (page.to_string(), without_rev(patch), self.clock.now_unix());
            // Reading the current version, comparing `If-Match`, merging and writing are one transaction of the single writer:
            // no foreign edit can wedge in between the check and the write.
            let outcome = self
                .db
                .write(move |tx| {
                    let row: Option<(String, i64)> = tx
                        .query_row(
                            "SELECT value_json, rev FROM settings WHERE key = ?1",
                            [&key],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    let (current, rev) = match row {
                        None => (default_doc(&key), 0),
                        Some((text, rev)) => match serde_json::from_str(&text) {
                            Ok(doc) => (doc, rev),
                            Err(_) => return Ok(Outcome::Corrupt),
                        },
                    };
                    if rev != if_match {
                        return Ok(Outcome::Stale(rev));
                    }
                    let merged = without_rev(merge_patch(&current, &patch));
                    let errors = validate_settings_patch(&key, &merged, &current);
                    if !errors.is_empty() {
                        return Ok(Outcome::Invalid(errors));
                    }
                    let text = serde_json::to_string(&merged)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                    tx.execute(
                        "INSERT INTO settings(key, value_json, rev, updated_at) VALUES (?1, ?2, 1, ?3) \
                         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, \
                         rev = settings.rev + 1, updated_at = excluded.updated_at",
                        rusqlite::params![key, text, now],
                    )?;
                    Ok(Outcome::Saved(merged, rev + 1))
                })
                .await
                .map_err(|e| unavailable(&e))?;
            match outcome {
                Outcome::Saved(doc, rev) => Ok(SettingsDoc { doc, rev }),
                Outcome::Stale(current) => Err(SettingsError::Stale { current }),
                Outcome::Invalid(f) => Err(SettingsError::Invalid(f)),
                Outcome::Corrupt => {
                    tracing::error!(
                        scope = "settings",
                        event = "row_corrupt",
                        "page document is not JSON"
                    );
                    Err(SettingsError::Unavailable)
                }
            }
        })
    }

    fn get_list<'a>(
        &'a self,
        key: &'a str,
        default: Value,
    ) -> PortFuture<'a, Result<SettingsDoc, SettingsError>> {
        Box::pin(async move {
            let k = key.to_string();
            let row: Option<(String, i64)> = self
                .db
                .read(move |c| {
                    c.query_row(
                        "SELECT value_json, rev FROM settings WHERE key = ?1",
                        [k],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                })
                .await
                .map_err(|e| unavailable(&e))?;
            match row {
                None => Ok(SettingsDoc {
                    doc: default,
                    rev: 0,
                }),
                Some((text, rev)) => serde_json::from_str(&text)
                    .map(|doc| SettingsDoc { doc, rev })
                    .map_err(|_| SettingsError::Unavailable),
            }
        })
    }

    fn put_list<'a>(
        &'a self,
        key: &'a str,
        list: Value,
        if_match: i64,
    ) -> PortFuture<'a, Result<SettingsDoc, SettingsError>> {
        Box::pin(async move {
            let (key, now) = (key.to_string(), self.clock.now_unix());
            let text = serde_json::to_string(&list)
                .map_err(|_| SettingsError::Invalid(vec![FieldError::new("body", "not JSON")]))?;
            let saved = self
                .db
                .write(move |tx| {
                    let cur: i64 = tx
                        .query_row("SELECT rev FROM settings WHERE key = ?1", [&key], |r| {
                            r.get(0)
                        })
                        .optional()?
                        .unwrap_or(0);
                    if cur != if_match {
                        return Ok(Err(cur));
                    }
                    tx.execute(
                        "INSERT INTO settings(key, value_json, rev, updated_at) VALUES (?1, ?2, 1, ?3) \
                         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, \
                         rev = settings.rev + 1, updated_at = excluded.updated_at",
                        rusqlite::params![key, text, now],
                    )?;
                    Ok(Ok(cur + 1))
                })
                .await
                .map_err(|e| unavailable(&e))?;
            match saved {
                Ok(rev) => Ok(SettingsDoc { doc: list, rev }),
                Err(current) => Err(SettingsError::Stale { current }),
            }
        })
    }

    fn revisions(&self) -> PortFuture<'_, Result<BTreeMap<String, i64>, SettingsError>> {
        Box::pin(async move {
            self.db
                .read(|c| {
                    let mut stmt = c.prepare("SELECT key, rev FROM settings")?;
                    let rows =
                        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                    rows.collect::<Result<Vec<_>, _>>()
                })
                .await
                .map(|rows| rows.into_iter().filter(|(k, _)| is_known_page(k)).collect())
                .map_err(|e| unavailable(&e))
        })
    }
}
