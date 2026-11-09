//! Session tree model (S3-03, S3-05): folders, sessions, personas and everything computed on read -
//! effective persona, thread path and context, order of visible rows. A persona is never copied.

use std::collections::{HashMap, HashSet};

use crate::form_types::{FolderChoice, PersonaChoice};
use crate::i18n::tr;
use crate::tree_types::{FolderItem, NodeId, PersonaItem, SessionItem};

/// Name length limit in a tree row, characters; the rest is truncated by the view.
pub const NAME_LIMIT: usize = 120;
/// Indent per nesting level, pixels.
pub const INDENT_STEP: u32 = 16;

/// Display name: a long one is truncated with "…", markup is neither interpreted nor escaped.
pub fn shorten(name: &str) -> String {
    let mut chars = name.chars();
    let head: String = chars.by_ref().take(NAME_LIMIT).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

#[derive(Debug, Default)]
pub struct TreeModel {
    pub folders: Vec<FolderItem>,
    pub sessions: Vec<SessionItem>,
    pub personas: Vec<PersonaItem>,
    /// Folder expansion state is kept by the client; it does not go to the core.
    pub expanded: HashMap<i64, bool>,
}

impl TreeModel {
    pub fn folder(&self, id: i64) -> Option<&FolderItem> {
        self.folders.iter().find(|f| f.id == id)
    }

    pub fn session(&self, id: i64) -> Option<&SessionItem> {
        self.sessions.iter().find(|s| s.id == id)
    }

    pub fn is_expanded(&self, folder: i64) -> bool {
        self.expanded.get(&folder).copied().unwrap_or(true)
    }

    /// Thread parent, if it is in the model.
    pub fn parent_of(&self, s: &SessionItem) -> Option<&SessionItem> {
        s.parent_id.and_then(|p| self.session(p))
    }

    /// Persona on read: own, the parent's (for a thread), the nearest folder's upward.
    pub fn effective_persona(&self, s: &SessionItem) -> Option<i64> {
        s.persona_id
            .or_else(|| self.parent_of(s).and_then(|p| p.persona_id))
            .or_else(|| self.folder_persona(s.folder_id.or_else(|| self.parent_of(s)?.folder_id)))
    }

    pub fn folder_persona(&self, folder: Option<i64>) -> Option<i64> {
        let mut cur = folder;
        for _ in 0..64 {
            let f = self.folder(cur?)?;
            if f.persona_id.is_some() {
                return f.persona_id;
            }
            cur = f.parent_id;
        }
        None
    }

    /// Thread path is the parent's on read; a session has its own.
    pub fn effective_path(&self, s: &SessionItem) -> Option<String> {
        self.parent_of(s)
            .and_then(|p| p.path.clone())
            .or_else(|| s.path.clone())
    }

    pub fn color_of(&self, persona: Option<i64>) -> Option<String> {
        let id = persona?;
        self.personas
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.color.clone())
    }

    /// A folder is a root if it has no parent or the parent has not arrived yet.
    fn folder_parent(&self, f: &FolderItem) -> Option<i64> {
        f.parent_id.filter(|p| self.folder(*p).is_some())
    }

    /// Top-level session (not a thread, or a thread whose parent is not in the model).
    fn is_top(&self, s: &SessionItem) -> bool {
        self.parent_of(s).is_none()
    }

    fn session_folder(&self, s: &SessionItem) -> Option<i64> {
        s.folder_id.filter(|f| self.folder(*f).is_some())
    }

    fn sorted_ids<T>(items: impl Iterator<Item = T>, id: impl Fn(&T) -> i64) -> Vec<T> {
        let mut v: Vec<T> = items.collect();
        v.sort_by_key(&id);
        v
    }

    /// Direct children of a folder (`None` - root): subfolders, then sessions, a session's threads after it. Node key and depth.
    pub fn visible(&self, collapsed_ok: bool) -> Vec<(NodeId, usize)> {
        let mut out = Vec::new();
        self.walk(None, 0, collapsed_ok, &mut out);
        out
    }

    fn walk(
        &self,
        parent: Option<i64>,
        depth: usize,
        only_open: bool,
        out: &mut Vec<(NodeId, usize)>,
    ) {
        let folders = Self::sorted_ids(
            self.folders
                .iter()
                .filter(|f| self.folder_parent(f) == parent),
            |f| f.id,
        );
        for f in folders {
            out.push((NodeId::Folder(f.id), depth));
            if !only_open || self.is_expanded(f.id) {
                self.walk(Some(f.id), depth + 1, only_open, out);
            }
        }
        let sessions = Self::sorted_ids(
            self.sessions
                .iter()
                .filter(|s| self.is_top(s) && self.session_folder(s) == parent),
            |s| s.id,
        );
        for s in sessions {
            out.push((NodeId::Session(s.id), depth));
            let threads = Self::sorted_ids(
                self.sessions.iter().filter(|t| t.parent_id == Some(s.id)),
                |t| t.id,
            );
            out.extend(threads.iter().map(|t| (NodeId::Session(t.id), depth + 1)));
        }
    }

    /// Node parent in the tree: folder, session (for a thread) or `None` at the root.
    pub fn parent_node(&self, node: NodeId) -> Option<NodeId> {
        match node {
            NodeId::Folder(id) => self
                .folder(id)
                .and_then(|f| self.folder_parent(f))
                .map(NodeId::Folder),
            NodeId::Session(id) => {
                let s = self.session(id)?;
                match self.parent_of(s) {
                    Some(p) => Some(NodeId::Session(p.id)),
                    None => self.session_folder(s).map(NodeId::Folder),
                }
            }
        }
    }

    pub fn threads_of(&self, session: i64) -> usize {
        self.sessions
            .iter()
            .filter(|s| s.parent_id == Some(session))
            .count()
    }

    /// All thread descendants of a session and the session itself: what goes away in a cascade.
    pub fn with_threads(&self, session: i64) -> HashSet<i64> {
        let mut ids: HashSet<i64> = self
            .sessions
            .iter()
            .filter(|s| s.parent_id == Some(session))
            .map(|s| s.id)
            .collect();
        ids.insert(session);
        ids
    }

    /// Folder list for forms: "No folder", then the tree with depth.
    pub fn folder_choices(&self) -> Vec<FolderChoice> {
        let mut choices = vec![FolderChoice {
            id: None,
            name: tr("dialog-no-folder"),
            depth: 0,
        }];
        let mut all = Vec::new();
        self.walk(None, 0, false, &mut all);
        for (node, depth) in all {
            if let NodeId::Folder(id) = node
                && let Some(f) = self.folder(id)
            {
                choices.push(FolderChoice {
                    id: Some(id),
                    name: shorten(&f.name),
                    depth,
                });
            }
        }
        choices
    }

    pub fn persona_choices(&self) -> Vec<PersonaChoice> {
        let mut choices = vec![PersonaChoice {
            id: None,
            name: tr("dialog-persona-inherit"),
            color: None,
        }];
        choices.extend(self.personas.iter().map(|p| PersonaChoice {
            id: Some(p.id),
            name: shorten(&p.name),
            color: Some(p.color.clone()),
        }));
        choices
    }
}
