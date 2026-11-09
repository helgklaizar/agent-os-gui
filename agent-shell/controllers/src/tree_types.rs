//! Session tree types as the controllers see them (api.md sections 4.3, 5.1): folders, sessions, personas, `sessions` frames.
//! Unknown fields are ignored, missing ones get defaults.

use serde::Deserialize;
use serde_json::Value;

/// Tree node. Folder and session ids come from different tables and may coincide, so the key carries the kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeId {
    Folder(i64),
    Session(i64),
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct FolderItem {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub persona_id: Option<i64>,
    pub open: bool,
}

/// Session settings the tree needs.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct SessionCfg {
    /// `parent` | `none`: thread parent context.
    pub ctx: Option<String>,
    /// Session model `account_id`; `None` - as persona.
    pub model: Option<i64>,
    /// Session memory.
    pub mem: bool,
    /// Autonomous mode.
    pub auto: bool,
    /// `all` | `important` | `off`; `None` - the default (`all`).
    pub notif: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct SessionItem {
    pub id: i64,
    pub folder_id: Option<i64>,
    /// Not `None` - a thread.
    pub parent_id: Option<i64>,
    pub name: String,
    pub persona_id: Option<i64>,
    pub path: Option<String>,
    pub cfg: SessionCfg,
    pub unread: i64,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct PersonaItem {
    pub id: i64,
    pub name: String,
    pub color: String,
}

/// Feed event at the scope the tree needs (the full feed is sprint 5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct FeedEvent {
    pub id: i64,
    pub text: String,
    pub session_id: Option<i64>,
}

/// `sessions` topic delta.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeFrame {
    FolderCreated(FolderItem),
    FolderUpdated(FolderItem),
    FolderDeleted(i64),
    SessionCreated(SessionItem),
    SessionUpdated(SessionItem),
    SessionDeleted(i64),
    Unread { session_id: i64, unread: i64 },
}

/// `sessions` snapshot: `{ folders, sessions }`; a frame of the wrong shape - `None`.
pub fn parse_snapshot(data: &Value) -> Option<(Vec<FolderItem>, Vec<SessionItem>)> {
    let list = |key: &str| data.get(key)?.as_array().cloned();
    let folders = list("folders")?
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()
        .ok()?;
    let sessions = list("sessions")?
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()
        .ok()?;
    Some((folders, sessions))
}

/// `sessions` delta; an unknown `op` - `None` (the client skips such frames, api.md section 2).
pub fn parse_delta(data: &Value) -> Option<TreeFrame> {
    let op = data.get("op")?.as_str()?;
    let id_of = |key: &str| data.get(key)?.get("id")?.as_i64();
    let folder = || serde_json::from_value(data.get("folder")?.clone()).ok();
    let session = || serde_json::from_value(data.get("session")?.clone()).ok();
    Some(match op {
        "folder.created" => TreeFrame::FolderCreated(folder()?),
        "folder.updated" => TreeFrame::FolderUpdated(folder()?),
        "folder.deleted" => TreeFrame::FolderDeleted(id_of("folder")?),
        "session.created" => TreeFrame::SessionCreated(session()?),
        "session.updated" => TreeFrame::SessionUpdated(session()?),
        "session.deleted" => TreeFrame::SessionDeleted(id_of("session")?),
        "unread" => TreeFrame::Unread {
            session_id: data.get("session_id")?.as_i64()?,
            unread: data.get("unread")?.as_i64()?,
        },
        _ => return None,
    })
}

/// `GET /v1/personas`: persona array.
pub fn parse_personas(data: &Value) -> Vec<PersonaItem> {
    data.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| serde_json::from_value(p.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}
