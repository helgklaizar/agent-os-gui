//! Personas (docs/reference/db.md section 3.4): fields, account list, soft delete (section 5), `rev` of the `personas` settings page.

use agentd_core::{PersonaInput, TreeError};
use rusqlite::{OptionalExtension, Transaction};
use serde_json::{Value, json};

use super::run::{Done, Op};
use super::{Event, json};

/// Bumps `rev` of the `personas` page and returns a `settings` topic delta. If `reset_priority` is the id of the persona being deleted,
/// resets `balance.priority_persona_id` when it points to it.
pub(crate) fn bump_page(
    tx: &Transaction<'_>,
    now: i64,
    reset_priority: Option<i64>,
) -> rusqlite::Result<Event> {
    let text: Option<String> = tx
        .query_row(
            "SELECT value_json FROM settings WHERE key = 'personas'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let mut doc: Value = text
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(id) = reset_priority
        && let Some(b) = doc.get_mut("balance").and_then(Value::as_object_mut)
        && b.get("priority_persona_id").and_then(Value::as_i64) == Some(id)
    {
        b.remove("priority_persona_id");
    }
    tx.execute(
        "INSERT INTO settings(key, value_json, rev, updated_at) VALUES ('personas', ?1, 1, ?2)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, rev = settings.rev + 1, updated_at = excluded.updated_at",
        rusqlite::params![doc.to_string(), now],
    )?;
    let rev: i64 = tx.query_row("SELECT rev FROM settings WHERE key = 'personas'", [], |r| {
        r.get(0)
    })?;
    Ok(("settings", json!({"page": "personas", "rev": rev})))
}

fn check_links(tx: &Transaction<'_>, input: &PersonaInput) -> rusqlite::Result<Option<TreeError>> {
    if let Some(Some(acc)) = input.model_acc
        && !json::alive(tx, "accounts", acc)?
    {
        return Ok(Some(TreeError::field("model_acc", "no such account")));
    }
    for acc in input.account_ids.iter().flatten() {
        if !json::alive(tx, "accounts", *acc)? {
            return Ok(Some(TreeError::field("account_ids", "no such account")));
        }
    }
    Ok(None)
}

fn set_accounts(tx: &Transaction<'_>, id: i64, ids: &[i64]) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM persona_accounts WHERE persona_id = ?1", [id])?;
    for acc in ids {
        tx.execute(
            "INSERT INTO persona_accounts(persona_id, account_id) VALUES (?1, ?2)",
            [id, *acc],
        )?;
    }
    Ok(())
}

pub(crate) fn create(
    tx: &Transaction<'_>,
    now: i64,
    input: PersonaInput,
) -> rusqlite::Result<Op<Value>> {
    if let Some(e) = check_links(tx, &input)? {
        return Ok(Err(e));
    }
    tx.execute(
        "INSERT INTO personas(name, color, model_acc, style, lang, signature, mem_mode, notif_mode, work_hours, quiet_hours)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            input.name, input.color, input.model_acc.flatten(),
            input.style.unwrap_or_default(), input.lang.unwrap_or_else(|| "ru".into()),
            input.signature.unwrap_or_default(), input.mem_mode.unwrap_or_else(|| "own".into()),
            input.notif_mode.unwrap_or_else(|| "all".into()),
            input.work_hours.unwrap_or_default(), input.quiet_hours.unwrap_or_default(),
        ],
    )?;
    let id = tx.last_insert_rowid();
    set_accounts(tx, id, input.account_ids.as_deref().unwrap_or(&[]))?;
    let event = bump_page(tx, now, None)?;
    Ok(Ok(Done::new(
        json::persona(tx, id)?.unwrap_or_default(),
        vec![event],
    )))
}

pub(crate) fn patch(
    tx: &Transaction<'_>,
    now: i64,
    id: i64,
    input: PersonaInput,
) -> rusqlite::Result<Op<Value>> {
    if !json::alive(tx, "personas", id)? {
        return Ok(Err(TreeError::NotFound));
    }
    if let Some(e) = check_links(tx, &input)? {
        return Ok(Err(e));
    }
    let mut sets: Vec<(&str, rusqlite::types::Value)> = Vec::new();
    let text = |v: Option<String>| v.map(rusqlite::types::Value::Text);
    let columns = [
        ("name", text(input.name)),
        ("color", text(input.color)),
        ("style", text(input.style)),
        ("lang", text(input.lang)),
        ("signature", text(input.signature)),
        ("mem_mode", text(input.mem_mode)),
        ("notif_mode", text(input.notif_mode)),
        ("work_hours", text(input.work_hours)),
        ("quiet_hours", text(input.quiet_hours)),
    ];
    sets.extend(columns.into_iter().filter_map(|(c, v)| Some((c, v?))));
    if let Some(m) = input.model_acc {
        sets.push((
            "model_acc",
            m.map_or(
                rusqlite::types::Value::Null,
                rusqlite::types::Value::Integer,
            ),
        ));
    }
    if !sets.is_empty() {
        let list = sets
            .iter()
            .enumerate()
            .map(|(i, (c, _))| format!("{c} = ?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let mut values: Vec<rusqlite::types::Value> = sets.into_iter().map(|(_, v)| v).collect();
        values.push(rusqlite::types::Value::Integer(id));
        tx.execute(
            &format!("UPDATE personas SET {list} WHERE id = ?{}", values.len()),
            rusqlite::params_from_iter(values),
        )?;
    }
    if let Some(ids) = &input.account_ids {
        set_accounts(tx, id, ids)?;
    }
    let event = bump_page(tx, now, None)?;
    Ok(Ok(Done::new(
        json::persona(tx, id)?.unwrap_or_default(),
        vec![event],
    )))
}

/// Soft delete (docs/reference/db.md section 5): `deleted_at` is set, session and folder references are nulled, `persona_accounts` are removed,
/// the balance priority is reset. Objects whose view changed get `folder.updated` / `session.updated`.
pub(crate) fn delete(tx: &Transaction<'_>, now: i64, id: i64) -> rusqlite::Result<Op<()>> {
    if !json::alive(tx, "personas", id)? {
        return Ok(Err(TreeError::NotFound));
    }
    let before = json::session_views(tx)?;
    let folder_ids: Vec<i64> = tx
        .prepare("SELECT id FROM folders WHERE persona_id = ?1 ORDER BY id")?
        .query_map([id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    tx.execute(
        "UPDATE folders SET persona_id = NULL WHERE persona_id = ?1",
        [id],
    )?;
    tx.execute(
        "UPDATE sessions SET persona_id = NULL WHERE persona_id = ?1",
        [id],
    )?;
    tx.execute("DELETE FROM persona_accounts WHERE persona_id = ?1", [id])?;
    tx.execute(
        "UPDATE personas SET deleted_at = ?2 WHERE id = ?1",
        rusqlite::params![id, now],
    )?;
    let mut events = vec![bump_page(tx, now, Some(id))?];
    for f in folder_ids {
        events.extend(
            json::folder(tx, f)?
                .map(|v| ("sessions", json!({"op": "folder.updated", "folder": v}))),
        );
    }
    events.extend(super::sessions::derived_updates(tx, &before, None)?);
    Ok(Ok(Done::new((), events)))
}
