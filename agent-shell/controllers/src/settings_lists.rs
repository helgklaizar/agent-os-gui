//! Settings pages with record lists: Personas, Connections, Modules, Tunnels, Files.

use serde_json::Value;

use crate::i18n::{tr, tr_args};
use crate::module_manifest;
use crate::settings_entities::{
    account_new_specs, mcp_new_specs, module_specs, module_url_spec, persona_specs,
    tunnel_new_specs,
};
use crate::settings_fields::{Spec, page_specs};
use crate::settings_model::{id_of, is_unofficial, text_of};
use crate::settings_policy::is_editable_path;
use crate::settings_state::{State, field_row};
use crate::settings_types::Action;
use crate::settings_view::{ItemRow, Row, RowAction};
use crate::types::SettingsPage;

pub use crate::settings_entities::entity_spec;

fn act(action: Action, label: String, destructive: bool) -> RowAction {
    RowAction {
        action,
        label,
        destructive,
    }
}

fn fields(st: &State, specs: &[Spec]) -> Vec<Row> {
    specs.iter().map(|s| field_row(st, s)).collect()
}

fn heading(text: &str) -> Row {
    Row::Heading(text.to_string())
}

/// Index of the persona being edited; `None` means the new-persona form.
pub fn editing_persona(st: &State) -> Option<i64> {
    match st.selected_persona {
        Some(id) if st.model.persona(id).is_some() => Some(id),
        Some(_) => None,
        None => st.model.personas.first().and_then(id_of),
    }
}

pub fn personas(st: &State) -> Vec<Row> {
    let current = editing_persona(st);
    let mut rows = vec![heading(&tr("set-h-personas"))];
    for p in &st.model.personas {
        let Some(id) = id_of(p) else { continue };
        rows.push(Row::Item(ItemRow {
            title: text_of(p, "name").to_string(),
            subtitle: format!("{} · {}", text_of(p, "lang"), text_of(p, "mem_mode")),
            tags: Vec::new(),
            actions: vec![
                act(Action::PersonaSelect(id), tr("set-edit"), false),
                act(Action::PersonaDelete(id), tr("set-delete"), true),
            ],
            selected: current == Some(id),
        }));
    }
    let key = current.map_or_else(|| "new".to_string(), |id| id.to_string());
    rows.push(heading(&if current.is_some() {
        tr("set-h-persona-edit")
    } else {
        tr("set-h-persona-new")
    }));
    rows.extend(fields(st, &persona_specs(&key, &st.model)));
    if current.is_none() {
        rows.push(Row::Button(act(
            Action::PersonaNew,
            tr("set-persona-create"),
            false,
        )));
    } else {
        rows.push(Row::Button(act(
            Action::PersonaFormNew,
            tr("set-persona-new"),
            false,
        )));
    }
    rows.push(heading(&tr("set-h-balance")));
    rows.extend(fields(st, &page_specs(SettingsPage::Personas, &st.model)));
    rows
}

fn mcp_list(st: &State) -> Vec<Value> {
    st.model
        .at("connections", "mcp")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub fn connections(st: &State) -> Vec<Row> {
    let mut rows = vec![heading(&tr("set-h-accounts"))];
    for a in &st.model.accounts {
        let Some(id) = id_of(a) else { continue };
        let tags = if is_unofficial(a) {
            vec![tr("set-unofficial")]
        } else {
            Vec::new()
        };
        rows.push(Row::Item(ItemRow {
            title: text_of(a, "label").to_string(),
            subtitle: format!("{} · {}", text_of(a, "type"), text_of(a, "status")),
            tags,
            actions: vec![
                act(Action::AccountConnect(id), tr("set-connect"), false),
                act(Action::AccountDelete(id), tr("set-delete"), true),
            ],
            selected: false,
        }));
    }
    rows.push(heading(&tr("set-h-account-new")));
    rows.extend(fields(st, &account_new_specs()));
    rows.push(Row::Button(act(
        Action::AccountAdd,
        tr("set-account-add"),
        false,
    )));
    rows.push(heading(&tr("set-h-mcp")));
    for (i, s) in mcp_list(st).iter().enumerate() {
        let on = s.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        let target = text_of(
            s,
            if text_of(s, "transport") == "http" {
                "url"
            } else {
                "command"
            },
        );
        rows.push(Row::Item(ItemRow {
            title: text_of(s, "name").to_string(),
            subtitle: format!("{} · {target}", text_of(s, "transport")),
            tags: Vec::new(),
            actions: vec![
                act(
                    Action::McpToggle(i),
                    if on {
                        tr("set-ind-off")
                    } else {
                        tr("set-ind-on")
                    },
                    false,
                ),
                act(Action::McpRemove(i), tr("set-delete"), true),
            ],
            selected: false,
        }));
    }
    rows.extend(fields(st, &mcp_new_specs()));
    rows.push(Row::Button(act(Action::McpAdd, tr("set-mcp-add"), false)));
    rows
}

pub fn modules(st: &State) -> Vec<Row> {
    let mut rows = vec![heading(&tr("set-h-installed"))];
    let installed = |m: &&Value| m.get("installed").and_then(Value::as_bool).unwrap_or(false);
    for m in st.model.modules.iter().filter(installed) {
        let id = text_of(m, "id").to_string();
        let on = m.get("enabled").and_then(Value::as_bool).unwrap_or(false);
        rows.push(Row::Item(ItemRow {
            title: text_of(m, "name").to_string(),
            subtitle: format!("{} · {}", id, text_of(m, "placement")),
            tags: vec![if on {
                tr("set-ind-state-on")
            } else {
                tr("set-ind-state-off")
            }],
            actions: vec![
                act(
                    Action::ModuleToggle(id.clone()),
                    if on {
                        tr("set-ind-off")
                    } else {
                        tr("set-ind-on")
                    },
                    false,
                ),
                act(Action::ModuleRemove(id), tr("set-delete"), true),
            ],
            selected: false,
        }));
        rows.extend(fields(st, &module_specs(m)));
    }
    rows.push(heading(&tr("set-h-market")));
    for m in st.model.modules.iter().filter(|m| !installed(m)) {
        let valid = module_manifest::validate(&m["manifest"]).is_empty();
        let actions = if valid {
            vec![act(
                Action::ModuleInstall(text_of(m, "id").to_string()),
                tr("set-install"),
                false,
            )]
        } else {
            Vec::new()
        };
        rows.push(Row::Item(ItemRow {
            title: text_of(m, "name").to_string(),
            subtitle: format!("{} · {}", text_of(m, "id"), text_of(m, "placement")),
            tags: if valid {
                Vec::new()
            } else {
                vec![tr("set-manifest-invalid")]
            },
            actions,
            selected: false,
        }));
    }
    rows.push(heading(&tr("set-h-module-url")));
    rows.push(Row::Note(tr("set-module-url-note")));
    rows.push(field_row(st, &module_url_spec()));
    rows.push(Row::Button(act(
        Action::ModuleInstallUrl,
        tr("set-install"),
        false,
    )));
    rows
}

pub fn tunnels(st: &State) -> Vec<Row> {
    let mut rows = fields(st, &page_specs(SettingsPage::Tunnels, &st.model));
    rows.push(heading(&tr("set-h-tunnels")));
    for t in &st.model.tunnels {
        let Some(id) = id_of(t) else { continue };
        let on = t.get("enabled").and_then(Value::as_bool).unwrap_or(false);
        let descr = text_of(t, "descr");
        rows.push(Row::Item(ItemRow {
            title: text_of(t, "type").to_string(),
            subtitle: if descr.is_empty() {
                text_of(t, "addr").to_string()
            } else {
                format!("{} · {descr}", text_of(t, "addr"))
            },
            tags: vec![text_of(t, "status").to_string()],
            actions: vec![
                act(
                    Action::TunnelToggle(id),
                    if on {
                        tr("set-ind-off")
                    } else {
                        tr("set-ind-on")
                    },
                    false,
                ),
                act(Action::TunnelDelete(id), tr("set-delete"), true),
            ],
            selected: false,
        }));
    }
    rows.push(heading(&tr("set-h-tunnel-new")));
    rows.extend(fields(st, &tunnel_new_specs()));
    rows.push(Row::Button(act(
        Action::TunnelAdd,
        tr("set-tunnel-add"),
        false,
    )));
    rows
}

pub fn files(st: &State) -> Vec<Row> {
    let mut rows = vec![Row::Note(tr("set-files-note"))];
    for f in st.model.files.iter().filter(|f| is_editable_path(&f.path)) {
        rows.push(Row::Item(ItemRow {
            title: f.path.clone(),
            subtitle: tr_args("set-file-size", &[("size", &f.size.to_string())]),
            tags: Vec::new(),
            actions: vec![act(Action::FileOpen(f.path.clone()), tr("set-edit"), false)],
            selected: st.file.as_ref().is_some_and(|o| o.path == f.path),
        }));
    }
    if let Some(open) = &st.file {
        rows.push(heading(&open.path));
        rows.push(field_row(st, &crate::settings_entities::file_text_spec()));
    }
    rows
}
