//! Module data: widget and button badges, hostile strings, the core record, name spoofing.

mod modules_common;

use controllers::Controller;
use controllers::controllers::modules::{ModulesController, ModulesMsg};
use controllers::manifest::Placement;
use controllers::module_types::{Badge, Icon, ModuleRecord, Tone, ViolationKind, ViolationReport};
use modules_common::*;

#[test]
fn modules_badge_from_json_keeps_only_closed_set() {
    let b = Badge::from_module_json(
        r#"{"text":"3 new","number":3,"icon":"warning","tone":"warn","html":"<b>x</b>","onclick":"alert(1)"}"#,
    );
    assert_eq!(b.text, "3 new");
    assert_eq!(b.number, Some(3.0));
    assert_eq!(b.icon, Some(Icon::Warning));
    assert_eq!(b.tone, Some(Tone::Warn));
    let b = Badge::from_module_json(r##"{"text":"x","icon":"../../etc/passwd","tone":"#ff0000"}"##);
    assert_eq!(b.icon, None, "icon outside the set");
    assert_eq!(b.tone, None, "colour outside the palette");
    assert_eq!(Badge::from_module_json("not json"), Badge::default());
    assert_eq!(Badge::from_module_json(r#"{"number":"NaN"}"#).number, None);
}

#[test]
fn modules_badge_text_is_plain_and_bounded() {
    // hostile strings stay literal text; control characters and RTL overrides are stripped; the length is capped
    let b = Badge::from_module_json(r#"{"text":"<img src=x onerror=alert(1)>"}"#);
    assert_eq!(b.text, "<img src=x onerror=alert(1)>");
    let b = Badge::from_module_json("{\"text\":\"a\\nb\\u202Ec\\u0000d\\u2066e\"}");
    assert_eq!(b.text, "abcde");
    let long = format!(r#"{{"text":"{}"}}"#, "é".repeat(10_000));
    assert!(Badge::from_module_json(&long).text.chars().count() <= 64);
}

#[test]
fn modules_badge_update_applies_to_widget_and_button() {
    let mut c = fixture();
    step(
        &mut c,
        ModulesMsg::BadgeUpdate {
            id: "wid1".to_string(),
            json: r#"{"text":"42","tone":"ok"}"#.to_string(),
        },
    );
    step(
        &mut c,
        ModulesMsg::BadgeUpdate {
            id: "btn1".to_string(),
            json: r#"{"text":"Launch","icon":"clock"}"#.to_string(),
        },
    );
    // an update of a nonexistent or panel module is ignored
    step(
        &mut c,
        ModulesMsg::BadgeUpdate {
            id: "pan1".to_string(),
            json: "{}".to_string(),
        },
    );
    step(
        &mut c,
        ModulesMsg::BadgeUpdate {
            id: "ghost".to_string(),
            json: "{}".to_string(),
        },
    );
    let v = c.view();
    assert_eq!(v.widgets[0].badge.text, "42");
    assert_eq!(v.widgets[0].badge.tone, Some(Tone::Ok));
    assert_eq!(v.buttons[0].badge.icon, Some(Icon::Clock));
}

#[test]
fn modules_evil_names_are_shown_literally() {
    // hostile strings in a module name reach the view as text, nothing is parsed
    let mut c = ModulesController::default();
    let mut records = Vec::new();
    for (i, evil) in controllers::testkit::evil_strings().into_iter().enumerate() {
        let id = format!("evil{i}");
        let mut r = record(&id, Placement::Panel, true);
        r.name = evil.clone();
        r.manifest_json = manifest(&id, &evil, "panel");
        records.push((evil, r));
    }
    step(
        &mut c,
        ModulesMsg::Snapshot(records.iter().map(|(_, r)| r.clone()).collect()),
    );
    let v = c.view();
    for (evil, r) in &records {
        let shown = v
            .panels
            .iter()
            .find(|p| p.id == r.id)
            .map(|p| p.title.clone());
        match shown {
            Some(title) => {
                let expect: String = evil
                    .chars()
                    .filter(|ch| {
                        !ch.is_control()
                            && !('\u{202A}'..='\u{202E}').contains(ch)
                            && !('\u{2066}'..='\u{2069}').contains(ch)
                    })
                    .take(128)
                    .collect();
                assert!(!title.is_empty());
                assert!(
                    title.chars().count() <= 129,
                    "name not truncated: {}",
                    title.chars().count()
                );
                assert!(
                    title.starts_with(expect.chars().take(20).collect::<String>().as_str()),
                    "{title:?} vs {expect:?}"
                );
            }
            None => assert!(
                v.rejected.iter().any(|x| x.id == r.id),
                "{evil:.30?}: neither shown nor rejected"
            ),
        }
    }
}

#[test]
fn modules_title_comes_from_validated_manifest() {
    // the name comes from the validated manifest, not from the record field, which the schema does not constrain
    let mut c = ModulesController::default();
    let mut r = record("pan1", Placement::Panel, true);
    r.name = "spoof <b>from the record</b>".to_string();
    step(&mut c, ModulesMsg::Snapshot(vec![r]));
    assert_eq!(c.view().panels[0].title, "Module pan1");
}

#[test]
fn modules_snapshot_replaces_state_and_drops_missing() {
    let mut c = fixture();
    step(&mut c, ModulesMsg::SelectPanel("pan2".to_string()));
    step(&mut c, ModulesMsg::Crashed("pan2".to_string()));
    step(
        &mut c,
        ModulesMsg::Snapshot(vec![record("pan1", Placement::Panel, true)]),
    );
    let v = c.view();
    assert_eq!(v.panels.len(), 1);
    assert!(v.widgets.is_empty() && v.buttons.is_empty());
    assert_eq!(
        v.active_panel, None,
        "the active panel disappeared from the list"
    );
    // a returning panel starts clean: not loaded and not crashed
    step(
        &mut c,
        ModulesMsg::Upsert(record("pan2", Placement::Panel, true)),
    );
    let pan2 = &c.view().panels[1];
    assert!(!pan2.loaded && !pan2.crashed, "{pan2:?}");
}

#[test]
fn modules_violation_event_carries_no_detail() {
    let mut c = fixture();
    let effects = step(
        &mut c,
        ModulesMsg::Violation(ViolationReport {
            module_id: "wid1".to_string(),
            kind: ViolationKind::BridgeMethod,
            detail: "POST /tokens secret-in-details".to_string(),
        }),
    );
    let text = format!("{effects:?}");
    assert!(
        text.contains("bridge_method") && !text.contains("secret"),
        "{text}"
    );
}

#[test]
fn modules_record_from_core_json() {
    let item = serde_json::json!({
        "id": "notes-panel", "name": "Notes", "installed": true, "enabled": true, "placement": "panel",
        "manifest": {"id": "notes-panel", "name": "Notes", "version": "1.2.0", "entry": "panel.html",
                     "placement": "panel"},
        "config": {"city": "Haifa"}
    });
    let r = ModuleRecord::from_value(&item).expect("record");
    assert_eq!(
        (r.id.as_str(), r.installed, r.enabled),
        ("notes-panel", true, true)
    );
    assert_eq!(r.placement, Placement::Panel);
    assert_eq!(r.config_json, r#"{"city":"Haifa"}"#);
    let mut c = ModulesController::default();
    step(&mut c, ModulesMsg::Snapshot(vec![r]));
    assert_eq!(
        c.view().panels.len(),
        1,
        "a manifest from a JSON object passes validation"
    );
}

#[test]
fn modules_record_from_core_json_skips_malformed_frames() {
    let ok = serde_json::json!({"id": "a", "placement": "widget"});
    assert!(ModuleRecord::from_value(&ok).is_some());
    for bad in [
        serde_json::json!(null),
        serde_json::json!([]),
        serde_json::json!({"placement": "panel"}),
        serde_json::json!({"id": 5, "placement": "panel"}),
        serde_json::json!({"id": "a"}),
        serde_json::json!({"id": "a", "placement": "window"}),
        serde_json::json!({"id": "a", "installed": false}),
    ] {
        assert!(ModuleRecord::from_value(&bad).is_none(), "{bad}");
    }
    // not installed and not enabled if the flags are not sent
    let r = ModuleRecord::from_value(&ok).expect("record");
    assert!(!r.installed && !r.enabled);
}
