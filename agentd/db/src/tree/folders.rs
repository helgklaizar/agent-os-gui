//! Folders (docs/reference/db.md sections 3.1, 5): nesting without cycles and no deeper than 8 is kept by DB triggers; deletion moves the contents up.

use agentd_core::{FolderInput, TreeError};
use rusqlite::{OptionalExtension, Transaction};
use serde_json::{Value, json};

use super::run::{Done, Op};
use super::sessions::derived_updates;
use super::{Event, json};

fn folder_event(tx: &Transaction<'_>, op: &str, id: i64) -> rusqlite::Result<Option<Event>> {
    Ok(json::folder(tx, id)?.map(|f| ("sessions", json!({"op": op, "folder": f}))))
}

fn check_links(tx: &Transaction<'_>, input: &FolderInput) -> rusqlite::Result<Option<TreeError>> {
    if let Some(Some(p)) = input.parent_id
        && !json::alive(tx, "folders", p)?
    {
        return Ok(Some(TreeError::field("parent_id", "no such folder")));
    }
    if let Some(Some(p)) = input.persona_id
        && !json::alive(tx, "personas", p)?
    {
        return Ok(Some(TreeError::field("persona_id", "no such persona")));
    }
    Ok(None)
}

pub(crate) fn create(
    tx: &Transaction<'_>,
    now: i64,
    input: FolderInput,
) -> rusqlite::Result<Op<Value>> {
    if let Some(e) = check_links(tx, &input)? {
        return Ok(Err(e));
    }
    tx.execute(
        "INSERT INTO folders(parent_id, name, persona_id, open, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            input.parent_id.flatten(), input.name, input.persona_id.flatten(),
            input.open.unwrap_or(true), now
        ],
    )?;
    let id = tx.last_insert_rowid();
    let events = folder_event(tx, "folder.created", id)?
        .into_iter()
        .collect();
    Ok(Ok(Done::new(
        json::folder(tx, id)?.unwrap_or_default(),
        events,
    )))
}

pub(crate) fn patch(
    tx: &Transaction<'_>,
    id: i64,
    input: FolderInput,
) -> rusqlite::Result<Op<Value>> {
    if !json::alive(tx, "folders", id)? {
        return Ok(Err(TreeError::NotFound));
    }
    if let Some(e) = check_links(tx, &input)? {
        return Ok(Err(e));
    }
    let before = json::session_views(tx)?;
    // One UPDATE: a cycle or depth trigger rolls back the whole PATCH, including the name.
    let mut sets: Vec<(&str, rusqlite::types::Value)> = Vec::new();
    if let Some(n) = input.name {
        sets.push(("name", n.into()));
    }
    if let Some(p) = input.parent_id {
        sets.push((
            "parent_id",
            p.map_or(rusqlite::types::Value::Null, Into::into),
        ));
    }
    if let Some(p) = input.persona_id {
        sets.push((
            "persona_id",
            p.map_or(rusqlite::types::Value::Null, Into::into),
        ));
    }
    if let Some(o) = input.open {
        sets.push(("open", i64::from(o).into()));
    }
    if !sets.is_empty() {
        let list = sets
            .iter()
            .enumerate()
            .map(|(i, (c, _))| format!("{c} = ?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let mut values: Vec<rusqlite::types::Value> = sets.into_iter().map(|(_, v)| v).collect();
        values.push(id.into());
        tx.execute(
            &format!("UPDATE folders SET {list} WHERE id = ?{}", values.len()),
            rusqlite::params_from_iter(values),
        )?;
    }
    let mut events: Vec<Event> = folder_event(tx, "folder.updated", id)?
        .into_iter()
        .collect();
    events.extend(derived_updates(tx, &before, None)?);
    Ok(Ok(Done::new(
        json::folder(tx, id)?.unwrap_or_default(),
        events,
    )))
}

/// Nested folders and sessions go to the deleted folder's parent (to the root if there is none), then the row is deleted.
pub(crate) fn delete(tx: &Transaction<'_>, id: i64) -> rusqlite::Result<Op<()>> {
    let parent: Option<Option<i64>> = tx
        .query_row("SELECT parent_id FROM folders WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(parent) = parent else {
        return Ok(Err(TreeError::NotFound));
    };
    let before = json::session_views(tx)?;
    let children: Vec<i64> = tx
        .prepare("SELECT id FROM folders WHERE parent_id = ?1 ORDER BY id")?
        .query_map([id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    tx.execute(
        "UPDATE folders SET parent_id = ?2 WHERE parent_id = ?1",
        rusqlite::params![id, parent],
    )?;
    tx.execute(
        "UPDATE sessions SET folder_id = ?2 WHERE folder_id = ?1",
        rusqlite::params![id, parent],
    )?;
    tx.execute("DELETE FROM folders WHERE id = ?1", [id])?;
    let mut events = Vec::new();
    for c in children {
        events.extend(folder_event(tx, "folder.updated", c)?);
    }
    events.extend(derived_updates(tx, &before, None)?);
    events.push((
        "sessions",
        json!({"op": "folder.deleted", "folder": {"id": id}}),
    ));
    Ok(Ok(Done::new((), events)))
}
