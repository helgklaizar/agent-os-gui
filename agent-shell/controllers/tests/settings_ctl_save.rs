//! Settings dialog: save errors, a stale `rev`, success, input validation.

mod settings_common;

use controllers::i18n::tr;
use controllers::settings_types::{Cmd, Op, Reply, SaveError};
use controllers::settings_view::{BannerKind, SettingsMsg};
use controllers::{Controller, SettingsPage};
use serde_json::json;
use settings_common::*;

#[test]
fn settings_save_error_keeps_value_and_shows_message() {
    let mut c = opened(SettingsPage::Account);
    edit(&mut c, "d:account:name", "Keep Me");
    let fx = send(&mut c, SettingsMsg::Save);
    assert_eq!(
        cmds(&fx),
        vec![Cmd::PatchPage {
            doc: "account".into(),
            rev: 3,
            patch: json!({"name": "Keep Me"})
        }]
    );
    assert!(c.view().saving);
    let err = SaveError {
        status: 500,
        code: "internal".into(),
        fields: vec![],
    };
    send(
        &mut c,
        SettingsMsg::Reply(Reply::Failed {
            op: Op::Patch("account".into()),
            error: err,
        }),
    );
    let v = c.view();
    assert!(!v.saving, "saving finished");
    let banner = v.banner.expect("the error is visible in the dialog");
    assert_eq!(banner.kind, BannerKind::Error);
    assert_eq!(banner.text, tr("set-err-unavailable"));
    let f = field(&c, "d:account:name").expect("field");
    assert_eq!(f.value, "Keep Me", "the entered value is not lost");
    assert!(f.dirty);
    let again = send(&mut c, SettingsMsg::Save);
    assert_eq!(cmds(&again).len(), 1, "saving again is possible");
}

#[test]
fn settings_save_error_by_field_is_shown_next_to_field() {
    let mut c = opened(SettingsPage::Personas);
    edit(&mut c, "d:personas:balance.warn_pct", "50");
    send(&mut c, SettingsMsg::Save);
    let err = SaveError {
        status: 422,
        code: "validation_failed".into(),
        fields: vec!["balance.warn_pct".into()],
    };
    send(
        &mut c,
        SettingsMsg::Reply(Reply::Failed {
            op: Op::Patch("personas".into()),
            error: err,
        }),
    );
    let f = field(&c, "d:personas:balance.warn_pct").expect("field");
    assert_eq!(
        f.error,
        Some(tr("set-err-field")),
        "the error is next to the field"
    );
    assert_eq!(f.value, "50");
    // the next edit clears the field error
    edit(&mut c, "d:personas:balance.warn_pct", "60");
    assert_eq!(
        field(&c, "d:personas:balance.warn_pct").and_then(|f| f.error),
        None
    );
}

#[test]
fn settings_stale_rev_refetches_and_keeps_input() {
    let mut c = opened(SettingsPage::Account);
    edit(&mut c, "d:account:name", "Mine");
    send(&mut c, SettingsMsg::Save);
    let err = SaveError {
        status: 412,
        code: "precondition_failed".into(),
        fields: vec![],
    };
    let fx = send(
        &mut c,
        SettingsMsg::Reply(Reply::Failed {
            op: Op::Patch("account".into()),
            error: err,
        }),
    );
    assert_eq!(
        cmds(&fx),
        vec![Cmd::FetchPage("account".into())],
        "the page is re-read"
    );
    assert_eq!(c.view().banner.map(|b| b.text), Some(tr("set-err-stale")));
    send(
        &mut c,
        SettingsMsg::Reply(page_reply(
            "account",
            json!({"name": "Phone", "language": "ru"}),
            9,
        )),
    );
    assert_eq!(
        field(&c, "d:account:name").map(|f| f.value),
        Some("Mine".into()),
        "the input is kept over someone else's edit"
    );
    let again = send(&mut c, SettingsMsg::Save);
    assert_eq!(
        cmds(&again),
        vec![Cmd::PatchPage {
            doc: "account".into(),
            rev: 9,
            patch: json!({"name": "Mine"})
        }],
        "the retry goes with the new rev"
    );
}

#[test]
fn settings_changed_frame_refetches_only_on_new_rev() {
    let mut c = opened(SettingsPage::Account);
    assert!(
        cmds(&send(
            &mut c,
            SettingsMsg::Changed {
                page: "account".into(),
                rev: 3
            }
        ))
        .is_empty()
    );
    let fx = send(
        &mut c,
        SettingsMsg::Changed {
            page: "account".into(),
            rev: 4,
        },
    );
    assert_eq!(cmds(&fx), vec![Cmd::FetchPage("account".into())]);
}

#[test]
fn settings_save_success_clears_draft_and_shows_saved() {
    let mut c = opened(SettingsPage::Account);
    edit(&mut c, "d:account:name", "New");
    send(&mut c, SettingsMsg::Save);
    send(
        &mut c,
        SettingsMsg::Reply(saved_reply(
            "account",
            json!({"name": "New", "language": "ru"}),
            4,
        )),
    );
    let v = c.view();
    assert!(!v.dirty && !v.saving);
    assert_eq!(v.banner.map(|b| b.text), Some(tr("set-saved")));
    assert!(!field(&c, "d:account:name").is_some_and(|f| f.dirty));
}

#[test]
fn settings_invalid_input_blocks_save_and_marks_field() {
    let mut c = opened(SettingsPage::Schedule);
    edit(&mut c, "d:schedule:summary.time", "25:99");
    assert!(
        field(&c, "d:schedule:summary.time")
            .and_then(|f| f.error)
            .is_some(),
        "the error is visible while typing"
    );
    let fx = send(&mut c, SettingsMsg::Save);
    assert!(
        cmds(&fx).is_empty(),
        "an invalid time is not sent to the core"
    );
    assert_eq!(c.view().banner.map(|b| b.text), Some(tr("set-err-invalid")));
    edit(&mut c, "d:schedule:summary.time", "07:30");
    let fx = send(&mut c, SettingsMsg::Save);
    assert_eq!(
        cmds(&fx),
        vec![Cmd::PatchPage {
            doc: "schedule".into(),
            rev: 4,
            patch: json!({"summary": {"time": "07:30"}})
        }]
    );
}
