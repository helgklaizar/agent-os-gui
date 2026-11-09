//! api.md section 6 objects from DB rows: `Folder`, `Session` (with computed fields), `Persona`, `Account`.
//! `secret_ref` and `deleted_at` do not enter the objects.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, Row};
use serde_json::{Map, Value, json};

/// Columns: id, folder_id, parent_id, name, persona_id, effective_persona_id, path (inherited by a thread), cfg_json,
/// unread, archived, created_at, updated_at. Effective persona: own → the parent's persona (for a thread) → the nearest folder
/// up along `folders.parent_id` (depth at most 32, as in the triggers) → `NULL`.
const SESSION_SELECT: &str = "SELECT s.id, s.folder_id, s.parent_id, s.name, s.persona_id,
  COALESCE(s.persona_id, par.persona_id, (
    WITH RECURSIVE up(pid, parent, d) AS (
      SELECT f.persona_id, f.parent_id, 1 FROM folders f WHERE f.id = COALESCE(par.folder_id, s.folder_id)
      UNION ALL SELECT f.persona_id, f.parent_id, up.d + 1 FROM folders f JOIN up ON f.id = up.parent WHERE up.d < 32)
    SELECT pid FROM up WHERE pid IS NOT NULL ORDER BY d LIMIT 1)),
  COALESCE(s.path, par.path), s.cfg_json, s.unread, s.archived, s.created_at, s.updated_at
  FROM sessions s LEFT JOIN sessions par ON par.id = s.parent_id";

fn session_from_row(r: &Row<'_>) -> rusqlite::Result<Value> {
    let parent_id: Option<i64> = r.get(2)?;
    let cfg_text: String = r.get(7)?;
    let mut cfg: Map<String, Value> = serde_json::from_str(&cfg_text).unwrap_or_default();
    if parent_id.is_some() {
        // Thread context: shared with the parent by default (`ctx = none` turns it off).
        cfg.entry("ctx").or_insert_with(|| json!("parent"));
    }
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "folder_id": r.get::<_, Option<i64>>(1)?,
        "parent_id": parent_id,
        "name": r.get::<_, String>(3)?,
        "persona_id": r.get::<_, Option<i64>>(4)?,
        "effective_persona_id": r.get::<_, Option<i64>>(5)?,
        "path": r.get::<_, Option<String>>(6)?,
        "cfg": cfg,
        "unread": r.get::<_, i64>(8)?,
        "archived": r.get::<_, i64>(9)? != 0,
        "created_at": r.get::<_, i64>(10)?,
        "updated_at": r.get::<_, i64>(11)?,
    }))
}

pub(crate) fn session(c: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    c.query_row(
        &format!("{SESSION_SELECT} WHERE s.id = ?1"),
        [id],
        session_from_row,
    )
    .optional()
}

/// All sessions and threads in ascending `id` order.
pub(crate) fn sessions(
    c: &Connection,
    folder_id: Option<i64>,
    parent_id: Option<i64>,
    archived: Option<bool>,
) -> rusqlite::Result<Vec<Value>> {
    c.prepare(&format!(
        "{SESSION_SELECT} WHERE (?1 IS NULL OR s.folder_id = ?1) AND (?2 IS NULL OR s.parent_id = ?2)
         AND (?3 IS NULL OR s.archived = ?3) ORDER BY s.id"
    ))?
    .query_map(rusqlite::params![folder_id, parent_id, archived], session_from_row)?
    .collect()
}

/// View of all sessions before and after a mutation: from the difference the core sends `session.updated` to those where something changed.
pub(crate) fn session_views(c: &Connection) -> rusqlite::Result<BTreeMap<i64, Value>> {
    Ok(sessions(c, None, None, None)?
        .into_iter()
        .filter_map(|v| Some((v["id"].as_i64()?, v)))
        .collect())
}

pub(crate) fn folder(c: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    c.query_row(
        "SELECT id, parent_id, name, persona_id, open, created_at FROM folders WHERE id = ?1",
        [id],
        folder_from_row,
    )
    .optional()
}

fn folder_from_row(r: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "parent_id": r.get::<_, Option<i64>>(1)?,
        "name": r.get::<_, String>(2)?,
        "persona_id": r.get::<_, Option<i64>>(3)?,
        "open": r.get::<_, i64>(4)? != 0,
        "created_at": r.get::<_, i64>(5)?,
    }))
}

pub(crate) fn folders(c: &Connection) -> rusqlite::Result<Vec<Value>> {
    c.prepare("SELECT id, parent_id, name, persona_id, open, created_at FROM folders ORDER BY id")?
        .query_map([], folder_from_row)?
        .collect()
}

const PERSONA_COLS: &str = "id, name, color, model_acc, style, lang, signature, mem_mode, notif_mode, work_hours, quiet_hours";

fn persona_from_row(c: &Connection, r: &Row<'_>) -> rusqlite::Result<Value> {
    let id: i64 = r.get(0)?;
    let account_ids: Vec<i64> = c
        .prepare(
            "SELECT account_id FROM persona_accounts WHERE persona_id = ?1 ORDER BY account_id",
        )?
        .query_map([id], |x| x.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(json!({
        "id": id, "name": r.get::<_, String>(1)?, "color": r.get::<_, String>(2)?,
        "model_acc": r.get::<_, Option<i64>>(3)?, "style": r.get::<_, String>(4)?,
        "lang": r.get::<_, String>(5)?, "signature": r.get::<_, String>(6)?,
        "mem_mode": r.get::<_, String>(7)?, "notif_mode": r.get::<_, String>(8)?,
        "work_hours": r.get::<_, String>(9)?, "quiet_hours": r.get::<_, String>(10)?,
        "account_ids": account_ids,
    }))
}

pub(crate) fn persona(c: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    c.query_row(
        &format!("SELECT {PERSONA_COLS} FROM personas WHERE id = ?1 AND deleted_at IS NULL"),
        [id],
        |r| persona_from_row(c, r),
    )
    .optional()
}

pub(crate) fn personas(c: &Connection) -> rusqlite::Result<Vec<Value>> {
    c.prepare(&format!(
        "SELECT {PERSONA_COLS} FROM personas WHERE deleted_at IS NULL ORDER BY id"
    ))?
    .query_map([], |r| persona_from_row(c, r))?
    .collect()
}

fn account_from_row(r: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, i64>(0)?, "type": r.get::<_, String>(1)?, "label": r.get::<_, String>(2)?,
        "status": r.get::<_, String>(3)?, "created_at": r.get::<_, i64>(4)?,
    }))
}

pub(crate) fn account(c: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    c.query_row(
        "SELECT id, type, label, status, created_at FROM accounts WHERE id = ?1 AND deleted_at IS NULL",
        [id],
        account_from_row,
    )
    .optional()
}

pub(crate) fn accounts(c: &Connection) -> rusqlite::Result<Vec<Value>> {
    c.prepare("SELECT id, type, label, status, created_at FROM accounts WHERE deleted_at IS NULL ORDER BY id")?
        .query_map([], account_from_row)?
        .collect()
}

/// Whether a live (not soft-deleted) row exists.
pub(crate) fn alive(c: &Connection, table: &str, id: i64) -> rusqlite::Result<bool> {
    let sql = match table {
        "folders" => "SELECT 1 FROM folders WHERE id = ?1",
        "personas" => "SELECT 1 FROM personas WHERE id = ?1 AND deleted_at IS NULL",
        "accounts" => "SELECT 1 FROM accounts WHERE id = ?1 AND deleted_at IS NULL",
        _ => "SELECT 1 FROM sessions WHERE id = ?1",
    };
    Ok(c.query_row(sql, [id], |_| Ok(())).optional()?.is_some())
}
