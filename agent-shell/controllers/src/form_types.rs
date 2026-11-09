//! Requests to open tree forms: what exactly to show in the dialog. Built by `SessionTreeController`,
//! which knows the tree and personas; `DialogController` knows nothing about the tree and shows what the request carries.

use crate::tree_types::NodeId;

/// Folder list row: depth sets the indent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderChoice {
    /// `None` is the "No folder" item.
    pub id: Option<i64>,
    pub name: String,
    pub depth: usize,
}

/// Persona list row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaChoice {
    /// `None` means "as the folder / default".
    pub id: Option<i64>,
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormRequest {
    NewSession {
        /// Folder the form was opened from.
        folder: Option<i64>,
        folders: Vec<FolderChoice>,
        personas: Vec<PersonaChoice>,
    },
    NewFolder {
        parent: Option<i64>,
        folders: Vec<FolderChoice>,
        personas: Vec<PersonaChoice>,
    },
    /// Thread: title only; persona, path and context are inherited from the parent.
    NewThread {
        parent_id: i64,
        parent_name: String,
    },
    /// "Move": only the folder changes.
    MoveSession {
        id: i64,
        name: String,
        folder: Option<i64>,
        folders: Vec<FolderChoice>,
    },
    /// "Rename": only the name changes.
    Rename {
        node: NodeId,
        name: String,
    },
    DeleteSession {
        id: i64,
        name: String,
        threads: usize,
    },
    DeleteFolder {
        id: i64,
        name: String,
    },
}
