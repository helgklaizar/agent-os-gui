//! Saving and core responses: the "Save" button collects drafts into commands; an error is shown next to the field
//! or as a banner, the input stays, and saving can be retried. Nothing disappears silently.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::controller::{CoreCommand, Effect};
use crate::i18n::tr;
use crate::permissions::{Mode, default_mode};
use crate::settings_fields::{merge, nest, parse};
use crate::settings_model::{id_of, text_of};
use crate::settings_pages::{sorted_indicators, spec_by_id};
use crate::settings_perms::perm_mode;
use crate::settings_state::State;
use crate::settings_types::{Cmd, SaveError};
use crate::settings_view::{Banner, BannerKind};

pub fn cmd(c: Cmd) -> Effect {
    Effect::Command(CoreCommand::Settings(c))
}

pub fn banner(kind: BannerKind, text: String) -> Banner {
    Banner { kind, text }
}

/// Banner text for a core response: code and fields, without core texts and paths.
pub fn error_banner(e: &SaveError) -> Banner {
    let text = match e.status {
        412 | 428 => tr("set-err-stale"),
        422 => tr("set-err-invalid"),
        403 => tr("set-err-forbidden"),
        0 | 500..=599 => tr("set-err-unavailable"),
        _ => tr("set-err-failed"),
    };
    banner(BannerKind::Error, text)
}

fn sent(st: &mut State, id: &str, value: &str) {
    st.sent.insert(id.to_string(), value.to_string());
}

/// The "Save" command series. An empty list means there is nothing to save or the input failed validation (errors are already on the fields).
pub fn save(st: &mut State) -> Vec<Effect> {
    if st.pending > 0 {
        return Vec::new();
    }
    st.errors.clear();
    st.failed = false;
    let mut patches: BTreeMap<String, Value> = BTreeMap::new();
    let mut out = Vec::new();
    let mut persona_bodies: BTreeMap<i64, Value> = BTreeMap::new();
    let mut module_bodies: BTreeMap<String, Value> = BTreeMap::new();
    let mut invalid = false;
    let drafts: Vec<(String, String)> = st
        .text
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (id, text) in &drafts {
        if id.starts_with("perm:") {
            continue;
        }
        let Some(spec) = spec_by_id(st, id) else {
            continue;
        };
        let value = match parse(&spec, text) {
            Ok(v) => v,
            Err(bad) => {
                st.errors.insert(id.clone(), bad.text());
                invalid = true;
                continue;
            }
        };
        if let Some(doc) = id.strip_prefix("d:").and_then(|r| r.split(':').next()) {
            merge(
                patches.entry(doc.to_string()).or_insert_with(|| json!({})),
                nest(&spec.path, value),
            );
            sent(st, id, text);
        } else if let Some(key) = spec.doc.strip_prefix("persona:") {
            let Ok(pid) = key.parse::<i64>() else {
                continue;
            };
            let body = persona_bodies.entry(pid).or_insert_with(|| json!({}));
            if let Some(aid) = spec.path.strip_prefix("acc:") {
                let mut ids = account_ids(st, pid);
                let aid: i64 = aid.parse().unwrap_or(-1);
                ids.retain(|x| *x != aid);
                if value == json!(true) {
                    ids.push(aid);
                }
                ids.sort_unstable();
                body["account_ids"] = json!(ids);
            } else {
                body[spec.path.as_str()] = value;
            }
            sent(st, id, text);
        } else if let Some(mid) = spec.doc.strip_prefix("module:") {
            let body = module_bodies
                .entry(mid.to_string())
                .or_insert_with(|| json!({}));
            if let Some(key) = spec.path.strip_prefix("cfg:") {
                if body.get("config").is_none() {
                    body["config"] = st
                        .model
                        .modules
                        .iter()
                        .find(|m| text_of(m, "id") == mid)
                        .map_or_else(|| json!({}), |m| m["config"].clone());
                }
                body["config"][key] = value;
            } else {
                body[spec.path.as_str()] = value;
            }
            sent(st, id, text);
        } else if id == "f:text"
            && let Some(file) = &st.file
        {
            out.push(cmd(Cmd::PutFile {
                path: file.path.clone(),
                rev: file.rev.clone(),
                text: text.clone(),
            }));
            sent(st, id, text);
        }
    }
    if invalid {
        st.banner = Some(banner(BannerKind::Error, tr("set-err-invalid")));
        return Vec::new();
    }
    for (id, secret) in st.secrets.clone() {
        let Some(reference) = id.strip_prefix("s:") else {
            continue;
        };
        out.push(cmd(Cmd::PutSecret {
            reference: reference.to_string(),
            value: secret,
        }));
        if let Some(n) = reference.strip_prefix("account.") {
            merge(
                patches.entry("models".into()).or_insert_with(|| json!({})),
                nest(&format!("accounts.{n}.secret_ref"), json!(reference)),
            );
        }
    }
    if let Some(items) = &st.indicators {
        let list = renumber(items);
        patches.insert("indicators".into(), json!({ "items": list }));
    }
    for (doc, patch) in patches {
        out.push(cmd(Cmd::PatchPage {
            rev: st.model.rev(&doc),
            doc,
            patch,
        }));
    }
    for (id, body) in persona_bodies {
        out.push(cmd(Cmd::PatchPersona { id, body }));
    }
    for (id, body) in module_bodies {
        out.push(cmd(Cmd::PatchModule { id, body }));
    }
    out.extend(perm_commands(st));
    st.pending = out.len();
    if out.is_empty() {
        st.banner = Some(banner(BannerKind::Info, tr("set-nothing")));
    } else {
        st.banner = None;
    }
    out
}

fn account_ids(st: &State, persona: i64) -> Vec<i64> {
    st.model
        .persona(persona)
        .and_then(|p| p.get("account_ids"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

fn renumber(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let mut it = it.clone();
            it["position"] = json!(i);
            it
        })
        .collect()
}

fn perm_commands(st: &mut State) -> Vec<Effect> {
    let ids: Vec<String> = st
        .text
        .keys()
        .filter(|k| k.starts_with("perm:"))
        .cloned()
        .collect();
    let persona = st
        .perm_persona
        .or_else(|| st.model.personas.first().and_then(id_of));
    let mut out = Vec::new();
    for id in ids {
        let mut parts = id.splitn(3, ':');
        let (_, level, action) = (
            parts.next(),
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
        );
        let Some(mode) = st.text.get(&id).and_then(|t| Mode::parse(t)) else {
            continue;
        };
        let scope_id = match level {
            "global" => 0,
            _ => match persona {
                Some(p) => p,
                None => continue,
            },
        };
        let sent_value = mode.as_str().to_string();
        st.sent.insert(id.clone(), sent_value);
        out.push(cmd(Cmd::PutPermission {
            scope: level.to_string(),
            scope_id,
            action: action.to_string(),
            mode,
        }));
    }
    out
}

/// Indicator list for editing: the draft or the order from the core.
pub fn indicator_draft(st: &mut State) -> &mut Vec<Value> {
    if st.indicators.is_none() {
        st.indicators = Some(sorted_indicators(st));
    }
    st.indicators.get_or_insert_with(Vec::new)
}

/// Permission mode in the matrix, for the weakening check.
pub fn current_mode(st: &State, level: &str, scope_id: i64, action: &str) -> Mode {
    perm_mode(st, level, scope_id, action).unwrap_or_else(|| default_mode(action))
}
