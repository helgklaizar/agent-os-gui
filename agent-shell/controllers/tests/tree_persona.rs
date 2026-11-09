//! S3-05, S3-10: persona is inherited on read, a thread inherits path and context.

mod tree_common;

use controllers::Controller;
use controllers::controllers::session_tree::TreeMsg;
use controllers::tree_types::{SessionCfg, TreeFrame};
use controllers::tree_view::TreeView;
use tree_common::*;

fn color(v: &TreeView, id: i64) -> Option<String> {
    row(v, s(id)).and_then(|r| r.persona_color.clone())
}

#[test]
fn persona_inherited_dynamically() {
    let mut c = loaded();
    let v = c.view();
    assert_eq!(color(&v, 7).as_deref(), Some(WORK_COLOR), "two folders up");
    assert_eq!(color(&v, 5).as_deref(), Some(WORK_COLOR));
    assert_eq!(
        color(&v, 8).as_deref(),
        Some(WORK_COLOR),
        "a thread is like its parent"
    );
    assert_eq!(
        row(&v, f(1))
            .and_then(|r| r.persona_color.clone())
            .as_deref(),
        Some(WORK_COLOR)
    );
    assert_eq!(color(&v, 3), None, "no persona anywhere: no dot");
    let (mut folders, _) = fixture();
    folders[0].persona_id = Some(HOME);
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::FolderUpdated(folders[0].clone())),
    );
    let v = c.view();
    assert_eq!(
        color(&v, 7).as_deref(),
        Some(HOME_COLOR),
        "changing a folder's persona changes the dot without recreating sessions"
    );
    assert_eq!(color(&v, 5).as_deref(), Some(HOME_COLOR));
    assert_eq!(color(&v, 8).as_deref(), Some(HOME_COLOR));
}

#[test]
fn persona_created_session_follows_later_folder_change() {
    let mut c = loaded();
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::SessionCreated(session(20, Some(1), "New"))),
    );
    assert_eq!(color(&c.view(), 20).as_deref(), Some(WORK_COLOR));
    let mut changed = folder(1, None, "Work", Some(HOME));
    changed.open = true;
    send(&mut c, TreeMsg::Frame(TreeFrame::FolderUpdated(changed)));
    assert_eq!(
        color(&c.view(), 20).as_deref(),
        Some(HOME_COLOR),
        "the persona was not copied on creation"
    );
}

#[test]
fn persona_nearest_folder_wins() {
    let mut c = loaded();
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::FolderUpdated(folder(
            2,
            Some(1),
            "Projects",
            Some(HOME),
        ))),
    );
    assert_eq!(
        color(&c.view(), 7).as_deref(),
        Some(HOME_COLOR),
        "the nearest folder beats the farther one"
    );
    assert_eq!(
        color(&c.view(), 5).as_deref(),
        Some(WORK_COLOR),
        "the neighbouring branch is untouched"
    );
}

#[test]
fn persona_own_overrides_folder() {
    let mut c = loaded();
    let mut own = session(20, Some(1), "Own");
    own.persona_id = Some(HOME);
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionCreated(own)));
    assert_eq!(color(&c.view(), 20).as_deref(), Some(HOME_COLOR));
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::FolderUpdated(folder(1, None, "Work", None))),
    );
    assert_eq!(
        color(&c.view(), 20).as_deref(),
        Some(HOME_COLOR),
        "changing a folder's persona does not touch its own"
    );
    send(
        &mut c,
        TreeMsg::Frame(TreeFrame::FolderUpdated(folder(
            1,
            None,
            "Work",
            Some(WORK),
        ))),
    );
    assert_eq!(color(&c.view(), 20).as_deref(), Some(HOME_COLOR));
}

#[test]
fn persona_unknown_id_has_no_dot() {
    let mut c = loaded();
    let mut odd = session(21, None, "Odd");
    odd.persona_id = Some(999);
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionCreated(odd)));
    assert_eq!(
        color(&c.view(), 21),
        None,
        "persona not in the list: no dot, no crash"
    );
}

#[test]
fn thread_inherits_persona_path_ctx() {
    let mut c = loaded();
    let info = c.session_info(8).expect("thread exists");
    assert_eq!(info.effective_persona_id, Some(WORK));
    assert_eq!(info.path.as_deref(), Some("/home/user/code/release"));
    assert!(info.context_from_parent);
    assert_eq!(info.context_label, None);
    let mut parent = session(7, Some(2), "Release");
    parent.path = Some("/srv/new".to_string());
    parent.persona_id = Some(HOME);
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionUpdated(parent)));
    let info = c.session_info(8).expect("thread exists");
    assert_eq!(
        info.path.as_deref(),
        Some("/srv/new"),
        "a thread takes the parent's path on read"
    );
    assert_eq!(
        info.effective_persona_id,
        Some(HOME),
        "the parent's persona beats the folder's"
    );
    assert_eq!(color(&c.view(), 8).as_deref(), Some(HOME_COLOR));
    let top = c.session_info(7).expect("session exists");
    assert!(!top.context_from_parent, "a session has no parent");
}

#[test]
fn thread_ctx_none_isolated() {
    let mut c = loaded();
    let mut iso = thread(9, 7, Some(2), "Hotfix");
    iso.cfg = SessionCfg {
        ctx: Some("none".to_string()),
        ..SessionCfg::default()
    };
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionUpdated(iso)));
    let info = c.session_info(9).expect("thread exists");
    assert!(!info.context_from_parent, "parent context is not attached");
    assert_eq!(
        info.context_label.as_deref(),
        Some("Parent context: isolated")
    );
    assert_eq!(
        info.effective_persona_id,
        Some(WORK),
        "persona is inherited"
    );
    assert_eq!(
        info.path.as_deref(),
        Some("/home/user/code/release"),
        "path is inherited"
    );
    assert!(
        c.session_info(8).is_some_and(|i| i.context_from_parent),
        "the neighbouring thread is untouched"
    );
    assert!(c.session_info(999).is_none());
}
