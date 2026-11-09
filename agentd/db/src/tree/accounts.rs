//! Accounts (docs/reference/db.md section 3.4): creation with a stable `slug`, signature, soft delete with cleanup of dependent rows (section 5).

use agentd_core::{AccountInput, TreeError};
use rusqlite::{OptionalExtension, Transaction};
use serde_json::json;

use super::json;
use super::personas::bump_page;
use super::run::{Done, Op};

/// `slug` prefix by type (stable prototype names: `cl1`, `wa1`, `gm2`).
fn slug_prefix(kind: &str) -> String {
    match kind {
        "anthropic" => "cl".into(),
        "gmail" => "gm".into(),
        "ollama" => "ol".into(),
        other => other
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(2)
            .collect(),
    }
}

/// Next number for the prefix. Rows of deleted accounts stay, so a `slug` is not reused.
fn next_slug(tx: &Transaction<'_>, kind: &str) -> rusqlite::Result<String> {
    let prefix = slug_prefix(kind);
    let max: Option<i64> = tx.query_row(
        "SELECT max(CAST(substr(slug, ?2 + 1) AS INTEGER)) FROM accounts
         WHERE substr(slug, 1, ?2) = ?1 AND substr(slug, ?2 + 1) NOT GLOB '*[^0-9]*' AND length(slug) > ?2",
        rusqlite::params![prefix, i64::try_from(prefix.len()).unwrap_or(0)],
        |r| r.get(0),
    )?;
    Ok(format!("{prefix}{}", max.unwrap_or(0) + 1))
}

pub(crate) fn create(
    tx: &Transaction<'_>,
    now: i64,
    input: AccountInput,
) -> rusqlite::Result<Op<serde_json::Value>> {
    let kind = input.kind.unwrap_or_default();
    let slug = next_slug(tx, &kind)?;
    tx.execute(
        "INSERT INTO accounts(slug, type, label, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![slug, kind, input.label.unwrap_or_default(), now],
    )?;
    let id = tx.last_insert_rowid();
    let account = json::account(tx, id)?.unwrap_or_default();
    let event = (
        "connections",
        json!({"account_id": id, "status": "disconnected"}),
    );
    Ok(Ok(Done::new(account, vec![event])))
}

pub(crate) fn patch(
    tx: &Transaction<'_>,
    id: i64,
    input: AccountInput,
) -> rusqlite::Result<Op<serde_json::Value>> {
    if !json::alive(tx, "accounts", id)? {
        return Ok(Err(TreeError::NotFound));
    }
    if let Some(label) = input.label {
        tx.execute(
            "UPDATE accounts SET label = ?1 WHERE id = ?2",
            rusqlite::params![label, id],
        )?;
    }
    Ok(Ok(Done::new(
        json::account(tx, id)?.unwrap_or_default(),
        Vec::new(),
    )))
}

pub(crate) fn delete(tx: &Transaction<'_>, now: i64, id: i64) -> rusqlite::Result<Op<()>> {
    if !json::alive(tx, "accounts", id)? {
        return Ok(Err(TreeError::NotFound));
    }
    let secret: Option<String> = tx
        .query_row("SELECT secret_ref FROM accounts WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?
        .flatten();
    let touched_personas: i64 = tx.query_row(
        "SELECT count(*) FROM personas WHERE deleted_at IS NULL AND (model_acc = ?1
           OR id IN (SELECT persona_id FROM persona_accounts WHERE account_id = ?1))",
        [id],
        |r| r.get(0),
    )?;
    tx.execute(
        "UPDATE personas SET model_acc = NULL WHERE model_acc = ?1",
        [id],
    )?;
    for table in ["persona_accounts", "quota_samples", "quotas"] {
        tx.execute(&format!("DELETE FROM {table} WHERE account_id = ?1"), [id])?;
    }
    tx.execute(
        "UPDATE accounts SET deleted_at = ?2, secret_ref = NULL WHERE id = ?1",
        rusqlite::params![id, now],
    )?;
    let mut events = Vec::new();
    if touched_personas > 0 {
        events.push(bump_page(tx, now, None)?);
    }
    let mut done = Done::new((), events);
    done.secrets.extend(secret);
    Ok(Ok(done))
}

/// Binding a secret to a live account (`PUT /secrets/account.<id>`); gives no deltas: `secret_ref` does not go outside.
pub(crate) fn link_secret(
    tx: &Transaction<'_>,
    id: i64,
    secret_ref: Option<String>,
) -> rusqlite::Result<Op<()>> {
    if !json::alive(tx, "accounts", id)? {
        return Ok(Err(TreeError::NotFound));
    }
    tx.execute(
        "UPDATE accounts SET secret_ref = ?2 WHERE id = ?1",
        rusqlite::params![id, secret_ref],
    )?;
    Ok(Ok(Done::new((), Vec::new())))
}

/// Status after a key check; the `connections` delta goes out only if the status changed.
pub(crate) fn set_status(
    tx: &Transaction<'_>,
    id: i64,
    status: &'static str,
) -> rusqlite::Result<Op<()>> {
    if !json::alive(tx, "accounts", id)? {
        return Ok(Err(TreeError::NotFound));
    }
    let changed = tx.execute(
        "UPDATE accounts SET status = ?2 WHERE id = ?1 AND status <> ?2",
        rusqlite::params![id, status],
    )?;
    let events = if changed > 0 {
        vec![("connections", json!({"account_id": id, "status": status}))]
    } else {
        Vec::new()
    };
    Ok(Ok(Done::new((), events)))
}
