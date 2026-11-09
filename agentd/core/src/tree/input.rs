//! Validated input data of tree operations. `Option<Option<T>>`: outer `None` — the field was not sent,
//! inner — an explicit `null`.

use serde_json::{Map, Value};

/// Name length limit.
pub const MAX_NAME_CHARS: usize = 200;
/// Session path length limit, in characters.
pub const MAX_PATH_CHARS: usize = 4096;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct FolderInput {
    pub name: Option<String>,
    pub parent_id: Option<Option<i64>>,
    pub persona_id: Option<Option<i64>>,
    pub open: Option<bool>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct SessionInput {
    pub name: Option<String>,
    pub folder_id: Option<Option<i64>>,
    pub persona_id: Option<Option<i64>>,
    pub path: Option<Option<String>>,
    /// Merge by keys: `null` removes a key. The shape is already validated; existence of `model` is checked by the store.
    pub cfg: Option<Map<String, Value>>,
    pub archived: Option<bool>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct PersonaInput {
    pub name: Option<String>,
    pub color: Option<String>,
    pub model_acc: Option<Option<i64>>,
    pub style: Option<String>,
    pub lang: Option<String>,
    pub signature: Option<String>,
    pub mem_mode: Option<String>,
    pub notif_mode: Option<String>,
    pub work_hours: Option<String>,
    pub quiet_hours: Option<String>,
    pub account_ids: Option<Vec<i64>>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct AccountInput {
    pub kind: Option<String>,
    pub label: Option<String>,
}

/// `GET /sessions` filters; without filters — all rows (sessions and threads, including archived).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionFilter {
    pub folder_id: Option<i64>,
    pub parent_id: Option<i64>,
    pub archived: Option<bool>,
}
