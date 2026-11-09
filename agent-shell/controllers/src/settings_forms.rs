//! Settings record creation forms: input validation and the command to the core.

use serde_json::{Value, json};

use crate::controller::Effect;
use crate::i18n::tr;
use crate::settings_entities::{
    account_new_specs, mcp_new_specs, module_url_spec, persona_specs, tunnel_new_specs,
};
use crate::settings_fields::{Spec, parse};
use crate::settings_save::{banner, cmd};
use crate::settings_state::State;
use crate::settings_types::Cmd;
use crate::settings_view::BannerKind;

/// Values of a new record's field group: `Err` is the field that failed validation (the error is already recorded on the field).
fn collect(st: &mut State, specs: &[Spec], required: &[&str]) -> Result<Value, ()> {
    let mut body = json!({});
    let mut ok = true;
    for s in specs {
        let text = st.value(s);
        if required.contains(&s.path.as_str()) && text.trim().is_empty() {
            st.errors.insert(s.id.clone(), tr("set-err-required"));
            ok = false;
            continue;
        }
        match parse(s, &text) {
            Ok(Value::Null) => {}
            Ok(v) if !s.path.starts_with("acc:") => body[s.path.as_str()] = v,
            Ok(_) => {}
            Err(bad) => {
                st.errors.insert(s.id.clone(), bad.text());
                ok = false;
            }
        }
    }
    if ok { Ok(body) } else { Err(()) }
}

/// What the form sent to the core: form fields are cleared only after a successful reply; on failure the input stays.
fn remember(st: &mut State, prefix: &str) {
    let sent: Vec<(String, String)> = st
        .text
        .iter()
        .filter(|(k, _)| k.starts_with(prefix))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    st.sent.extend(sent);
}

pub(crate) fn issue(st: &mut State, c: Cmd) -> Vec<Effect> {
    st.pending += 1;
    st.failed = false;
    st.banner = None;
    vec![cmd(c)]
}

pub(crate) fn mcp(st: &State) -> Vec<Value> {
    st.model
        .at("connections", "mcp")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn patch_mcp(st: &mut State, list: Vec<Value>) -> Vec<Effect> {
    let rev = st.model.rev("connections");
    issue(
        st,
        Cmd::PatchPage {
            doc: "connections".into(),
            rev,
            patch: json!({ "mcp": list }),
        },
    )
}

fn valid_mcp_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 32
        && !n.contains("__")
        && n.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub(crate) fn persona_new(st: &mut State) -> Vec<Effect> {
    let specs = persona_specs("new", &st.model);
    let Ok(mut body) = collect(st, &specs, &["name"]) else {
        st.banner = Some(banner(BannerKind::Error, tr("set-err-invalid")));
        return Vec::new();
    };
    let ids: Vec<i64> = specs
        .iter()
        .filter(|s| s.path.starts_with("acc:") && st.value(s) == "true")
        .filter_map(|s| s.path.strip_prefix("acc:")?.parse().ok())
        .collect();
    if !ids.is_empty() {
        body["account_ids"] = json!(ids);
    }
    for s in &specs {
        let text = st.value(s);
        st.sent.insert(s.id.clone(), text);
    }
    issue(st, Cmd::CreatePersona(body))
}

pub(crate) fn account_add(st: &mut State) -> Vec<Effect> {
    let Ok(body) = collect(st, &account_new_specs(), &["label"]) else {
        return Vec::new();
    };
    let mut body = body;
    if body.get("type").is_none() {
        body["type"] = json!("wa");
    }
    for s in account_new_specs() {
        let text = st.value(&s);
        st.sent.insert(s.id.clone(), text);
    }
    remember(st, "a:new:");
    issue(st, Cmd::CreateAccount(body))
}

pub(crate) fn mcp_add(st: &mut State) -> Vec<Effect> {
    let specs = mcp_new_specs();
    let (name, url) = (st.value(&specs[0]), st.value(&specs[1]));
    let mut bad = false;
    if !valid_mcp_name(&name) {
        st.errors
            .insert(specs[0].id.clone(), tr("set-err-mcp-name"));
        bad = true;
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() < 9 {
        st.errors.insert(specs[1].id.clone(), tr("set-err-url"));
        bad = true;
    }
    if bad {
        return Vec::new();
    }
    let mut list = mcp(st);
    list.push(json!({"name": name, "transport": "http", "url": url, "enabled": true}));
    remember(st, "c:new:");
    patch_mcp(st, list)
}

pub(crate) fn module_url(st: &mut State) -> Vec<Effect> {
    let spec = module_url_spec();
    let url = st.value(&spec);
    // https only: an address with another scheme never reaches the core (the core checks SSRF, but we reject the scheme up front)
    if !url.starts_with("https://")
        || url.len() < 12
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        st.errors.insert(spec.id.clone(), tr("set-err-url"));
        return Vec::new();
    }
    remember(st, "m:url");
    issue(st, Cmd::InstallModule(json!({ "url": url })))
}

pub(crate) fn tunnel_add(st: &mut State) -> Vec<Effect> {
    let Ok(mut body) = collect(st, &tunnel_new_specs(), &["addr"]) else {
        return Vec::new();
    };
    if body.get("type").is_none() {
        body["type"] = json!("wireguard");
    }
    remember(st, "t:new:");
    issue(st, Cmd::CreateTunnel(body))
}
