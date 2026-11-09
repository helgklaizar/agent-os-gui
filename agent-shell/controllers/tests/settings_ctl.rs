//! Settings dialog: entry and exit, draft, theme, the "Account" page, button reachability.

mod settings_common;

use controllers::controllers::settings::SettingsController;
use controllers::settings_types::{Close, Cmd};
use controllers::settings_view::{Kind, Row, SettingsMsg};
use controllers::{BusEvent, Controller, Effect, FocusTarget, MenuItem, SettingsPage, Theme};
use settings_common::*;

#[test]
fn settings_opens_from_menu_and_quota() {
    // the events are published by the bar (menu, bar_weather_menu.rs) and quota (bar_quota_view.rs): here, how the dialog answers them
    let mut c = SettingsController::new();
    let fx = send(
        &mut c,
        SettingsMsg::Bus(BusEvent::OpenSettings(SettingsPage::Root)),
    );
    assert!(c.view().open);
    assert_eq!(
        c.view().page,
        SettingsPage::Account,
        "root is the first page"
    );
    assert_eq!(
        cmds(&fx),
        vec![Cmd::FetchAll],
        "all pages are fetched on open"
    );
    // entry from quota opens "Models"
    let mut c = SettingsController::new();
    send(
        &mut c,
        SettingsMsg::Bus(BusEvent::OpenSettings(SettingsPage::Models)),
    );
    assert_eq!(c.view().page, SettingsPage::Models);
}

#[test]
fn settings_close_by_esc_button_backdrop() {
    for how in [Close::Esc, Close::Button, Close::Backdrop] {
        let mut c = SettingsController::new();
        send(
            &mut c,
            SettingsMsg::Bus(BusEvent::OpenSettings(SettingsPage::Root)),
        );
        assert!(c.view().open);
        let fx = send(&mut c, SettingsMsg::Close(how));
        assert!(!c.view().open, "{how:?}: dialog closed");
        assert!(
            fx.contains(&Effect::Publish(BusEvent::FocusRestore(
                FocusTarget::MenuItem(MenuItem::Settings)
            ))),
            "{how:?}: focus returns to the menu item that opened it: {fx:?}"
        );
        assert!(
            send(&mut c, SettingsMsg::Close(how)).is_empty(),
            "closing again does nothing"
        );
    }
}

#[test]
fn settings_esc_cancels_confirm_before_closing() {
    let mut c = opened(SettingsPage::Personas);
    send(
        &mut c,
        SettingsMsg::Act(controllers::settings_types::Action::PersonaDelete(1)),
    );
    assert!(c.view().confirm.is_some());
    send(&mut c, SettingsMsg::Close(Close::Esc));
    assert!(
        c.view().open && c.view().confirm.is_none(),
        "the first Esc closes the confirmation"
    );
    send(&mut c, SettingsMsg::Close(Close::Esc));
    assert!(!c.view().open);
}

#[test]
fn settings_draft_survives_page_switch() {
    let mut c = opened(SettingsPage::Account);
    edit(&mut c, "d:account:name", "Draft Name");
    assert!(c.view().dirty);
    for page in [
        SettingsPage::Personas,
        SettingsPage::Models,
        SettingsPage::Tunnels,
    ] {
        send(&mut c, SettingsMsg::Select(page));
    }
    send(&mut c, SettingsMsg::Select(SettingsPage::Account));
    let f = field(&c, "d:account:name").expect("name field");
    assert_eq!(f.value, "Draft Name", "draft is kept");
    assert!(f.dirty);
    // and after closing and reopening
    send(&mut c, SettingsMsg::Close(Close::Button));
    send(&mut c, SettingsMsg::Open(SettingsPage::Account));
    assert_eq!(
        field(&c, "d:account:name").map(|f| f.value),
        Some("Draft Name".into())
    );
}

#[test]
fn settings_theme_choice_goes_to_theme_controller() {
    let mut c = opened(SettingsPage::Account);
    let fx = send(
        &mut c,
        SettingsMsg::Edit {
            id: "theme".into(),
            value: "dark".into(),
        },
    );
    assert_eq!(
        fx,
        vec![Effect::Publish(BusEvent::SelectTheme(Theme::Dark))]
    );
    assert_eq!(field(&c, "theme").map(|f| f.value), Some("dark".into()));
    assert!(
        send(
            &mut c,
            SettingsMsg::Edit {
                id: "theme".into(),
                value: "red".into()
            }
        )
        .is_empty()
    );
    send(&mut c, SettingsMsg::ThemeNow(Theme::Light));
    assert_eq!(
        field(&c, "theme").map(|f| f.value),
        Some("light".into()),
        "the applied theme is shown"
    );
}

#[test]
fn settings_account_has_no_password_or_profile_export() {
    let c = opened(SettingsPage::Account);
    let rows = rows(&c);
    assert!(
        rows.iter().all(|r| matches!(r, Row::Field(_))),
        "only fields on the page, no buttons: {rows:?}"
    );
    let text = shown_text(&c).to_lowercase();
    for word in ["pam", "export", "import", "password"] {
        assert!(!text.contains(word), "{word}: {text}");
    }
    let ids: Vec<_> = rows
        .iter()
        .filter_map(|r| {
            if let Row::Field(f) = r {
                Some(f.id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(ids, ["d:account:name", "theme", "d:account:language"]);
    let lang = field(&c, "d:account:language").expect("language");
    match lang.kind {
        Kind::Choice(o) => assert_eq!(
            o.iter().map(|(v, _)| v.as_str()).collect::<Vec<_>>(),
            ["en"]
        ),
        other => panic!("language is a choice: {other:?}"),
    }
}

#[test]
fn settings_dialog_buttons_reachable_in_every_state() {
    // the 1024×640 geometry is checked by a UI scenario; here, that the controller never hides the buttons in any state
    let mut c = SettingsController::new();
    assert!(c.view().save_visible && c.view().close_visible, "closed");
    send(&mut c, SettingsMsg::Open(SettingsPage::Account));
    assert!(
        c.view().loading && c.view().save_visible && c.view().close_visible,
        "loading"
    );
    for r in replies() {
        send(&mut c, SettingsMsg::Reply(r));
    }
    for (_, pages) in controllers::settings_types::NAV {
        for page in pages {
            send(&mut c, SettingsMsg::Select(*page));
            let v = c.view();
            assert!(v.save_visible && v.close_visible, "{page:?}");
        }
    }
    send(&mut c, SettingsMsg::Select(SettingsPage::Personas));
    send(
        &mut c,
        SettingsMsg::Act(controllers::settings_types::Action::PersonaDelete(1)),
    );
    assert!(
        c.view().confirm.is_some() && c.view().save_visible && c.view().close_visible,
        "confirmation"
    );
}
