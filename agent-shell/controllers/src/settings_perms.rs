//! Settings permission matrix: modes per action; a lower level can only tighten (ADR-0005).

use crate::i18n::tr;
use crate::permissions::{Mode, persona_rule_options};
use crate::settings_model::id_of;
use crate::settings_pages::{heading, specs_rows};
use crate::settings_state::State;
use crate::settings_view::{FieldRow, Kind, Row};
use crate::types::SettingsPage;

/// Permissions the matrix shows.
pub const ACTIONS: [&str; 8] = [
    "read",
    "send",
    "delete",
    "pay",
    "shell",
    "browse",
    "memory_write",
    "config_write",
];

/// Permission mode at a level: the draft (`perm:<level>:<action>`) or the matrix value.
pub fn perm_mode(st: &State, level: &str, scope_id: i64, action: &str) -> Option<Mode> {
    let id = format!("perm:{level}:{action}");
    if let Some(t) = st.text.get(&id) {
        return Mode::parse(t);
    }
    saved_perm(st, level, scope_id, action)
}

/// Permission mode from the core matrix, without the draft.
pub fn saved_perm(st: &State, level: &str, scope_id: i64, action: &str) -> Option<Mode> {
    st.model
        .perms
        .rows
        .iter()
        .find(|r| r.scope == level && r.scope_id == scope_id && r.action == action)
        .map(|r| r.mode)
}

fn mode_label(m: Mode) -> String {
    match m {
        Mode::Allow => tr("perm-allow"),
        Mode::Ask => tr("perm-ask"),
        Mode::Deny => tr("perm-deny"),
    }
}

pub fn permissions(st: &State) -> Vec<Row> {
    let mut rows = specs_rows(st, SettingsPage::Permissions);
    let persona = st
        .perm_persona
        .or_else(|| st.model.personas.first().and_then(id_of));
    rows.push(heading(&tr("set-h-matrix")));
    rows.push(Row::Note(tr("set-perm-note")));
    for action in ACTIONS {
        let global = perm_mode(st, "global", 0, action);
        let options = [Mode::Allow, Mode::Ask, Mode::Deny]
            .map(|m| (m.as_str().to_string(), mode_label(m)))
            .to_vec();
        let current = global.unwrap_or_else(|| crate::permissions::default_mode(action));
        rows.push(Row::Field(FieldRow {
            id: format!("perm:global:{action}"),
            label: format!("{action} · {}", tr("set-level-global")),
            kind: Kind::Choice(options),
            value: current.as_str().to_string(),
            error: st.errors.get(&format!("perm:global:{action}")).cloned(),
            dirty: st.text.contains_key(&format!("perm:global:{action}")),
        }));
        if let Some(pid) = persona {
            let own = perm_mode(st, "persona", pid, action);
            let allowed = persona_rule_options(Some(current));
            let options: Vec<(String, String)> = allowed
                .iter()
                .map(|m| (m.as_str().to_string(), mode_label(*m)))
                .collect();
            rows.push(Row::Field(FieldRow {
                id: format!("perm:persona:{action}"),
                label: format!("{action} · {}", tr("set-level-persona")),
                kind: Kind::Choice(options),
                value: own.unwrap_or(current).as_str().to_string(),
                error: st.errors.get(&format!("perm:persona:{action}")).cloned(),
                dirty: st.text.contains_key(&format!("perm:persona:{action}")),
            }));
        }
    }
    rows
}
