//! `SessionTreeController`: tree of folders, sessions and threads. The selection is keyed by session id, folder expansion is
//! client state; a thread's persona, path and context are computed on read (`tree_model`).

use crate::time::Instant;
use std::collections::HashSet;

use crate::bus::BusEvent;
use crate::controller::{Controller, CoreCommand, Effect};
use crate::i18n::tr;
use crate::tree_forms;
use crate::tree_model::TreeModel;
use crate::tree_nav::{Nav, navigate};
use crate::tree_render::Render;
use crate::tree_types::{FeedEvent, FolderItem, NodeId, PersonaItem, SessionItem, TreeFrame};
use crate::tree_view::{NavKey, SessionInfo, TreeView};
use crate::types::Tab;

/// Tree form request: the button at the bottom of the column, a row action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeRequest {
    /// "New session" button; `folder` is the preselected folder.
    NewSession {
        folder: Option<i64>,
    },
    /// "New folder" button; `parent` is the preselected parent.
    NewFolder {
        parent: Option<i64>,
    },
    /// Folder row action "New session".
    NewSessionInFolder {
        folder: i64,
    },
    /// Folder row action "Subfolder".
    NewSubfolder {
        parent: i64,
    },
    NewThread {
        parent: i64,
    },
    Move {
        session: i64,
    },
    Rename {
        node: NodeId,
    },
    Delete {
        node: NodeId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TreeMsg {
    Snapshot {
        folders: Vec<FolderItem>,
        sessions: Vec<SessionItem>,
    },
    Frame(TreeFrame),
    Personas(Vec<PersonaItem>),
    Feed(Vec<FeedEvent>),
    /// Open a session (click, Enter).
    Select(i64),
    /// Session opened from elsewhere (a feed card): select it and clear unread, publishing nothing.
    Opened(i64),
    Focus(NodeId),
    ToggleFolder(i64),
    Key(NavKey),
    /// Column collapsed or expanded.
    Collapsed(bool),
    Request(TreeRequest),
}

#[derive(Default)]
pub struct SessionTreeController {
    model: TreeModel,
    selected: Option<i64>,
    focus: Option<NodeId>,
    collapsed: bool,
    feed: Vec<FeedEvent>,
}

fn open_session(id: i64) -> Effect {
    Effect::Publish(BusEvent::OpenSession {
        session_id: id,
        tab: Tab::Chat,
    })
}

impl SessionTreeController {
    pub fn new() -> Self {
        Self::default()
    }

    fn rows_nodes(&self) -> Vec<NodeId> {
        self.model
            .visible(true)
            .into_iter()
            .map(|(n, _)| n)
            .collect()
    }

    fn visible_sessions(&self) -> Vec<i64> {
        self.rows_nodes()
            .into_iter()
            .filter_map(|n| match n {
                NodeId::Session(id) => Some(id),
                NodeId::Folder(_) => None,
            })
            .collect()
    }

    /// What the tree knows about a session: inherited values, computed on read.
    pub fn session_info(&self, id: i64) -> Option<SessionInfo> {
        let s = self.model.session(id)?;
        let thread = self.model.parent_of(s).is_some();
        let from_parent = thread && s.cfg.ctx.as_deref() != Some("none");
        Some(SessionInfo {
            effective_persona_id: self.model.effective_persona(s),
            path: self.model.effective_path(s),
            context_from_parent: from_parent,
            context_label: (thread && !from_parent).then(|| tr("tree-ctx-isolated")),
        })
    }

    fn select(&mut self, id: i64, fx: &mut Vec<Effect>) {
        let Some(s) = self.model.sessions.iter_mut().find(|s| s.id == id) else {
            return;
        };
        self.selected = Some(id);
        self.focus = Some(NodeId::Session(id));
        let had_unread = s.unread > 0;
        s.unread = 0;
        fx.push(open_session(id));
        if had_unread {
            fx.push(Effect::Command(CoreCommand::MarkRead { id }));
        }
    }

    /// Removes sessions and threads; the selection moves to the neighbour in order before deletion; feed events remain without `session_id`.
    fn remove_sessions(&mut self, ids: &HashSet<i64>, fx: &mut Vec<Effect>) {
        let before = self.visible_sessions();
        self.model.sessions.retain(|s| !ids.contains(&s.id));
        for e in &mut self.feed {
            if e.session_id.is_some_and(|s| ids.contains(&s)) {
                e.session_id = None;
            }
        }
        let Some(sel) = self.selected.filter(|s| ids.contains(s)) else {
            return;
        };
        let pos = before.iter().position(|s| *s == sel);
        let alive = |s: &&i64| !ids.contains(*s);
        let pick = match pos {
            Some(p) => before[p + 1..]
                .iter()
                .find(alive)
                .or_else(|| before[..p].iter().rev().find(alive)),
            None => before.iter().find(alive),
        };
        self.selected = None;
        if let Some(next) = pick.copied() {
            self.select(next, fx);
        }
    }

    fn apply_frame(&mut self, frame: TreeFrame, fx: &mut Vec<Effect>) {
        match frame {
            TreeFrame::FolderCreated(f) | TreeFrame::FolderUpdated(f) => {
                self.model.expanded.entry(f.id).or_insert(f.open);
                match self.model.folders.iter_mut().find(|x| x.id == f.id) {
                    Some(old) => *old = f,
                    None => self.model.folders.push(f),
                }
            }
            TreeFrame::FolderDeleted(id) => {
                self.model.folders.retain(|f| f.id != id);
                self.model.expanded.remove(&id);
            }
            TreeFrame::SessionCreated(s) | TreeFrame::SessionUpdated(s) => {
                let mut s = s;
                if self.selected == Some(s.id) {
                    s.unread = 0;
                }
                if let Some(old) = self.model.sessions.iter_mut().find(|x| x.id == s.id) {
                    *old = s;
                } else {
                    self.model.sessions.push(s);
                }
            }
            TreeFrame::SessionDeleted(id) => {
                let ids = self.model.with_threads(id);
                if self.model.session(id).is_some() {
                    self.remove_sessions(&ids, fx);
                }
            }
            TreeFrame::Unread { session_id, unread } => {
                let open = self.selected == Some(session_id);
                if let Some(s) = self.model.sessions.iter_mut().find(|s| s.id == session_id) {
                    s.unread = if open { 0 } else { unread.max(0) };
                    if open && unread > 0 {
                        fx.push(Effect::Command(CoreCommand::MarkRead { id: session_id }));
                    }
                }
            }
        }
    }

    fn toggle(&mut self, folder: i64) {
        if self.model.folder(folder).is_some() {
            let open = self.model.is_expanded(folder);
            self.model.expanded.insert(folder, !open);
        }
    }

    fn key(&mut self, key: NavKey, fx: &mut Vec<Effect>) {
        let nodes = self.rows_nodes();
        match navigate(&self.model, &nodes, self.focus, key) {
            Nav::Focus(node) => self.focus = node,
            Nav::Toggle(id) => self.toggle(id),
            Nav::Open(id) => self.select(id, fx),
        }
    }

    /// Focus must not point at a vanished row: it moves to the row with the same index.
    fn keep_focus(&mut self, before: &[NodeId]) {
        let after = self.rows_nodes();
        let Some(focus) = self.focus.filter(|f| !after.contains(f)) else {
            return;
        };
        let was = before.iter().position(|n| *n == focus).unwrap_or(0);
        self.focus = after.get(was.min(after.len().saturating_sub(1))).copied();
    }
}

impl Controller for SessionTreeController {
    type Msg = TreeMsg;
    type View = TreeView;

    fn update(&mut self, msg: TreeMsg, _now: Instant) -> Vec<Effect> {
        let before = self.rows_nodes();
        let mut fx = Vec::new();
        match msg {
            TreeMsg::Snapshot { folders, sessions } => {
                for f in &folders {
                    self.model.expanded.entry(f.id).or_insert(f.open);
                }
                self.model
                    .expanded
                    .retain(|id, _| folders.iter().any(|f| f.id == *id));
                self.model.folders = folders;
                self.model.sessions = sessions;
                if self
                    .selected
                    .is_some_and(|s| self.model.session(s).is_none())
                {
                    self.selected = None;
                }
            }
            TreeMsg::Frame(frame) => self.apply_frame(frame, &mut fx),
            TreeMsg::Personas(p) => self.model.personas = p,
            TreeMsg::Feed(events) => self.feed = events,
            TreeMsg::Select(id) => self.select(id, &mut fx),
            TreeMsg::Opened(id) => {
                let had_unread = self.model.session(id).map(|s| s.unread > 0);
                if let Some(had_unread) = had_unread {
                    self.selected = Some(id);
                    self.focus = Some(NodeId::Session(id));
                    if let Some(s) = self.model.sessions.iter_mut().find(|s| s.id == id) {
                        s.unread = 0;
                    }
                    if had_unread {
                        fx.push(Effect::Command(CoreCommand::MarkRead { id }));
                    }
                }
            }
            TreeMsg::Focus(node) => self.focus = Some(node),
            TreeMsg::ToggleFolder(id) => self.toggle(id),
            TreeMsg::Key(key) => self.key(key, &mut fx),
            TreeMsg::Collapsed(c) => self.collapsed = c,
            TreeMsg::Request(request) => {
                if let Some((form, opener)) = tree_forms::form_for(&self.model, request) {
                    fx.push(Effect::Publish(BusEvent::OpenForm { form, opener }));
                }
            }
        }
        self.keep_focus(&before);
        fx
    }

    fn view(&self) -> TreeView {
        let render = Render {
            model: &self.model,
            selected: self.selected,
            focus: self.focus,
        };
        let rows = self
            .model
            .visible(true)
            .into_iter()
            .filter_map(|(n, d)| render.row(n, d))
            .collect();
        let empty = self.model.sessions.is_empty();
        TreeView {
            rows,
            selected: self.selected,
            focus: self.focus,
            empty,
            empty_hint: (empty && self.selected.is_none()).then(|| tr("chat-empty")),
            collapsed: self.collapsed,
            strip: self.collapsed.then(|| render.strip()),
            feed: self.feed.clone(),
            new_session: tr("tree-new-session"),
            new_folder: tr("tree-new-folder"),
        }
    }
}
