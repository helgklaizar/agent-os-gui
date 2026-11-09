//! S3-04, S3-10: creation forms, indented folder list, default persona.

mod tree_common;

use controllers::controllers::dialog::{DialogController, DialogMsg};
use controllers::controllers::session_tree::TreeRequest;
use controllers::i18n::tr;
use controllers::{BusEvent, Controller, CoreCommand, DialogKind, Effect, FocusTarget, TreeButton};
use tree_common::*;

#[test]
fn dialog_create_session_empty_name_rejected() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::NewSession { folder: Some(2) });
    assert_eq!(top(&d.view()), Some(DialogKind::NewSession));
    let form = d.view().form.expect("form");
    assert!(
        form.has_name && !form.can_submit,
        "\"Create\" is blocked with an empty name"
    );
    assert_eq!(form.submit_label, tr("dialog-create"));
    let fx = dlg(&mut d, DialogMsg::Submit);
    assert!(
        commands(&fx).is_empty(),
        "an empty name is not accepted: {fx:?}"
    );
    assert!(
        d.view().form.is_some_and(|f| f.name_error.is_some()),
        "the hint is shown"
    );
    assert_eq!(
        top(&d.view()),
        Some(DialogKind::NewSession),
        "the dialog stays"
    );
    dlg(&mut d, DialogMsg::SetName("   \t ".to_string()));
    assert!(
        commands(&dlg(&mut d, DialogMsg::Submit)).is_empty(),
        "whitespace counts as empty"
    );
    dlg(&mut d, DialogMsg::SetName("New".to_string()));
    assert!(
        d.view()
            .form
            .is_some_and(|f| f.can_submit && f.name_error.is_none())
    );
    let fx = dlg(&mut d, DialogMsg::Submit);
    assert_eq!(
        commands(&fx),
        vec![CoreCommand::CreateSession {
            name: "New".to_string(),
            folder_id: Some(2),
            persona_id: None
        }]
    );
    assert!(d.view().stack.is_empty(), "the dialog closed");
    assert!(
        fx.iter().any(|e| matches!(
            e,
            Effect::Publish(BusEvent::FocusRestore(FocusTarget::TreeButton(
                TreeButton::NewSession
            )))
        )),
        "{fx:?}"
    );
}

#[test]
fn dialog_name_is_trimmed_and_kept_literal() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::NewSession { folder: None });
    dlg(&mut d, DialogMsg::SetName("  <b>x</b> \" ".to_string()));
    let fx = dlg(&mut d, DialogMsg::Submit);
    assert_eq!(
        commands(&fx),
        vec![CoreCommand::CreateSession {
            name: "<b>x</b> \"".to_string(),
            folder_id: None,
            persona_id: None
        }]
    );
}

#[test]
fn dialog_folder_list_indented() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::NewSession { folder: None });
    let folders = d.view().form.and_then(|f| f.folders).expect("folder list");
    let line: Vec<_> = folders
        .iter()
        .map(|o| (o.id, o.label.as_str(), o.indent))
        .collect();
    assert_eq!(
        line,
        vec![
            (None, "No folder", 0),
            (Some(1), "Work", 0),
            (Some(2), "Projects", 16),
            (Some(3), "Home", 0)
        ]
    );
    assert_eq!(tr("dialog-no-folder"), "No folder");
    assert!(
        folders[0].selected,
        "a call without a folder selects \"No folder\""
    );
    dlg(&mut d, DialogMsg::SetFolder(Some(2)));
    let folders = d.view().form.and_then(|f| f.folders).expect("folder list");
    assert_eq!(
        folders
            .iter()
            .filter(|o| o.selected)
            .map(|o| o.id)
            .collect::<Vec<_>>(),
        vec![Some(2)]
    );
}

#[test]
fn dialog_folder_preselected_from_caller() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::NewSession { folder: Some(3) });
    let folders = d.view().form.and_then(|f| f.folders).expect("folder list");
    assert_eq!(
        folders.iter().find(|o| o.selected).map(|o| o.id),
        Some(Some(3))
    );
}

#[test]
fn dialog_persona_default_option() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::NewSession { folder: Some(1) });
    let personas = d
        .view()
        .form
        .and_then(|f| f.personas)
        .expect("persona list");
    assert_eq!(personas[0].id, None);
    assert_eq!(personas[0].label, "Same as folder / default");
    assert_eq!(personas[0].label, tr("dialog-persona-inherit"));
    assert!(
        personas[0].selected,
        "the \"same as folder\" item is selected by default"
    );
    assert_eq!(
        personas.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![None, Some(WORK), Some(HOME)]
    );
    assert_eq!(personas[1].color.as_deref(), Some(WORK_COLOR));
    dlg(&mut d, DialogMsg::SetName("A".to_string()));
    let fx = dlg(&mut d, DialogMsg::Submit);
    assert_eq!(
        commands(&fx),
        vec![CoreCommand::CreateSession {
            name: "A".to_string(),
            folder_id: Some(1),
            persona_id: None
        }],
        "persona_id = NULL"
    );
}

#[test]
fn dialog_persona_explicit_choice_is_sent() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::NewSession { folder: None });
    dlg(&mut d, DialogMsg::SetName("B".to_string()));
    dlg(&mut d, DialogMsg::SetPersona(Some(HOME)));
    let fx = dlg(&mut d, DialogMsg::Submit);
    assert_eq!(
        commands(&fx),
        vec![CoreCommand::CreateSession {
            name: "B".to_string(),
            folder_id: None,
            persona_id: Some(HOME)
        }]
    );
}

#[test]
fn dialog_new_folder_creates_in_chosen_parent() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::NewFolder { parent: Some(1) });
    assert_eq!(top(&d.view()), Some(DialogKind::NewFolder));
    assert!(
        commands(&dlg(&mut d, DialogMsg::Submit)).is_empty(),
        "an empty folder name is not accepted"
    );
    dlg(&mut d, DialogMsg::SetName("Subfolder".to_string()));
    dlg(&mut d, DialogMsg::SetFolder(Some(3)));
    let fx = dlg(&mut d, DialogMsg::Submit);
    assert_eq!(
        commands(&fx),
        vec![CoreCommand::CreateFolder {
            name: "Subfolder".to_string(),
            parent_id: Some(3),
            persona_id: None
        }]
    );
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Publish(BusEvent::FocusRestore(FocusTarget::TreeButton(
            TreeButton::NewFolder
        )))
    )));
}

#[test]
fn dialog_thread_name_only() {
    let (mut c, mut d) = (loaded(), DialogController::new());
    open_form(&mut c, &mut d, TreeRequest::NewThread { parent: 7 });
    assert_eq!(top(&d.view()), Some(DialogKind::NewThread));
    let form = d.view().form.expect("form");
    assert!(form.has_name);
    assert!(
        form.folders.is_none() && form.personas.is_none(),
        "persona and path are inherited from the parent, no choice"
    );
    assert!(commands(&dlg(&mut d, DialogMsg::Submit)).is_empty());
    dlg(&mut d, DialogMsg::SetName("Discussion".to_string()));
    let fx = dlg(&mut d, DialogMsg::Submit);
    assert_eq!(
        commands(&fx),
        vec![CoreCommand::CreateThread {
            parent_id: 7,
            name: "Discussion".to_string()
        }]
    );
}
