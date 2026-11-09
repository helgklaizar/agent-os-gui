//! `ModulesController`: rendering from the manifest, enabling without a restart, a module crash, violations.

mod modules_common;

use controllers::controllers::modules::{ModulesController, ModulesMsg};
use controllers::manifest::Placement;
use controllers::module_types::{ViolationKind, ViolationReport};
use controllers::{BusEvent, Controller, Effect};
use modules_common::*;

#[test]
fn modules_render_all_three_placements() {
    let c = fixture();
    let v = c.view();
    assert_eq!(
        v.panels.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["pan1", "pan2"]
    );
    assert_eq!(
        v.widgets.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
        ["wid1"]
    );
    assert_eq!(
        v.buttons.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
        ["btn1"]
    );
    assert!(v.rejected.is_empty());
    assert_eq!(v.panels[0].title, "Module pan1");
}

#[test]
fn modules_disabled_and_not_installed_are_hidden() {
    let mut c = ModulesController::default();
    let mut not_installed = record("cat1", Placement::Panel, true);
    not_installed.installed = false;
    step(
        &mut c,
        ModulesMsg::Snapshot(vec![
            record("off1", Placement::Widget, false),
            not_installed,
        ]),
    );
    let v = c.view();
    assert!(v.panels.is_empty() && v.widgets.is_empty() && v.buttons.is_empty());
}

#[test]
fn modules_toggle_without_restart() {
    let mut c = fixture();
    step(
        &mut c,
        ModulesMsg::Upsert(record("wid1", Placement::Widget, false)),
    );
    step(
        &mut c,
        ModulesMsg::Upsert(record("btn1", Placement::Button, false)),
    );
    step(
        &mut c,
        ModulesMsg::Upsert(record("pan2", Placement::Panel, false)),
    );
    let v = c.view();
    assert!(v.widgets.is_empty() && v.buttons.is_empty());
    assert_eq!(v.panels.len(), 1);
    step(
        &mut c,
        ModulesMsg::Upsert(record("wid1", Placement::Widget, true)),
    );
    step(
        &mut c,
        ModulesMsg::Upsert(record("btn1", Placement::Button, true)),
    );
    let v = c.view();
    assert_eq!(v.widgets.len(), 1);
    assert_eq!(v.buttons.len(), 1);
    step(&mut c, ModulesMsg::Removed("wid1".to_string()));
    assert!(c.view().widgets.is_empty());
}

#[test]
fn modules_panel_selection_keeps_state() {
    let mut c = fixture();
    assert_eq!(c.view().active_panel, None);
    step(&mut c, ModulesMsg::SelectPanel("pan1".to_string()));
    step(&mut c, ModulesMsg::SelectPanel("pan2".to_string()));
    step(&mut c, ModulesMsg::SelectPanel("pan1".to_string()));
    let v = c.view();
    assert_eq!(v.active_panel.as_deref(), Some("pan1"));
    // both panels stay loaded: the WebView is not recreated on switching
    assert!(v.panels.iter().all(|p| p.loaded), "{:?}", v.panels);
}

#[test]
fn modules_select_unknown_panel_is_ignored() {
    let mut c = fixture();
    step(&mut c, ModulesMsg::SelectPanel("pan1".to_string()));
    step(&mut c, ModulesMsg::SelectPanel("nope".to_string()));
    step(&mut c, ModulesMsg::SelectPanel("wid1".to_string()));
    assert_eq!(c.view().active_panel.as_deref(), Some("pan1"));
}

#[test]
fn modules_disabling_active_panel_clears_selection() {
    let mut c = fixture();
    step(&mut c, ModulesMsg::SelectPanel("pan2".to_string()));
    step(
        &mut c,
        ModulesMsg::Upsert(record("pan2", Placement::Panel, false)),
    );
    assert_eq!(c.view().active_panel, None);
}

#[test]
fn modules_crash_isolated_and_recoverable() {
    let mut c = fixture();
    step(&mut c, ModulesMsg::SelectPanel("pan1".to_string()));
    step(&mut c, ModulesMsg::Crashed("pan1".to_string()));
    let v = c.view();
    assert!(v.panels[0].crashed, "pan1 is marked as crashed");
    assert!(!v.panels[1].crashed, "pan2 is untouched");
    assert_eq!(v.widgets.len(), 1);
    assert_eq!(v.buttons.len(), 1);
    step(&mut c, ModulesMsg::Reload("pan1".to_string()));
    assert!(!c.view().panels[0].crashed);
    // a crash of a nonexistent module is not a panic
    step(&mut c, ModulesMsg::Crashed("ghost".to_string()));
}

#[test]
fn modules_invalid_manifest_rejected_shell_survives() {
    let mut c = ModulesController::default();
    let mut bad = record("bad1", Placement::Panel, true);
    bad.manifest_json = r#"{"id":"bad1","name":"x","version":"1.0.0","entry":"index.html","placement":"panel","sneaky":1}"#.to_string();
    let mut garbage = record("bad2", Placement::Widget, true);
    garbage.manifest_json = "{not json".to_string();
    step(
        &mut c,
        ModulesMsg::Snapshot(vec![bad, garbage, record("ok1", Placement::Panel, true)]),
    );
    let v = c.view();
    assert_eq!(v.panels.len(), 1);
    assert_eq!(v.panels[0].id, "ok1");
    assert_eq!(v.rejected.len(), 2);
    for r in &v.rejected {
        assert!(!r.reason.is_empty() && !r.reason.contains('⟦'), "{r:?}");
    }
}

#[test]
fn modules_manifest_id_or_placement_must_match_record() {
    let mut c = ModulesController::default();
    let mut wrong_id = record("a1", Placement::Panel, true);
    wrong_id.manifest_json = manifest("other", "x", "panel");
    let mut wrong_place = record("a2", Placement::Panel, true);
    wrong_place.manifest_json = manifest("a2", "x", "widget");
    step(&mut c, ModulesMsg::Snapshot(vec![wrong_id, wrong_place]));
    let v = c.view();
    assert!(v.panels.is_empty() && v.widgets.is_empty());
    assert_eq!(v.rejected.len(), 2);
}

#[test]
fn modules_violation_is_recorded_and_published() {
    let mut c = fixture();
    let effects = step(
        &mut c,
        ModulesMsg::Violation(ViolationReport {
            module_id: "pan1".to_string(),
            kind: ViolationKind::ExternalOrigin,
            detail: "https://evil.example/x".to_string(),
        }),
    );
    assert_eq!(
        effects,
        vec![Effect::Publish(BusEvent::ModuleViolation {
            module_id: "pan1".to_string(),
            kind: "external_origin".to_string(),
        })]
    );
    assert_eq!(c.view().panels[0].violations, 1);
    assert_eq!(c.view().panels[1].violations, 0);
}

#[test]
fn modules_violation_kind_names_are_distinct() {
    let kinds = [
        ViolationKind::ExternalOrigin,
        ViolationKind::AgentdSocket,
        ViolationKind::FileScheme,
        ViolationKind::BridgeMethod,
        ViolationKind::Navigation,
    ];
    let mut names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
    assert!(names.iter().all(|n| !n.is_empty()));
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), kinds.len());
}
