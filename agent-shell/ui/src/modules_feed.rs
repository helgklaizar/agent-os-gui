//! Bridge from core-client events to `ModulesController`: the `modules` topic, the `GET /v1/modules` list, connection.
//! No GTK: works on macOS too, so it is covered by integration tests against `fake-agentd`.

use controllers::Controller;
use controllers::controllers::modules::{ModulesController, ModulesMsg};
use controllers::module_types::ModuleRecord;
use controllers::time::Instant;
use core_client::api::FrameKind;
use core_client::{ConnState, CoreEvent};
use serde_json::Value;

/// Topics the modules subscribe to.
pub const TOPICS: [&str; 1] = ["modules"];

/// What the layer above should do after parsing an event.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Routed {
    /// Re-read the list (`GET /v1/modules`) and pass it to [`apply_list`].
    pub refresh: bool,
}

fn records(items: &[Value]) -> Vec<ModuleRecord> {
    items.iter().filter_map(ModuleRecord::from_value).collect()
}

/// Core client event -> modules controller. A frame of the wrong shape is skipped (api.md section 2).
pub fn route(c: &mut ModulesController, event: &CoreEvent, now: Instant) -> Routed {
    match event {
        // the list may have changed while there was no connection, after every (re)connect
        CoreEvent::State(ConnState::Connected) => Routed { refresh: true },
        CoreEvent::Frame(f) if f.topic == "modules" => match f.kind {
            FrameKind::Snapshot => match f.data.get("items").and_then(Value::as_array) {
                Some(items) => {
                    c.update(ModulesMsg::Snapshot(records(items)), now);
                    Routed::default()
                }
                // a snapshot without a list: fetch the full list with a request
                None => Routed { refresh: true },
            },
            FrameKind::Delta => {
                let removed = f.data.get("installed") == Some(&Value::Bool(false))
                    && f.data.get("placement").is_none();
                if removed {
                    if let Some(id) = f.data.get("id").and_then(Value::as_str) {
                        c.update(ModulesMsg::Removed(id.to_string()), now);
                    }
                } else if let Some(r) = ModuleRecord::from_value(&f.data) {
                    c.update(ModulesMsg::Upsert(r), now);
                }
                Routed::default()
            }
            FrameKind::Event => Routed::default(),
        },
        _ => Routed::default(),
    }
}

/// List from `GET /v1/modules`: rows of the wrong shape are skipped.
pub fn apply_list(c: &mut ModulesController, list: &[Value], now: Instant) {
    c.update(ModulesMsg::Snapshot(records(list)), now);
}
