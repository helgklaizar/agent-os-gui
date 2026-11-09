//! Shared helpers for session tree tests: fixture, message sending, row lookup.
#![allow(dead_code)]

use std::time::Instant;

use controllers::controllers::dialog::{DialogController, DialogMsg, DialogView};
use controllers::controllers::session_tree::{SessionTreeController, TreeMsg, TreeRequest};
use controllers::tree_types::{FolderItem, NodeId, PersonaItem, SessionCfg, SessionItem};
use controllers::tree_view::{TreeRow, TreeView};
use controllers::{BusEvent, Controller, CoreCommand, DialogKind, Effect, Tab};

pub const WORK: i64 = 10;
pub const HOME: i64 = 11;
pub const WORK_COLOR: &str = "#1c7ed6";
pub const HOME_COLOR: &str = "#e64980";

pub fn folder(id: i64, parent: Option<i64>, name: &str, persona: Option<i64>) -> FolderItem {
    FolderItem {
        id,
        parent_id: parent,
        name: name.to_string(),
        persona_id: persona,
        open: true,
    }
}

pub fn session(id: i64, folder: Option<i64>, name: &str) -> SessionItem {
    SessionItem {
        id,
        folder_id: folder,
        name: name.to_string(),
        ..SessionItem::default()
    }
}

pub fn thread(id: i64, parent: i64, folder: Option<i64>, name: &str) -> SessionItem {
    SessionItem {
        parent_id: Some(parent),
        cfg: SessionCfg::default(),
        ..session(id, folder, name)
    }
}

pub fn personas() -> Vec<PersonaItem> {
    vec![
        PersonaItem {
            id: WORK,
            name: "Work".to_string(),
            color: WORK_COLOR.to_string(),
        },
        PersonaItem {
            id: HOME,
            name: "Personal".to_string(),
            color: HOME_COLOR.to_string(),
        },
    ]
}

/// Folders: 1 "Work" (persona Work) contains 2 "Projects"; 3 "Home" (persona Personal).
/// Sessions: 7 "Release" in "Projects" with threads 8, 9, 10; 5 "Letters" in "Work"; 6 "Vacation" in "Home"; 3 "Loose" and 4 "Notes" without a folder.
pub fn fixture() -> (Vec<FolderItem>, Vec<SessionItem>) {
    let folders = vec![
        folder(1, None, "Work", Some(WORK)),
        folder(2, Some(1), "Projects", None),
        folder(3, None, "Home", Some(HOME)),
    ];
    let mut release = session(7, Some(2), "Release");
    release.path = Some("/home/user/code/release".to_string());
    let mut leave = session(6, Some(3), "Vacation");
    leave.unread = 2;
    let mut notes = session(4, None, "Notes");
    notes.persona_id = Some(HOME);
    let sessions = vec![
        session(3, None, "Loose"),
        notes,
        session(5, Some(1), "Letters"),
        leave,
        release,
        thread(8, 7, Some(2), "Review"),
        thread(9, 7, Some(2), "Hotfix"),
        thread(10, 7, Some(2), "Third"),
    ];
    (folders, sessions)
}

pub fn loaded() -> SessionTreeController {
    let mut c = SessionTreeController::new();
    let (folders, sessions) = fixture();
    send(&mut c, TreeMsg::Personas(personas()));
    send(&mut c, TreeMsg::Snapshot { folders, sessions });
    c
}

pub fn send(c: &mut SessionTreeController, msg: TreeMsg) -> Vec<Effect> {
    c.update(msg, Instant::now())
}

pub fn row(view: &TreeView, node: NodeId) -> Option<&TreeRow> {
    view.rows.iter().find(|r| r.node == node)
}

pub fn nodes(view: &TreeView) -> Vec<NodeId> {
    view.rows.iter().map(|r| r.node).collect()
}

pub fn s(id: i64) -> NodeId {
    NodeId::Session(id)
}

pub fn f(id: i64) -> NodeId {
    NodeId::Folder(id)
}

pub fn open_session(id: i64) -> Effect {
    Effect::Publish(BusEvent::OpenSession {
        session_id: id,
        tab: Tab::Chat,
    })
}

pub fn has_core_command(fx: &[Effect]) -> bool {
    fx.iter().any(|e| matches!(e, Effect::Command(_)))
}

pub fn open_form(
    c: &mut SessionTreeController,
    d: &mut DialogController,
    request: TreeRequest,
) -> Vec<Effect> {
    let fx = send(c, TreeMsg::Request(request));
    let Some(Effect::Publish(BusEvent::OpenForm { form, opener })) = fx.first().cloned() else {
        panic!("form not requested: {fx:?}");
    };
    dlg(d, DialogMsg::OpenForm { form, opener })
}

pub fn dlg(d: &mut DialogController, msg: DialogMsg) -> Vec<Effect> {
    d.update(msg, std::time::Instant::now())
}

pub fn commands(fx: &[Effect]) -> Vec<CoreCommand> {
    fx.iter()
        .filter_map(|e| {
            if let Effect::Command(c) = e {
                Some(c.clone())
            } else {
                None
            }
        })
        .collect()
}

pub fn top(v: &DialogView) -> Option<DialogKind> {
    v.stack.last().copied()
}
