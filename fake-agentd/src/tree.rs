//! Tree of folders, sessions and threads of `fake-agentd`: reading rows, effective persona, JSON per `openapi-draft.yaml`.
//!
//! The effective persona and thread path are computed at read time and never copied (db.md section 3.1).

use std::collections::HashMap;

use rusqlite::Connection;
use serde_json::{Map, Value, json};

/// Folder depth is at most 8 (migration 0003 triggers).
pub const MAX_FOLDER_DEPTH: usize = 8;

#[derive(Debug, Clone)]
pub struct FolderRow {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub persona_id: Option<i64>,
    pub open: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub parent_id: Option<i64>,
    pub name: String,
    pub persona_id: Option<i64>,
    pub path: Option<String>,
    pub cfg_json: String,
    pub unread: i64,
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The whole tree in one read.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub folders: Vec<FolderRow>,
    pub sessions: Vec<SessionRow>,
}

pub fn load(conn: &Connection) -> rusqlite::Result<Tree> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, name, persona_id, open, created_at FROM folders ORDER BY id",
    )?;
    let folders = stmt
        .query_map([], |r| {
            Ok(FolderRow {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                name: r.get(2)?,
                persona_id: r.get(3)?,
                open: r.get::<_, i64>(4)? != 0,
                created_at: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut stmt = conn.prepare(
        "SELECT id, folder_id, parent_id, name, persona_id, path, cfg_json, unread, archived, created_at, updated_at \
         FROM sessions ORDER BY id",
    )?;
    let sessions = stmt
        .query_map([], |r| {
            Ok(SessionRow {
                id: r.get(0)?,
                folder_id: r.get(1)?,
                parent_id: r.get(2)?,
                name: r.get(3)?,
                persona_id: r.get(4)?,
                path: r.get(5)?,
                cfg_json: r.get(6)?,
                unread: r.get(7)?,
                archived: r.get::<_, i64>(8)? != 0,
                created_at: r.get(9)?,
                updated_at: r.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Tree { folders, sessions })
}

impl Tree {
    pub fn folder(&self, id: i64) -> Option<&FolderRow> {
        self.folders.iter().find(|f| f.id == id)
    }

    pub fn session(&self, id: i64) -> Option<&SessionRow> {
        self.sessions.iter().find(|s| s.id == id)
    }

    /// Folder level: the root is 1.
    pub fn folder_depth(&self, id: i64) -> usize {
        let mut depth = 0;
        let mut cur = Some(id);
        while let Some(f) = cur.and_then(|i| self.folder(i)) {
            depth += 1;
            cur = f.parent_id;
            if depth > 64 {
                break;
            }
        }
        depth
    }

    /// Height of a folder subtree: the folder itself is 1.
    pub fn folder_height(&self, id: i64) -> usize {
        1 + self
            .folders
            .iter()
            .filter(|f| f.parent_id == Some(id))
            .map(|f| self.folder_height(f.id))
            .max()
            .unwrap_or(0)
    }

    /// `ancestor` equals `id` or lies above it.
    pub fn is_ancestor_or_self(&self, ancestor: i64, id: i64) -> bool {
        let mut cur = Some(id);
        for _ in 0..64 {
            match cur {
                Some(c) if c == ancestor => return true,
                Some(c) => cur = self.folder(c).and_then(|f| f.parent_id),
                None => return false,
            }
        }
        false
    }

    /// Persona at read time: its own, the parent's persona (for a thread), then the nearest folder upward.
    pub fn effective_persona(&self, s: &SessionRow) -> Option<i64> {
        s.persona_id
            .or_else(|| {
                s.parent_id
                    .and_then(|p| self.session(p))
                    .and_then(|p| p.persona_id)
            })
            .or_else(|| {
                let mut cur = s.folder_id;
                for _ in 0..64 {
                    let f = self.folder(cur?)?;
                    if f.persona_id.is_some() {
                        return f.persona_id;
                    }
                    cur = f.parent_id;
                }
                None
            })
    }

    pub fn folder_json(&self, f: &FolderRow) -> Value {
        json!({
            "id": f.id, "parent_id": f.parent_id, "name": f.name, "persona_id": f.persona_id,
            "open": f.open, "created_at": f.created_at,
        })
    }

    /// Session per the `Session` schema; a thread takes its path and context from the parent if it has none of its own.
    pub fn session_json(&self, s: &SessionRow) -> Value {
        let parent = s.parent_id.and_then(|p| self.session(p));
        let mut cfg: Map<String, Value> = serde_json::from_str(&s.cfg_json).unwrap_or_default();
        if parent.is_some() {
            cfg.entry("ctx").or_insert_with(|| json!("parent"));
        }
        let path = s
            .path
            .clone()
            .or_else(|| parent.and_then(|p| p.path.clone()));
        json!({
            "id": s.id, "folder_id": s.folder_id, "parent_id": s.parent_id, "name": s.name,
            "persona_id": s.persona_id, "effective_persona_id": self.effective_persona(s),
            "path": path, "cfg": cfg, "unread": s.unread, "archived": s.archived,
            "created_at": s.created_at, "updated_at": s.updated_at,
        })
    }

    pub fn folders_json(&self) -> Vec<Value> {
        self.folders.iter().map(|f| self.folder_json(f)).collect()
    }

    pub fn sessions_json(&self) -> Vec<Value> {
        self.sessions
            .iter()
            .filter(|s| !s.archived)
            .map(|s| self.session_json(s))
            .collect()
    }

    /// State of the `sessions` topic (api.md section 4.3): the snapshot and what a new subscriber gets.
    pub fn snapshot(&self) -> Value {
        json!({ "folders": self.folders_json(), "sessions": self.sessions_json() })
    }

    pub fn session_map(&self) -> HashMap<i64, Value> {
        self.sessions
            .iter()
            .filter(|s| !s.archived)
            .map(|s| (s.id, self.session_json(s)))
            .collect()
    }

    pub fn folder_map(&self) -> HashMap<i64, Value> {
        self.folders
            .iter()
            .map(|f| (f.id, self.folder_json(f)))
            .collect()
    }
}

/// Session persona at read time: its own, then the nearest folder upward.
pub fn effective_persona_id(conn: &Connection, session_id: i64) -> rusqlite::Result<Option<i64>> {
    let tree = load(conn)?;
    Ok(tree
        .session(session_id)
        .and_then(|s| tree.effective_persona(s)))
}

/// Personas for `GET /personas` (soft-deleted ones are hidden) with accounts from `persona_accounts`.
pub fn personas_json(conn: &Connection) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, color, model_acc, style, lang, signature, mem_mode, notif_mode, work_hours, quiet_hours \
         FROM personas WHERE deleted_at IS NULL ORDER BY id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                json!({
                    "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "color": r.get::<_, String>(2)?,
                    "model_acc": r.get::<_, Option<i64>>(3)?, "style": r.get::<_, String>(4)?,
                    "lang": r.get::<_, String>(5)?, "signature": r.get::<_, String>(6)?,
                    "mem_mode": r.get::<_, String>(7)?, "notif_mode": r.get::<_, String>(8)?,
                    "work_hours": r.get::<_, String>(9)?, "quiet_hours": r.get::<_, String>(10)?,
                }),
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut acc = conn.prepare(
        "SELECT account_id FROM persona_accounts WHERE persona_id = ?1 ORDER BY account_id",
    )?;
    let mut out = Vec::new();
    for (id, mut p) in rows {
        let ids: Vec<i64> = acc
            .query_map([id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        p["account_ids"] = json!(ids);
        out.push(p);
    }
    Ok(out)
}
