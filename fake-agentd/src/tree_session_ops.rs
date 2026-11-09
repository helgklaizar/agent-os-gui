//! Session and thread mutations: creation, edit, cascading delete, reset and increment of unread.

use rusqlite::params;
use serde_json::{Map, Value};

use crate::state::Shared;
use crate::tree_ops::{Res, SessionPatch, TreeError, check_folder, check_persona, name_of, run};

pub fn create_session(
    shared: &Shared,
    name: &str,
    folder: Option<i64>,
    persona: Option<i64>,
    path: Option<String>,
) -> Res<i64> {
    let name = name_of(name)?;
    run(shared, |c, t, now| {
        check_folder(t, folder)?;
        check_persona(c, persona)?;
        c.execute(
            "INSERT INTO sessions(folder_id, name, persona_id, path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![folder, name, persona, path, now],
        )?;
        Ok(c.last_insert_rowid())
    })
}

/// A thread inherits the folder, persona, path and context of its parent at read time; the row stores only its own.
pub fn create_thread(shared: &Shared, parent: i64, name: &str) -> Res<i64> {
    let name = name_of(name)?;
    run(shared, |c, t, now| {
        let p = t.session(parent).ok_or(TreeError::NotFound)?;
        if p.parent_id.is_some() {
            return Err(TreeError::Invalid(
                "parent_id",
                "A thread cannot create a thread",
            ));
        }
        c.execute(
            "INSERT INTO sessions(folder_id, parent_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![p.folder_id, parent, name, now],
        )?;
        Ok(c.last_insert_rowid())
    })
}

fn valid_cfg(cfg: &Map<String, Value>) -> Res<()> {
    let ok = cfg
        .get("ctx")
        .is_none_or(|v| matches!(v.as_str(), Some("parent" | "none")))
        && cfg.get("auto").is_none_or(Value::is_boolean)
        && cfg.get("mem").is_none_or(Value::is_boolean)
        && cfg
            .get("notif")
            .is_none_or(|v| matches!(v.as_str(), Some("all" | "important" | "off")))
        && cfg
            .get("model")
            .is_none_or(|v| v.is_null() || v.as_i64().is_some());
    if ok {
        Ok(())
    } else {
        Err(TreeError::Invalid(
            "cfg",
            "cfg: ctx parent or none, mem and auto are booleans, notif all, important or off, model is account_id or null",
        ))
    }
}

pub fn patch_session(shared: &Shared, id: i64, p: SessionPatch) -> Res<()> {
    let name = p.name.as_deref().map(name_of).transpose()?;
    if let Some(cfg) = &p.cfg {
        valid_cfg(cfg)?;
    }
    run(shared, |c, t, now| {
        let cur = t.session(id).ok_or(TreeError::NotFound)?;
        if let Some(f) = p.folder {
            if cur.parent_id.is_some() {
                return Err(TreeError::Invalid(
                    "folder_id",
                    "A thread follows its parent",
                ));
            }
            check_folder(t, f)?;
        }
        if let Some(pe) = p.persona {
            check_persona(c, pe)?;
        }
        let mut cfg: Map<String, Value> = serde_json::from_str(&cur.cfg_json).unwrap_or_default();
        cfg.extend(p.cfg.clone().unwrap_or_default());
        let folder = p.folder.unwrap_or(cur.folder_id);
        c.execute(
            "UPDATE sessions SET name = ?1, folder_id = ?2, persona_id = ?3, path = ?4, cfg_json = ?5, archived = ?6, updated_at = ?7 WHERE id = ?8",
            params![
                name.clone().unwrap_or_else(|| cur.name.clone()),
                folder,
                p.persona.unwrap_or(cur.persona_id),
                p.path.clone().unwrap_or_else(|| cur.path.clone()),
                Value::Object(cfg).to_string(),
                i64::from(p.archived.unwrap_or(cur.archived)),
                now,
                id
            ],
        )?;
        if p.folder.is_some() {
            c.execute(
                "UPDATE sessions SET folder_id = ?1 WHERE parent_id = ?2",
                params![folder, id],
            )?;
        }
        Ok(())
    })
}

/// Threads go with it by cascade (`sessions.parent_id ON DELETE CASCADE`).
pub fn delete_session(shared: &Shared, id: i64) -> Res<()> {
    run(shared, |c, t, _| {
        t.session(id).ok_or(TreeError::NotFound)?;
        // browser bookmarks live in `settings` without a foreign key: cleaned together with the session and its threads
        let mut gone = vec![id];
        let mut st = c.prepare("SELECT id FROM sessions WHERE parent_id = ?1")?;
        gone.extend(
            st.query_map(params![id], |r| r.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
        drop(st);
        for s in gone {
            c.execute(
                "DELETE FROM settings WHERE key = ?1",
                params![format!("browser.bookmarks.{s}")],
            )?;
        }
        c.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(())
    })
}

/// `POST /sessions/{id}/read`: reset the counter.
pub fn mark_read(shared: &Shared, id: i64) -> Res<()> {
    run(shared, |c, t, _| {
        t.session(id).ok_or(TreeError::NotFound)?;
        c.execute("UPDATE sessions SET unread = 0 WHERE id = ?1", params![id])?;
        Ok(())
    })
}

/// Test feed: the session gained unread (in the real core a message arrival does this).
pub fn add_unread(shared: &Shared, id: i64, n: i64) -> Res<()> {
    run(shared, |c, t, _| {
        t.session(id).ok_or(TreeError::NotFound)?;
        c.execute(
            "UPDATE sessions SET unread = unread + ?1 WHERE id = ?2",
            params![n, id],
        )?;
        Ok(())
    })
}
