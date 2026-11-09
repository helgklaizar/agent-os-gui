//! Sessions and threads (docs/reference/db.md sections 3.1, 5): creation, `PATCH` without mixing move and rename, `cfg`, path,
//! deletion with bookmark cleanup, the `unread` counter.

use std::collections::BTreeMap;
use std::path::Path;

use agentd_core::{SessionInput, TreeError};
use rusqlite::{OptionalExtension, Transaction};
use serde_json::{Map, Value, json};

use super::run::{Done, Op};
use super::{Event, json};

fn event(op: &str, session: &Value) -> Event {
    ("sessions", json!({"op": op, "session": session}))
}

/// `session.updated` for objects whose view (persona, path, folder…) changed since `before`; `skip` is an already notified object.
pub(crate) fn derived_updates(
    tx: &Transaction<'_>,
    before: &BTreeMap<i64, Value>,
    skip: Option<i64>,
) -> rusqlite::Result<Vec<Event>> {
    let after = json::session_views(tx)?;
    Ok(after
        .iter()
        .filter(|(id, v)| Some(**id) != skip && before.get(id).is_some_and(|b| b != *v))
        .map(|(_, v)| event("session.updated", v))
        .collect())
}

/// Session links: folder, persona, path (symlinks leading out of home), `cfg.model`.
fn check_links(
    tx: &Transaction<'_>,
    input: &SessionInput,
    home: Option<&Path>,
) -> rusqlite::Result<Option<TreeError>> {
    if let Some(Some(f)) = input.folder_id
        && !json::alive(tx, "folders", f)?
    {
        return Ok(Some(TreeError::field("folder_id", "no such folder")));
    }
    if let Some(Some(p)) = input.persona_id
        && !json::alive(tx, "personas", p)?
    {
        return Ok(Some(TreeError::field("persona_id", "no such persona")));
    }
    if let (Some(Some(p)), Some(home)) = (&input.path, home)
        && agentd_sys::paths::symlink_escapes_home(Path::new(p), home)
    {
        return Ok(Some(TreeError::field(
            "path",
            "symlink leaves the home directory",
        )));
    }
    if let Some(m) = input
        .cfg
        .as_ref()
        .and_then(|c| c.get("model"))
        .and_then(Value::as_i64)
        && !json::alive(tx, "accounts", m)?
    {
        return Ok(Some(TreeError::field("cfg.model", "no such account")));
    }
    Ok(None)
}

pub(crate) fn create(
    tx: &Transaction<'_>,
    now: i64,
    input: SessionInput,
    home: Option<&Path>,
) -> rusqlite::Result<Op<Value>> {
    if let Some(e) = check_links(tx, &input, home)? {
        return Ok(Err(e));
    }
    // The persona is not copied into the row: the effective one is computed on read.
    tx.execute(
        "INSERT INTO sessions(folder_id, name, persona_id, path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        rusqlite::params![input.folder_id.flatten(), input.name, input.persona_id.flatten(), input.path.flatten(), now],
    )?;
    let session = json::session(tx, tx.last_insert_rowid())?.unwrap_or_default();
    Ok(Ok(Done::new(
        session.clone(),
        vec![event("session.created", &session)],
    )))
}

pub(crate) fn create_thread(
    tx: &Transaction<'_>,
    now: i64,
    parent_id: i64,
    name: String,
) -> rusqlite::Result<Op<Value>> {
    let parent: Option<(Option<i64>, Option<i64>)> = tx
        .query_row(
            "SELECT parent_id, folder_id FROM sessions WHERE id = ?1",
            [parent_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((grand, folder)) = parent else {
        return Ok(Err(TreeError::NotFound));
    };
    if grand.is_some() {
        return Ok(Err(TreeError::field("id", "a thread cannot have threads")));
    }
    tx.execute(
        "INSERT INTO sessions(folder_id, parent_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        rusqlite::params![folder, parent_id, name, now],
    )?;
    let session = json::session(tx, tx.last_insert_rowid())?.unwrap_or_default();
    Ok(Ok(Done::new(
        session.clone(),
        vec![event("session.created", &session)],
    )))
}

/// Merge of `cfg` by keys: `null` removes the key.
fn merged_cfg(old: &str, patch: &Map<String, Value>) -> String {
    let mut cfg: Map<String, Value> = serde_json::from_str(old).unwrap_or_default();
    for (k, v) in patch {
        if v.is_null() {
            cfg.remove(k);
        } else {
            cfg.insert(k.clone(), v.clone());
        }
    }
    Value::Object(cfg).to_string()
}

pub(crate) fn patch(
    tx: &Transaction<'_>,
    now: i64,
    id: i64,
    input: SessionInput,
    home: Option<&Path>,
) -> rusqlite::Result<Op<Value>> {
    let row: Option<(Option<i64>, String)> = tx
        .query_row(
            "SELECT parent_id, cfg_json FROM sessions WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((parent, old_cfg)) = row else {
        return Ok(Err(TreeError::NotFound));
    };
    if parent.is_some() && input.folder_id.is_some() {
        return Ok(Err(TreeError::field(
            "folder_id",
            "a thread follows its parent",
        )));
    }
    if let Some(e) = check_links(tx, &input, home)? {
        return Ok(Err(e));
    }
    let before = json::session_views(tx)?;
    let mut sets: Vec<(&str, rusqlite::types::Value)> = vec![("updated_at", now.into())];
    let opt = |v: Option<i64>| v.map_or(rusqlite::types::Value::Null, Into::into);
    if let Some(n) = input.name {
        sets.push(("name", n.into()));
    }
    if let Some(f) = input.folder_id {
        sets.push(("folder_id", opt(f)));
    }
    if let Some(p) = input.persona_id {
        sets.push(("persona_id", opt(p)));
    }
    if let Some(p) = input.path {
        sets.push(("path", p.map_or(rusqlite::types::Value::Null, Into::into)));
    }
    if let Some(c) = &input.cfg {
        sets.push(("cfg_json", merged_cfg(&old_cfg, c).into()));
    }
    if let Some(a) = input.archived {
        sets.push(("archived", i64::from(a).into()));
    }
    let list = sets
        .iter()
        .enumerate()
        .map(|(i, (c, _))| format!("{c} = ?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let mut values: Vec<rusqlite::types::Value> = sets.into_iter().map(|(_, v)| v).collect();
    values.push(id.into());
    tx.execute(
        &format!("UPDATE sessions SET {list} WHERE id = ?{}", values.len()),
        rusqlite::params_from_iter(values),
    )?;
    if let Some(f) = input.folder_id {
        // Threads follow the parent; the name does not change.
        tx.execute(
            "UPDATE sessions SET folder_id = ?2 WHERE parent_id = ?1",
            rusqlite::params![id, f],
        )?;
    }
    let session = json::session(tx, id)?.unwrap_or_default();
    let mut events = vec![event("session.updated", &session)];
    events.extend(derived_updates(tx, &before, Some(id))?);
    Ok(Ok(Done::new(session, events)))
}

/// Deleting a session or thread: threads go by cascade, browser bookmarks (`browser.bookmarks.<id>`) are cleaned up here too,
/// until the trigger `trg_sessions_cleanup` arrives with 0004. One delta, for the deleted row; the client removes its threads too.
pub(crate) fn delete(tx: &Transaction<'_>, id: i64) -> rusqlite::Result<Op<()>> {
    if !json::alive(tx, "sessions", id)? {
        return Ok(Err(TreeError::NotFound));
    }
    let ids: Vec<i64> = tx
        .prepare("SELECT id FROM sessions WHERE id = ?1 OR parent_id = ?1")?
        .query_map([id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    // TODO(A4-04): open permission requests of this session are resolved as deny; TODO(A5-02): tasks pause, triggers turn off.
    tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
    for gone in ids {
        tx.execute(
            "DELETE FROM settings WHERE key = ?1",
            [format!("browser.bookmarks.{gone}")],
        )?;
    }
    Ok(Ok(Done::new(
        (),
        vec![(
            "sessions",
            json!({"op": "session.deleted", "session": {"id": id}}),
        )],
    )))
}

/// Changes the counter and returns an `unread` delta if the value changed.
fn set_unread(
    tx: &Transaction<'_>,
    id: i64,
    f: impl FnOnce(i64) -> i64,
) -> rusqlite::Result<Op<()>> {
    let current: Option<i64> = tx
        .query_row("SELECT unread FROM sessions WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(current) = current else {
        return Ok(Err(TreeError::NotFound));
    };
    let next = f(current);
    if next == current {
        return Ok(Ok(Done::new((), Vec::new())));
    }
    tx.execute("UPDATE sessions SET unread = ?2 WHERE id = ?1", [id, next])?;
    Ok(Ok(Done::new(
        (),
        vec![(
            "sessions",
            json!({"op": "unread", "session_id": id, "unread": next}),
        )],
    )))
}

/// Resets `unread` and marks the session's events as read (A5-03); the `read` delta goes to the `events` topic.
pub(crate) fn mark_read(tx: &Transaction<'_>, id: i64) -> rusqlite::Result<Op<()>> {
    let mut op = set_unread(tx, id, |_| 0)?;
    if let Ok(done) = &mut op {
        let ids = crate::events::mark_session_events(tx, id)?;
        if !ids.is_empty() {
            done.events.push(("events", json!({"read": ids})));
        }
    }
    Ok(op)
}

pub(crate) fn bump_unread(tx: &Transaction<'_>, id: i64) -> rusqlite::Result<Op<()>> {
    set_unread(tx, id, |n| n.saturating_add(1))
}
