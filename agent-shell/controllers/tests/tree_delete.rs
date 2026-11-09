//! S3-06, S3-10: Esc in forms and delete confirmation.

mod tree_common;

use controllers::controllers::dialog::{DialogController, DialogMsg};
use controllers::controllers::session_tree::{TreeMsg, TreeRequest};
use controllers::i18n::tr;
use controllers::{BusEvent, Controller, CoreCommand, DialogKind, Effect, FocusTarget, TreeButton};
use tree_common::*;

#[test]
fn dialog_esc_noop_restores_focus() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::NewSession { folder: None });
    dlg(
        &mut d,
        DialogMsg::SetName("Will not be created".to_string()),
    );
    let fx = dlg(&mut d, DialogMsg::Esc);
    assert!(commands(&fx).is_empty(), "Esc creates nothing: {fx:?}");
    assert_eq!(
        fx,
        vec![Effect::Publish(BusEvent::FocusRestore(
            FocusTarget::TreeButton(TreeButton::NewSession)
        ))]
    );
    assert!(d.view().stack.is_empty() && d.view().form.is_none());
    assert_eq!(c.view().rows.len(), 11, "the tree is unchanged");
    open_form(&mut c, &mut d, TreeRequest::NewSession { folder: None });
    assert_eq!(
        d.view().form.map(|f| f.name),
        Some(String::new()),
        "the new form is empty, old input is gone"
    );
}

#[test]
fn dialog_esc_from_row_action_restores_row_focus() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::Rename { node: s(5) });
    let fx = dlg(&mut d, DialogMsg::Esc);
    assert_eq!(
        fx,
        vec![Effect::Publish(BusEvent::FocusRestore(FocusTarget::Node(
            s(5)
        )))]
    );
    open_form(&mut c, &mut d, TreeRequest::Delete { node: f(3) });
    let fx = dlg(&mut d, DialogMsg::Backdrop);
    assert_eq!(
        fx,
        vec![Effect::Publish(BusEvent::FocusRestore(FocusTarget::Node(
            f(3)
        )))]
    );
}

#[test]
fn delete_confirm_lists_threads() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::Delete { node: s(7) });
    assert_eq!(top(&d.view()), Some(DialogKind::ConfirmDeleteSession));
    let form = d.view().form.expect("confirmation form");
    assert_eq!(form.title, tr("delete-session-title"));
    let text = form.lines.join("\n");
    assert!(text.contains("Release"), "names the session: {text}");
    assert!(text.contains("3 threads"), "and three threads: {text}");
    assert!(!form.has_name && form.folders.is_none());
    assert_eq!(form.submit_label, tr("dialog-delete"));
    let fx = dlg(&mut d, DialogMsg::Confirm);
    assert_eq!(commands(&fx), vec![CoreCommand::DeleteSession { id: 7 }]);
    assert!(d.view().stack.is_empty());
}

#[test]
fn delete_confirm_without_threads_has_no_threads_line() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::Delete { node: s(5) });
    let text = d
        .view()
        .form
        .map(|f| f.lines.join("\n"))
        .unwrap_or_default();
    assert!(
        text.contains("Letters") && !text.contains("thread"),
        "{text}"
    );
}

#[test]
fn delete_thread_confirm_names_only_the_thread() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::Delete { node: s(8) });
    let text = d
        .view()
        .form
        .map(|f| f.lines.join("\n"))
        .unwrap_or_default();
    assert!(text.contains("Review"), "{text}");
    let fx = dlg(&mut d, DialogMsg::Confirm);
    assert_eq!(commands(&fx), vec![CoreCommand::DeleteSession { id: 8 }]);
}

#[test]
fn delete_folder_confirm_text() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::Delete { node: f(1) });
    assert_eq!(top(&d.view()), Some(DialogKind::ConfirmDeleteFolder));
    let form = d.view().form.expect("confirmation form");
    assert_eq!(form.title, tr("delete-folder-title"));
    let text = form.lines.join("\n");
    assert!(text.contains("Work"), "{text}");
    assert!(text.contains("its contents move up one level"), "{text}");
    let fx = dlg(&mut d, DialogMsg::Confirm);
    assert_eq!(commands(&fx), vec![CoreCommand::DeleteFolder { id: 1 }]);
}

#[test]
fn delete_cancel_sends_nothing() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::Delete { node: s(7) });
    let fx = dlg(&mut d, DialogMsg::Cancel);
    assert!(commands(&fx).is_empty());
    assert_eq!(
        fx,
        vec![Effect::Publish(BusEvent::FocusRestore(FocusTarget::Node(
            s(7)
        )))]
    );
}

#[test]
fn delete_confirm_text_keeps_evil_name_literal() {
    let mut c = controllers::controllers::session_tree::SessionTreeController::new();
    let evil = "<img src=x onerror=alert(1)>";
    send(
        &mut c,
        TreeMsg::Snapshot {
            folders: vec![],
            sessions: vec![session(1, None, evil)],
        },
    );
    let mut d = DialogController::new();
    open_form(&mut c, &mut d, TreeRequest::Delete { node: s(1) });
    let text = d
        .view()
        .form
        .map(|f| f.lines.join("\n"))
        .unwrap_or_default();
    assert!(text.contains(evil), "{text}");
}

#[test]
fn logout_and_about_dialogs_still_work_with_forms_in_stack() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::NewFolder { parent: None });
    assert!(d.view().form.is_some());
    dlg(
        &mut d,
        DialogMsg::Open {
            kind: DialogKind::About,
            opener: FocusTarget::MenuItem(controllers::MenuItem::About),
        },
    );
    assert_eq!(top(&d.view()), Some(DialogKind::About));
    dlg(&mut d, DialogMsg::Esc);
    assert_eq!(
        top(&d.view()),
        Some(DialogKind::NewFolder),
        "the form under About stays"
    );
}
