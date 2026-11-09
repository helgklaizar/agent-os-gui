//! Actions on Settings records: personas, accounts, MCP, modules, tunnels,
//! indicators, files. Deleting a persona, account, module or tunnel goes only through a confirmation listing what is affected.

use serde_json::{Value, json};

use crate::controller::Effect;
use crate::i18n::{tr, tr_args};
use crate::settings_forms::{
    account_add, issue, mcp, mcp_add, module_url, patch_mcp, persona_new, tunnel_add,
};
use crate::settings_model::{id_of, text_of};
use crate::settings_policy::is_editable_path;
use crate::settings_save::{banner, cmd, indicator_draft};
use crate::settings_state::{OpenFile, PendingConfirm, State};
use crate::settings_types::{Action, Cmd};
use crate::settings_view::BannerKind;

fn confirm(st: &mut State, action: Action, title: &str, lines: Vec<String>) -> Vec<Effect> {
    st.confirm = Some(PendingConfirm {
        action,
        title: title.to_string(),
        lines,
    });
    Vec::new()
}

fn flip(list: &[Value], id: &str) -> bool {
    !list
        .iter()
        .find(|x| text_of(x, "id") == id)
        .and_then(|x| x.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn move_indicator(st: &mut State, id: &str, delta: isize) {
    let list = indicator_draft(st);
    let Some(i) = list.iter().position(|x| text_of(x, "id") == id) else {
        return;
    };
    let j = i.saturating_add_signed(delta);
    if j < list.len() {
        list.swap(i, j);
    }
    renumber(list);
}

/// `position` is the list index: order and value never diverge.
fn renumber(list: &mut [Value]) {
    for (i, it) in list.iter_mut().enumerate() {
        it["position"] = json!(i);
    }
}

fn open_file(st: &mut State, path: &str) -> Vec<Effect> {
    if !is_editable_path(path) {
        st.banner = Some(banner(BannerKind::Error, tr("set-err-path")));
        return Vec::new();
    }
    st.text.remove("f:text");
    st.file = Some(OpenFile {
        path: path.to_string(),
        rev: String::new(),
        text: String::new(),
        loaded: false,
    });
    vec![cmd(Cmd::FetchFile(path.to_string()))]
}

/// Perform the action.
pub fn act(st: &mut State, action: Action) -> Vec<Effect> {
    match action {
        Action::PersonaFormNew => {
            st.selected_persona = Some(0);
            Vec::new()
        }
        Action::PersonaSelect(id) => {
            st.selected_persona = Some(id);
            Vec::new()
        }
        Action::PersonaNew => persona_new(st),
        Action::PersonaDelete(id) => {
            let name = st.model.persona(id).map(|p| text_of(p, "name").to_string());
            let mut lines = vec![
                tr_args("set-confirm-name", &[("name", &name.unwrap_or_default())]),
                tr("set-confirm-persona-refs"),
                tr("set-confirm-persona-accounts"),
            ];
            let prio = st.model.at("personas", "balance.priority_persona_id");
            if prio.and_then(Value::as_i64) == Some(id) {
                lines.push(tr("set-confirm-persona-priority"));
            }
            confirm(
                st,
                Action::PersonaDelete(id),
                &tr("set-confirm-persona"),
                lines,
            )
        }
        Action::AccountAdd => account_add(st),
        Action::AccountConnect(id) => issue(st, Cmd::ConnectAccount(id)),
        Action::AccountDelete(id) => {
            let label = st.model.account_label(id);
            let lines = vec![
                tr_args("set-confirm-name", &[("name", &label)]),
                tr("set-confirm-account-links"),
            ];
            confirm(
                st,
                Action::AccountDelete(id),
                &tr("set-confirm-account"),
                lines,
            )
        }
        Action::McpAdd => mcp_add(st),
        Action::McpToggle(i) => {
            let mut list = mcp(st);
            if let Some(s) = list.get_mut(i) {
                let on = s.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                s["enabled"] = json!(!on);
            }
            patch_mcp(st, list)
        }
        Action::McpRemove(i) => {
            let mut list = mcp(st);
            if i < list.len() {
                list.remove(i);
            }
            patch_mcp(st, list)
        }
        Action::ModuleInstall(id) => issue(st, Cmd::InstallModule(json!({ "catalog_id": id }))),
        Action::ModuleInstallUrl => module_url(st),
        Action::ModuleToggle(id) => {
            let on = flip(&st.model.modules, &id);
            issue(
                st,
                Cmd::PatchModule {
                    id,
                    body: json!({ "enabled": on }),
                },
            )
        }
        Action::ModuleRemove(id) => {
            let name = st
                .model
                .modules
                .iter()
                .find(|m| text_of(m, "id") == id)
                .map(|m| text_of(m, "name").to_string())
                .unwrap_or_default();
            let lines = vec![
                tr_args("set-confirm-name", &[("name", &name)]),
                tr("set-confirm-module-lines"),
            ];
            confirm(
                st,
                Action::ModuleRemove(id),
                &tr("set-confirm-module"),
                lines,
            )
        }
        Action::TunnelAdd => tunnel_add(st),
        Action::TunnelToggle(id) => {
            let on = !st
                .model
                .tunnels
                .iter()
                .find(|t| id_of(t) == Some(id))
                .and_then(|t| t.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            issue(
                st,
                Cmd::PatchTunnel {
                    id,
                    body: json!({ "enabled": on }),
                },
            )
        }
        Action::TunnelDelete(id) => {
            let addr = st
                .model
                .tunnels
                .iter()
                .find(|t| id_of(t) == Some(id))
                .map(|t| text_of(t, "addr").to_string())
                .unwrap_or_default();
            let lines = vec![
                tr_args("set-confirm-name", &[("name", &addr)]),
                tr("set-confirm-tunnel-lines"),
            ];
            confirm(
                st,
                Action::TunnelDelete(id),
                &tr("set-confirm-tunnel"),
                lines,
            )
        }
        Action::IndicatorToggle(id) => {
            if let Some(it) = indicator_draft(st)
                .iter_mut()
                .find(|x| text_of(x, "id") == id)
            {
                let on = it.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                it["enabled"] = json!(!on);
            }
            Vec::new()
        }
        Action::IndicatorUp(id) => {
            move_indicator(st, &id, -1);
            Vec::new()
        }
        Action::IndicatorDown(id) => {
            move_indicator(st, &id, 1);
            Vec::new()
        }
        Action::FileOpen(path) => open_file(st, &path),
    }
}

/// Answer to a confirmation: "yes" performs the deletion, "no" changes nothing.
pub fn confirmed(st: &mut State, yes: bool) -> Vec<Effect> {
    let Some(pending) = st.confirm.take() else {
        return Vec::new();
    };
    if !yes {
        return Vec::new();
    }
    let c = match pending.action {
        Action::PersonaDelete(id) => Cmd::DeletePersona(id),
        Action::AccountDelete(id) => Cmd::DeleteAccount(id),
        Action::ModuleRemove(id) => Cmd::DeleteModule(id),
        Action::TunnelDelete(id) => Cmd::DeleteTunnel(id),
        _ => return Vec::new(),
    };
    issue(st, c)
}
