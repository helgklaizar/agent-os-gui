//! `ModulesController`: module panels, widgets and buttons driven by the manifest.
//!
//! Model without GTK or WebKit: what to show and in which state. The WebView itself and its sandbox live in `ui`.

use std::collections::{HashMap, HashSet};

use crate::bus::BusEvent;
use crate::controller::{Controller, Effect};
use crate::i18n::tr;
use crate::manifest::{Manifest, Placement, parse_manifest};
use crate::module_types::{Badge, MAX_TITLE, ModuleRecord, ViolationReport, plain_text};
use crate::time::Instant;

/// Panel tab in the tools column.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelTab {
    pub id: String,
    pub title: String,
    /// The WebView is created and lives while the tab is enabled: switching tabs does not lose state.
    pub loaded: bool,
    pub crashed: bool,
    pub violations: usize,
}

/// Module indicator in the bar.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetItem {
    pub id: String,
    pub title: String,
    pub badge: Badge,
}

/// Module button in the sessions column.
#[derive(Debug, Clone, PartialEq)]
pub struct ButtonItem {
    pub id: String,
    pub title: String,
    pub badge: Badge,
}

/// A module that cannot be shown, and the reason (in plain language).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModulesView {
    pub panels: Vec<PanelTab>,
    pub widgets: Vec<WidgetItem>,
    pub buttons: Vec<ButtonItem>,
    pub active_panel: Option<String>,
    pub rejected: Vec<Rejected>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModulesMsg {
    /// Full list (`GET /modules` or a snapshot of the `modules` topic).
    Snapshot(Vec<ModuleRecord>),
    /// Changed or added row (delta).
    Upsert(ModuleRecord),
    Removed(String),
    SelectPanel(String),
    /// The module's WebView process crashed.
    Crashed(String),
    /// The user restarted the crashed panel.
    Reload(String),
    Violation(ViolationReport),
    /// Widget or button data from the module (raw JSON, sanitised in `Badge::from_module_json`).
    BadgeUpdate {
        id: String,
        json: String,
    },
}

/// Validated entry: the manifest is parsed and consistent with the table row.
fn check_record(r: &ModuleRecord) -> Result<Manifest, String> {
    let manifest = parse_manifest(&r.manifest_json).map_err(|e| e.message())?;
    if manifest.id != r.id {
        return Err(tr("module-error-id-mismatch"));
    }
    if manifest.placement != r.placement {
        return Err(tr("module-error-placement-mismatch"));
    }
    Ok(manifest)
}

#[derive(Debug, Default)]
pub struct ModulesController {
    records: Vec<ModuleRecord>,
    active: Option<String>,
    loaded: HashSet<String>,
    crashed: HashSet<String>,
    violations: HashMap<String, usize>,
    badges: HashMap<String, Badge>,
}

impl ModulesController {
    /// A module is shown if it is installed, enabled and its manifest is validated.
    fn shown(r: &ModuleRecord) -> bool {
        r.installed && r.enabled && check_record(r).is_ok()
    }

    fn is_shown_panel(&self, id: &str) -> bool {
        self.records
            .iter()
            .any(|r| r.id == id && r.placement == Placement::Panel && Self::shown(r))
    }

    /// Forget the state of a module that is no longer shown (WebView closed, profile not needed).
    fn forget_if_hidden(&mut self, id: &str) {
        if !self.records.iter().any(|r| r.id == id && Self::shown(r)) {
            self.loaded.remove(id);
            self.crashed.remove(id);
            self.badges.remove(id);
            if self.active.as_deref() == Some(id) {
                self.active = None;
            }
        }
    }

    fn forget_all_hidden(&mut self) {
        let ids: Vec<String> = self
            .loaded
            .iter()
            .chain(self.crashed.iter())
            .chain(self.badges.keys())
            .chain(self.active.iter())
            .cloned()
            .collect();
        for id in ids {
            self.forget_if_hidden(&id);
        }
    }
}

impl Controller for ModulesController {
    type Msg = ModulesMsg;
    type View = ModulesView;

    fn update(&mut self, msg: ModulesMsg, _now: Instant) -> Vec<Effect> {
        match msg {
            ModulesMsg::Snapshot(records) => {
                self.records = records;
                self.forget_all_hidden();
            }
            ModulesMsg::Upsert(record) => {
                let id = record.id.clone();
                match self.records.iter_mut().find(|r| r.id == id) {
                    Some(slot) => *slot = record,
                    None => self.records.push(record),
                }
                self.forget_if_hidden(&id);
            }
            ModulesMsg::Removed(id) => {
                self.records.retain(|r| r.id != id);
                self.violations.remove(&id);
                self.forget_if_hidden(&id);
            }
            ModulesMsg::SelectPanel(id) => {
                if self.is_shown_panel(&id) {
                    self.loaded.insert(id.clone());
                    self.active = Some(id);
                }
            }
            ModulesMsg::Crashed(id) => {
                if self.is_shown_panel(&id) {
                    self.crashed.insert(id);
                }
            }
            ModulesMsg::Reload(id) => {
                self.crashed.remove(&id);
            }
            ModulesMsg::Violation(report) => {
                *self.violations.entry(report.module_id.clone()).or_default() += 1;
                // details (address, method) stay in the upper layer's local log; only the kind goes to the bus
                return vec![Effect::Publish(BusEvent::ModuleViolation {
                    module_id: report.module_id,
                    kind: report.kind.as_str().to_string(),
                })];
            }
            ModulesMsg::BadgeUpdate { id, json } => {
                let takes_badge = self
                    .records
                    .iter()
                    .any(|r| r.id == id && r.placement != Placement::Panel && Self::shown(r));
                if takes_badge {
                    self.badges.insert(id, Badge::from_module_json(&json));
                }
            }
        }
        Vec::new()
    }

    fn view(&self) -> ModulesView {
        let mut view = ModulesView {
            active_panel: self.active.clone(),
            ..ModulesView::default()
        };
        for r in self.records.iter().filter(|r| r.installed && r.enabled) {
            let manifest = match check_record(r) {
                Ok(m) => m,
                Err(reason) => {
                    view.rejected.push(Rejected {
                        id: r.id.clone(),
                        reason,
                    });
                    continue;
                }
            };
            let title = plain_text(&manifest.name, MAX_TITLE, true);
            let badge = self.badges.get(&r.id).cloned().unwrap_or_default();
            match r.placement {
                Placement::Panel => view.panels.push(PanelTab {
                    id: r.id.clone(),
                    title,
                    loaded: self.loaded.contains(&r.id),
                    crashed: self.crashed.contains(&r.id),
                    violations: self.violations.get(&r.id).copied().unwrap_or(0),
                }),
                Placement::Widget => view.widgets.push(WidgetItem {
                    id: r.id.clone(),
                    title,
                    badge,
                }),
                Placement::Button => view.buttons.push(ButtonItem {
                    id: r.id.clone(),
                    title,
                    badge,
                }),
            }
        }
        view
    }
}
