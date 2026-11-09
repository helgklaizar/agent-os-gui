//! Shared helpers for module tests: a `modules` table record, a manifest, a controller step, a four-module fixture.
#![allow(dead_code)]

use std::time::Instant;

use controllers::controllers::modules::{ModulesController, ModulesMsg};
use controllers::manifest::Placement;
use controllers::module_types::ModuleRecord;
use controllers::{Controller, Effect};

pub fn manifest(id: &str, name: &str, placement: &str) -> String {
    serde_json::json!({
        "id": id, "name": name, "version": "1.0.0", "entry": "index.html", "placement": placement,
        "permissions": ["GET /sessions"], "network": [], "fs": []
    })
    .to_string()
}

pub fn record(id: &str, placement: Placement, enabled: bool) -> ModuleRecord {
    let p = match placement {
        Placement::Panel => "panel",
        Placement::Widget => "widget",
        Placement::Button => "button",
    };
    ModuleRecord {
        id: id.to_string(),
        name: format!("Module {id}"),
        manifest_json: manifest(id, &format!("Module {id}"), p),
        installed: true,
        enabled,
        placement,
        config_json: "{}".to_string(),
    }
}

pub fn step(c: &mut ModulesController, msg: ModulesMsg) -> Vec<Effect> {
    c.update(msg, Instant::now())
}

pub fn fixture() -> ModulesController {
    let mut c = ModulesController::default();
    step(
        &mut c,
        ModulesMsg::Snapshot(vec![
            record("pan1", Placement::Panel, true),
            record("pan2", Placement::Panel, true),
            record("wid1", Placement::Widget, true),
            record("btn1", Placement::Button, true),
        ]),
    );
    c
}
