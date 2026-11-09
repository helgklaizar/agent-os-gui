//! S3-07: move and rename are separate, form requests.

mod tree_common;

use controllers::controllers::dialog::{DialogController, DialogMsg};
use controllers::controllers::session_tree::{TreeMsg, TreeRequest};
use controllers::tree_types::TreeFrame;
use controllers::{BusEvent, Controller, CoreCommand, Effect, FocusTarget, TreeButton};
use tree_common::*;

fn form_of(
    c: &mut controllers::controllers::session_tree::SessionTreeController,
    request: TreeRequest,
) -> Vec<Effect> {
    send(c, TreeMsg::Request(request))
}

fn open_in_dialog(effects: &[Effect], d: &mut DialogController) {
    let Some(Effect::Publish(BusEvent::OpenForm { form, opener })) = effects.first().cloned()
    else {
        panic!("form not requested: {effects:?}");
    };
    send_dialog(d, DialogMsg::OpenForm { form, opener });
}

fn send_dialog(d: &mut DialogController, msg: DialogMsg) -> Vec<Effect> {
    d.update(msg, std::time::Instant::now())
}

#[test]
fn move_does_not_rename() {
    let mut c = loaded();
    let mut d = DialogController::new();
    let fx = form_of(&mut c, TreeRequest::Move { session: 7 });
    open_in_dialog(&fx, &mut d);
    let form = d.view().form.expect("Move form");
    assert!(!form.has_name, "the Move form has no name field");
    assert!(form.personas.is_none());
    send_dialog(&mut d, DialogMsg::SetFolder(Some(3)));
    let fx = send_dialog(&mut d, DialogMsg::Submit);
    let commands: Vec<_> = fx
        .iter()
        .filter_map(|e| {
            if let Effect::Command(c) = e {
                Some(c.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        commands,
        vec![CoreCommand::MoveSession {
            id: 7,
            folder_id: Some(3)
        }],
        "only the folder changes"
    );
    let mut moved = session(7, Some(3), "Release");
    moved.path = Some("/home/user/code/release".to_string());
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionUpdated(moved)));
    let v = c.view();
    assert_eq!(
        row(&v, s(7)).map(|r| (r.name.as_str(), r.depth)),
        Some(("Release", 1)),
        "same name, the row is in the new folder"
    );
}

#[test]
fn move_to_no_folder_is_null_folder() {
    let mut c = loaded();
    let mut d = DialogController::new();
    let fx = form_of(&mut c, TreeRequest::Move { session: 7 });
    open_in_dialog(&fx, &mut d);
    send_dialog(&mut d, DialogMsg::SetFolder(None));
    let fx = send_dialog(&mut d, DialogMsg::Submit);
    assert!(
        fx.contains(&Effect::Command(CoreCommand::MoveSession {
            id: 7,
            folder_id: None
        })),
        "{fx:?}"
    );
}

#[test]
fn rename_does_not_move() {
    let mut c = loaded();
    let mut d = DialogController::new();
    let fx = form_of(&mut c, TreeRequest::Rename { node: s(7) });
    open_in_dialog(&fx, &mut d);
    let form = d.view().form.expect("Rename form");
    assert!(form.has_name);
    assert_eq!(form.name, "Release", "the field holds the current name");
    assert!(
        form.folders.is_none() && form.personas.is_none(),
        "the Rename form has no folder"
    );
    send_dialog(&mut d, DialogMsg::SetName("Release 1.3".to_string()));
    let fx = send_dialog(&mut d, DialogMsg::Submit);
    let commands: Vec<_> = fx
        .iter()
        .filter_map(|e| {
            if let Effect::Command(c) = e {
                Some(c.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        commands,
        vec![CoreCommand::RenameSession {
            id: 7,
            name: "Release 1.3".to_string()
        }],
        "only the name changes"
    );
    let mut renamed = session(7, Some(2), "Release 1.3");
    renamed.path = Some("/home/user/code/release".to_string());
    send(&mut c, TreeMsg::Frame(TreeFrame::SessionUpdated(renamed)));
    assert_eq!(
        row(&c.view(), s(7)).map(|r| (r.name.clone(), r.depth)),
        Some(("Release 1.3".to_string(), 2))
    );
}

#[test]
fn rename_folder_sends_folder_command_and_empty_name_rejected() {
    let mut c = loaded();
    let mut d = DialogController::new();
    let fx = form_of(&mut c, TreeRequest::Rename { node: f(1) });
    open_in_dialog(&fx, &mut d);
    send_dialog(&mut d, DialogMsg::SetName("  ".to_string()));
    assert!(
        send_dialog(&mut d, DialogMsg::Submit)
            .iter()
            .all(|e| !matches!(e, Effect::Command(_))),
        "an empty name is not accepted"
    );
    assert_eq!(d.view().stack.len(), 1, "the dialog stays open");
    send_dialog(&mut d, DialogMsg::SetName("Clients".to_string()));
    let fx = send_dialog(&mut d, DialogMsg::Submit);
    assert!(
        fx.contains(&Effect::Command(CoreCommand::RenameFolder {
            id: 1,
            name: "Clients".to_string()
        })),
        "{fx:?}"
    );
}

#[test]
fn request_for_missing_target_does_nothing() {
    let mut c = loaded();
    assert!(form_of(&mut c, TreeRequest::Move { session: 999 }).is_empty());
    assert!(form_of(&mut c, TreeRequest::Rename { node: f(999) }).is_empty());
    assert!(form_of(&mut c, TreeRequest::Delete { node: s(999) }).is_empty());
    assert!(form_of(&mut c, TreeRequest::NewThread { parent: 999 }).is_empty());
    assert!(
        form_of(&mut c, TreeRequest::Move { session: 8 }).is_empty(),
        "a thread cannot be moved on its own"
    );
}

#[test]
fn request_opener_is_the_calling_element() {
    let mut c = loaded();
    let fx = form_of(&mut c, TreeRequest::NewSession { folder: None });
    assert!(
        matches!(
            &fx[..],
            [Effect::Publish(BusEvent::OpenForm {
                opener: FocusTarget::TreeButton(TreeButton::NewSession),
                ..
            })]
        ),
        "{fx:?}"
    );
    let fx = form_of(&mut c, TreeRequest::NewFolder { parent: None });
    assert!(
        matches!(
            &fx[..],
            [Effect::Publish(BusEvent::OpenForm {
                opener: FocusTarget::TreeButton(TreeButton::NewFolder),
                ..
            })]
        ),
        "{fx:?}"
    );
    let fx = form_of(&mut c, TreeRequest::Rename { node: s(5) });
    assert!(
        matches!(&fx[..], [Effect::Publish(BusEvent::OpenForm { opener: FocusTarget::Node(n), .. })] if *n == s(5)),
        "{fx:?}"
    );
}
