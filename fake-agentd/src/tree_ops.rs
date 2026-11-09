//! Tree mutations: each runs in one transaction of the single writer; `sessions` frames are built
//! by comparing the tree before and after, so the client also gets indirect changes (inherited persona and path).

use rusqlite::{Connection, params};
use serde_json::{Map, Value};

use crate::FrameType;
use crate::state::Shared;
use crate::tree::{self, MAX_FOLDER_DEPTH, Tree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    NotFound,
    /// `422 validation_failed`: field and reason.
    Invalid(&'static str, &'static str),
    /// `409 conflict`.
    Conflict(&'static str),
    /// Storage failure (the response is `500 internal` without details).
    Db(String),
}

impl From<rusqlite::Error> for TreeError {
    fn from(e: rusqlite::Error) -> Self {
        TreeError::Db(e.to_string())
    }
}

pub(crate) type Res<T> = Result<T, TreeError>;

/// What to change in a folder: `Some(None)` in `parent` and `persona` is an explicit `null`.
#[derive(Debug, Default, Clone)]
pub struct FolderPatch {
    pub name: Option<String>,
    pub parent: Option<Option<i64>>,
    pub persona: Option<Option<i64>>,
    pub open: Option<bool>,
}

#[derive(Debug, Default, Clone)]
pub struct SessionPatch {
    pub name: Option<String>,
    pub folder: Option<Option<i64>>,
    pub persona: Option<Option<i64>>,
    pub path: Option<Option<String>>,
    pub cfg: Option<Map<String, Value>>,
    pub archived: Option<bool>,
}

/// Transaction + frames. An operation error rolls everything back and sends no frames.
pub(crate) fn run<T>(
    shared: &Shared,
    op: impl FnOnce(&Connection, &Tree, i64) -> Res<T>,
) -> Res<T> {
    let now = shared.clock.now_unix();
    shared
        .db
        .write(|w| {
            let tx = w.transaction()?;
            let before = tree::load(&tx)?;
            let result = match op(&tx, &before, now) {
                Ok(v) => v,
                Err(e) => return Ok(Err(e)),
            };
            let after = tree::load(&tx)?;
            tx.commit()?;
            crate::tree_pub::publish_diff(shared, &before, &after);
            Ok(Ok(result))
        })
        .map_err(|e| TreeError::Db(e.to_string()))?
}

pub(crate) fn name_of(raw: &str) -> Res<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(TreeError::Invalid("name", "Name must not be empty"));
    }
    Ok(name.to_string())
}

pub(crate) fn persona_exists(conn: &Connection, id: i64) -> Res<()> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM personas WHERE id = ?1 AND deleted_at IS NULL",
        params![id],
        |r| r.get(0),
    )?;
    if n == 0 {
        return Err(TreeError::Invalid("persona_id", "Persona not found"));
    }
    Ok(())
}

pub(crate) fn check_persona(conn: &Connection, persona: Option<i64>) -> Res<()> {
    persona.map_or(Ok(()), |p| persona_exists(conn, p))
}

pub(crate) fn check_folder(t: &Tree, folder: Option<i64>) -> Res<()> {
    match folder {
        Some(f) if t.folder(f).is_none() => {
            Err(TreeError::Invalid("folder_id", "Folder not found"))
        }
        _ => Ok(()),
    }
}

pub fn create_folder(
    shared: &Shared,
    name: &str,
    parent: Option<i64>,
    persona: Option<i64>,
) -> Res<i64> {
    let name = name_of(name)?;
    run(shared, |c, t, now| {
        if let Some(p) = parent {
            if t.folder(p).is_none() {
                return Err(TreeError::Invalid("parent_id", "Parent folder not found"));
            }
            if t.folder_depth(p) + 1 > MAX_FOLDER_DEPTH {
                return Err(TreeError::Invalid("parent_id", "Folder depth is at most 8"));
            }
        }
        check_persona(c, persona)?;
        c.execute(
            "INSERT INTO folders(parent_id, name, persona_id, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![parent, name, persona, now],
        )?;
        Ok(c.last_insert_rowid())
    })
}

pub fn patch_folder(shared: &Shared, id: i64, p: FolderPatch) -> Res<()> {
    let name = p.name.as_deref().map(name_of).transpose()?;
    run(shared, |c, t, _| {
        let cur = t.folder(id).ok_or(TreeError::NotFound)?;
        if let Some(Some(np)) = p.parent {
            if t.folder(np).is_none() {
                return Err(TreeError::Invalid("parent_id", "Parent folder not found"));
            }
            if t.is_ancestor_or_self(id, np) {
                return Err(TreeError::Conflict(
                    "A folder cannot be moved into itself or a descendant",
                ));
            }
            if t.folder_depth(np) + t.folder_height(id) > MAX_FOLDER_DEPTH {
                return Err(TreeError::Invalid("parent_id", "Folder depth is at most 8"));
            }
        }
        if let Some(pe) = p.persona {
            check_persona(c, pe)?;
        }
        c.execute(
            "UPDATE folders SET name = ?1, parent_id = ?2, persona_id = ?3, open = ?4 WHERE id = ?5",
            params![
                name.clone().unwrap_or_else(|| cur.name.clone()),
                p.parent.unwrap_or(cur.parent_id),
                p.persona.unwrap_or(cur.persona_id),
                i64::from(p.open.unwrap_or(cur.open)),
                id
            ],
        )?;
        Ok(())
    })
}

/// Folder deletion (db.md section 5): children move one level up, then the row is deleted.
pub fn delete_folder(shared: &Shared, id: i64) -> Res<()> {
    run(shared, |c, t, _| {
        let up = t.folder(id).ok_or(TreeError::NotFound)?.parent_id;
        c.execute(
            "UPDATE folders SET parent_id = ?1 WHERE parent_id = ?2",
            params![up, id],
        )?;
        c.execute(
            "UPDATE sessions SET folder_id = ?1 WHERE folder_id = ?2",
            params![up, id],
        )?;
        c.execute("DELETE FROM folders WHERE id = ?1", params![id])?;
        Ok(())
    })
}

/// Fixtures and the first state of the `sessions` topic at startup.
pub fn start(shared: &Shared, db: &crate::db::Db) -> Result<(), crate::db::DbError> {
    let now = shared.clock.now_unix();
    db.write(|w| crate::tree_seed::seed_rows(w, now))?;
    let conn = db.reader()?;
    let t = tree::load(&conn)?;
    shared
        .hub
        .emit("sessions", FrameType::Snapshot, t.snapshot());
    Ok(())
}
