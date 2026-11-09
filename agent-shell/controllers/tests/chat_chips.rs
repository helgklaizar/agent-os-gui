//! S4-06: persona and local folder chips.

mod chat_common;

use chat_common::*;
use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::{Controller, CoreCommand};
use serde_json::json;

#[test]
fn chips_hidden_without_session() {
    let mut c = ChatController::new();
    step(&mut c, ChatMsg::Select(None));
    assert!(c.view().chips.is_none() && c.view().persona_choices.is_none());
}

#[test]
fn chips_show_persona_and_path() {
    let c = open(Some(WORK), vec![]);
    let chips = c.view().chips.expect("chips exist when there is a session");
    assert_eq!(chips.persona_label, "Work");
    assert_eq!(chips.persona_color.as_deref(), Some("#1c7ed6"));
    assert_eq!(chips.path_label, "/home/u/code");
    let c = open(None, vec![]);
    assert_eq!(
        c.view().chips.map(|c| c.persona_label).as_deref(),
        Some("No persona selected")
    );
}

#[test]
fn persona_chip_list_not_prompt() {
    let mut c = open(Some(WORK), vec![]);
    let fx = step(&mut c, ChatMsg::PersonaChipClicked);
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::ListPersonas],
        "the list comes from the core"
    );
    let list = c.view().persona_choices.expect("the list is open");
    let names: Vec<_> = list
        .iter()
        .map(|p| (p.id, p.name.as_str(), p.current))
        .collect();
    assert_eq!(
        names,
        [
            (None, "Same as folder", false),
            (Some(WORK), "Work", true),
            (Some(HOME), "Personal", false)
        ]
    );
    let fx = step(&mut c, ChatMsg::PickPersona(Some(HOME)));
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::SetSessionPersona {
            session_id: SID,
            persona_id: Some(HOME)
        }]
    );
    assert!(
        c.view().persona_choices.is_none(),
        "a choice closes the list"
    );
}

#[test]
fn persona_chip_inherit_choice_sends_null() {
    let mut c = open(Some(WORK), vec![]);
    step(&mut c, ChatMsg::PersonaChipClicked);
    let fx = step(&mut c, ChatMsg::PickPersona(None));
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::SetSessionPersona {
            session_id: SID,
            persona_id: None
        }]
    );
}

#[test]
fn persona_chip_same_choice_sends_nothing() {
    let mut c = open(Some(WORK), vec![]);
    step(&mut c, ChatMsg::PersonaChipClicked);
    assert!(commands(&step(&mut c, ChatMsg::PickPersona(Some(WORK)))).is_empty());
}

#[test]
fn persona_chip_follows_session_updated_frame() {
    let mut c = open(Some(WORK), vec![]);
    let upd = event(
        "session.updated",
        json!({ "session": { "id": SID, "persona_id": HOME, "effective_persona_id": HOME, "path": "/x" } }),
    );
    frame(&mut c, false, upd);
    let chips = c.view().chips.unwrap();
    assert_eq!(
        (chips.persona_label.as_str(), chips.path_label.as_str()),
        ("Personal", "/x")
    );
}

#[test]
fn path_chip_file_dialog() {
    let mut c = open(None, vec![]);
    assert!(!c.view().want_path_dialog);
    let fx = step(&mut c, ChatMsg::PathChipClicked);
    assert!(
        commands(&fx).is_empty(),
        "the command goes after the choice, not on the click"
    );
    assert!(c.view().want_path_dialog, "the view opens the file dialog");
    let fx = step(&mut c, ChatMsg::PickPath(Some("/srv/app".into())));
    assert_eq!(
        commands(&fx),
        vec![&CoreCommand::SetSessionPath {
            session_id: SID,
            path: "/srv/app".into()
        }]
    );
    assert!(!c.view().want_path_dialog);
}

#[test]
fn path_chip_cancel_sends_nothing() {
    let mut c = open(None, vec![]);
    step(&mut c, ChatMsg::PathChipClicked);
    assert!(commands(&step(&mut c, ChatMsg::PickPath(None))).is_empty());
    assert!(!c.view().want_path_dialog);
    for empty in ["", "   "] {
        step(&mut c, ChatMsg::PathChipClicked);
        assert!(commands(&step(&mut c, ChatMsg::PickPath(Some(empty.into())))).is_empty());
    }
}

#[test]
fn chips_without_session_ignore_clicks() {
    let mut c = ChatController::new();
    assert!(step(&mut c, ChatMsg::PersonaChipClicked).is_empty());
    assert!(step(&mut c, ChatMsg::PathChipClicked).is_empty());
    assert!(!c.view().want_path_dialog);
}

#[test]
fn persona_chip_inherit_marked_current_without_own_persona() {
    let mut c = open(None, vec![]);
    step(&mut c, ChatMsg::PersonaChipClicked);
    let list = c.view().persona_choices.unwrap();
    let current: Vec<_> = list.iter().filter(|p| p.current).map(|p| p.id).collect();
    assert_eq!(
        current,
        [None],
        "the session has no persona of its own: \"Same as folder\" is marked"
    );
}
