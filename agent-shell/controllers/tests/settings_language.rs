//! A language change in Settings → Account reaches the shell as a bus event once saved in the core, not before.
//! The protocol value `ru` is accepted as data but the GUI supports English only, so no event is published for it.

mod settings_common;

use controllers::controllers::settings::SettingsController;
use controllers::i18n::Lang;
use controllers::settings_view::SettingsMsg;
use controllers::{BusEvent, Effect, SettingsPage};
use serde_json::json;
use settings_common::*;

fn loaded() -> SettingsController {
    let mut c = SettingsController::new();
    send(&mut c, SettingsMsg::Open(SettingsPage::Account));
    for r in replies() {
        send(&mut c, SettingsMsg::Reply(r));
    }
    c
}

fn languages(fx: &[Effect]) -> Vec<Lang> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Publish(BusEvent::LanguageChanged(l)) => Some(*l),
            _ => None,
        })
        .collect()
}

#[test]
fn language_saved_in_account_is_published() {
    let mut c = loaded();
    let fx = send(
        &mut c,
        SettingsMsg::Reply(saved_reply(
            "account",
            json!({"name": "Alex", "language": "en"}),
            4,
        )),
    );
    assert_eq!(languages(&fx), [Lang::En]);
    let fx = send(
        &mut c,
        SettingsMsg::Reply(saved_reply(
            "account",
            json!({"name": "Alex", "language": "ru"}),
            5,
        )),
    );
    assert!(languages(&fx).is_empty(), "ru is not a GUI language");
}

#[test]
fn language_is_not_published_without_saved_valid_account_language() {
    let mut c = loaded();
    let cases = [
        // page only read, not saved
        page_reply("account", json!({"language": "en"}), 4),
        // someone else's page
        saved_reply("personas", json!({"language": "en"}), 4),
        // unknown code, not a string, field missing
        saved_reply("account", json!({"language": "de"}), 4),
        saved_reply("account", json!({"language": "EN"}), 4),
        saved_reply("account", json!({"language": 5}), 4),
        saved_reply("account", json!({"name": "Alex"}), 4),
    ];
    for r in cases {
        let fx = send(&mut c, SettingsMsg::Reply(r.clone()));
        assert!(languages(&fx).is_empty(), "{r:?}");
    }
}
