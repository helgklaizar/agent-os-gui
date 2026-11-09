//! S2-08, S2-10: dialog stack, Esc and backdrop close the top one, focus returns, About, Logout with confirmation.

mod bar_common;

use bar_common::*;
use controllers::controllers::dialog::{DialogController, DialogMsg};
use controllers::testkit::evil_strings;
use controllers::{BusEvent, Controller, CoreCommand, DialogKind, Effect, FocusTarget, MenuItem};

const ABOUT_FROM_MENU: FocusTarget = FocusTarget::MenuItem(MenuItem::About);
const LOGOUT_FROM_MENU: FocusTarget = FocusTarget::MenuItem(MenuItem::Logout);

fn open(c: &mut DialogController, kind: DialogKind, opener: FocusTarget) -> Vec<Effect> {
    send(c, DialogMsg::Open { kind, opener })
}

#[test]
fn dialog_esc_closes_restores_focus() {
    let mut c = DialogController::new();
    let fx = open(&mut c, DialogKind::About, ABOUT_FROM_MENU);
    assert_eq!(c.view().stack, [DialogKind::About]);
    assert!(
        fx.contains(&Effect::Command(CoreCommand::FetchAbout)),
        "About asks for the core version: {fx:?}"
    );
    let fx = send(&mut c, DialogMsg::Esc);
    assert!(c.view().stack.is_empty(), "Esc closed the dialog");
    assert_eq!(
        fx,
        vec![Effect::Publish(BusEvent::FocusRestore(ABOUT_FROM_MENU))]
    );
    assert!(
        send(&mut c, DialogMsg::Esc).is_empty(),
        "Esc without a dialog does nothing"
    );
}

#[test]
fn dialog_backdrop_click_closes_top_and_restores_focus() {
    let mut c = DialogController::new();
    open(&mut c, DialogKind::About, ABOUT_FROM_MENU);
    let fx = send(&mut c, DialogMsg::Backdrop);
    assert!(c.view().stack.is_empty());
    assert_eq!(
        fx,
        vec![Effect::Publish(BusEvent::FocusRestore(ABOUT_FROM_MENU))]
    );
    assert!(send(&mut c, DialogMsg::Backdrop).is_empty());
}

#[test]
fn dialog_stack_closes_only_top_and_returns_its_own_focus() {
    let mut c = DialogController::new();
    open(&mut c, DialogKind::About, ABOUT_FROM_MENU);
    open(&mut c, DialogKind::ConfirmLogout, LOGOUT_FROM_MENU);
    assert_eq!(
        c.view().stack,
        [DialogKind::About, DialogKind::ConfirmLogout]
    );
    let fx = send(&mut c, DialogMsg::Esc);
    assert_eq!(
        c.view().stack,
        [DialogKind::About],
        "only the top one is closed"
    );
    assert_eq!(
        fx,
        vec![Effect::Publish(BusEvent::FocusRestore(LOGOUT_FROM_MENU))]
    );
    let fx = send(&mut c, DialogMsg::Esc);
    assert!(c.view().stack.is_empty());
    assert_eq!(
        fx,
        vec![Effect::Publish(BusEvent::FocusRestore(ABOUT_FROM_MENU))]
    );
}

#[test]
fn dialog_same_kind_is_not_stacked_twice() {
    let mut c = DialogController::new();
    open(&mut c, DialogKind::About, ABOUT_FROM_MENU);
    assert!(open(&mut c, DialogKind::About, ABOUT_FROM_MENU).is_empty());
    assert_eq!(c.view().stack.len(), 1);
}

#[test]
fn logout_cancel_noop() {
    let mut c = DialogController::new();
    let before = c.view();
    open(&mut c, DialogKind::ConfirmLogout, LOGOUT_FROM_MENU);
    assert_eq!(
        c.view().stack,
        [DialogKind::ConfirmLogout],
        "confirmation request"
    );
    for dismiss in [DialogMsg::Cancel, DialogMsg::Esc, DialogMsg::Backdrop] {
        open(&mut c, DialogKind::ConfirmLogout, LOGOUT_FROM_MENU);
        let fx = send(&mut c, dismiss.clone());
        assert!(
            !fx.iter().any(|e| matches!(e, Effect::Command(_))),
            "{dismiss:?} does not end the session: {fx:?}"
        );
        assert_eq!(c.view(), before, "{dismiss:?}: state as before opening");
    }
}

#[test]
fn logout_confirm_ends_session_once() {
    let mut c = DialogController::new();
    assert!(
        send(&mut c, DialogMsg::Confirm).is_empty(),
        "nothing to confirm without a request"
    );
    open(&mut c, DialogKind::ConfirmLogout, LOGOUT_FROM_MENU);
    let fx = send(&mut c, DialogMsg::Confirm);
    assert!(fx.contains(&Effect::Command(CoreCommand::Logout)), "{fx:?}");
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::Command(CoreCommand::Logout)))
            .count(),
        1
    );
    assert!(c.view().stack.is_empty());
    assert!(
        !send(&mut c, DialogMsg::Confirm)
            .iter()
            .any(|e| matches!(e, Effect::Command(_))),
        "repeated confirmation does not send Logout"
    );
}

#[test]
fn about_shows_name_version_platform_config() {
    let mut c = DialogController::new();
    open(&mut c, DialogKind::About, ABOUT_FROM_MENU);
    let a = c.view().about;
    assert_eq!(
        (a.name.as_str(), a.platform.as_str(), a.config_path.as_str()),
        ("Agent OS", "Ubuntu 26.04 · Wayland", "~/.agentos/")
    );
    assert_eq!(a.version, "no data", "no core reply yet");
    send(&mut c, DialogMsg::AboutVersion(Some("0.3.1".into())));
    assert_eq!(c.view().about.version, "0.3.1");
    send(&mut c, DialogMsg::AboutVersion(None));
    assert_eq!(c.view().about.version, "no data");
}

#[test]
fn escape_about_fields() {
    for evil in evil_strings() {
        let mut c = DialogController::new();
        send(&mut c, DialogMsg::AboutVersion(Some(evil.clone())));
        assert_eq!(
            c.view().about.version,
            evil,
            "version from the core verbatim"
        );
    }
}

#[test]
fn confirm_does_not_logout_from_other_dialogs() {
    let mut c = DialogController::new();
    open(&mut c, DialogKind::About, ABOUT_FROM_MENU);
    let fx = send(&mut c, DialogMsg::Confirm);
    assert!(fx.is_empty(), "Confirm in About does nothing: {fx:?}");
    assert_eq!(c.view().stack, [DialogKind::About], "About stayed open");
}
