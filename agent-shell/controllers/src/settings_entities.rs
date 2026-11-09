//! Settings record fields: persona, new account, MCP server, tunnel, module, file. Values come from the core lists
//! (`CoreModel`), not from page documents; the id is `p:<id|new>:<field>`, `a:new:<field>` and so on.

use serde_json::Value;

use crate::i18n::tr;
use crate::settings_fields::{Spec, Ty};
use crate::settings_model::{CoreModel, id_of, text_of};
pub use crate::settings_new::{
    ACCOUNT_TYPES, FILE_LIMIT, TUNNEL_TYPES, account_new_specs, file_text_spec, mcp_new_specs,
    module_url_spec, tunnel_new_specs,
};
use crate::settings_state::State;

pub(crate) fn entity(
    id: &str,
    doc: &str,
    path: &str,
    label: String,
    ty: Ty,
    default: &str,
) -> Spec {
    Spec {
        id: id.to_string(),
        doc: doc.to_string(),
        path: path.to_string(),
        label,
        ty,
        default: default.to_string(),
    }
}

fn ai_choices(m: &CoreModel) -> Vec<(String, String)> {
    let mut out = vec![(String::new(), tr("set-none"))];
    for a in m.ai_accounts() {
        if let Some(id) = id_of(a) {
            out.push((id.to_string(), text_of(a, "label").to_string()));
        }
    }
    out
}

/// Default colour of a new persona: the core requires a colour on creation.
pub const DEFAULT_COLOR: &str = "#808080";

/// Fields of one persona (`key` is the number or `new`).
pub fn persona_specs(key: &str, m: &CoreModel) -> Vec<Spec> {
    let doc = format!("persona:{key}");
    let f = |field: &str, label: String, ty: Ty, default: &str| {
        entity(&format!("p:{key}:{field}"), &doc, field, label, ty, default)
    };
    let mut v = vec![
        f("name", tr("set-f-name"), Ty::Text(80), ""),
        f(
            "color",
            tr("set-f-color"),
            Ty::Text(16),
            if key == "new" { DEFAULT_COLOR } else { "" },
        ),
        f(
            "lang",
            tr("set-f-language"),
            Ty::Choice(vec![("en".into(), tr("set-lang-en"))]),
            "en",
        ),
        f("style", tr("set-f-style"), Ty::Text(200), ""),
        f("signature", tr("set-f-signature"), Ty::Text(500), ""),
        f(
            "model_acc",
            tr("set-f-model-acc"),
            Ty::ChoiceId(ai_choices(m)),
            "",
        ),
        f(
            "mem_mode",
            tr("set-f-mem-mode"),
            Ty::Choice(vec![
                ("own".into(), tr("set-mem-own")),
                ("shared".into(), tr("set-mem-shared")),
                ("none".into(), tr("set-mem-none")),
            ]),
            "own",
        ),
        f(
            "notif_mode",
            tr("set-f-notif-mode"),
            Ty::Choice(vec![
                ("all".into(), tr("ss-notif-all")),
                ("important".into(), tr("ss-notif-important")),
                ("digest".into(), tr("set-notif-digest")),
                ("off".into(), tr("ss-notif-off")),
            ]),
            "all",
        ),
        f("work_hours", tr("set-f-work-hours"), Ty::Text(40), ""),
        f("quiet_hours", tr("set-f-quiet-hours"), Ty::Text(40), ""),
    ];
    for a in m.accounts.iter() {
        if let Some(id) = id_of(a) {
            v.push(f(
                &format!("acc:{id}"),
                text_of(a, "label").to_string(),
                Ty::Toggle,
                "",
            ));
        }
    }
    v
}

/// Fields of an installed module: placement and answers to the manifest's setup questions.
pub fn module_specs(module: &Value) -> Vec<Spec> {
    let id = text_of(module, "id");
    let doc = format!("module:{id}");
    let mut v = vec![entity(
        &format!("m:{id}:placement"),
        &doc,
        "placement",
        tr("set-f-placement"),
        Ty::Choice(vec![
            ("panel".into(), tr("set-pl-panel")),
            ("widget".into(), tr("set-pl-widget")),
            ("button".into(), tr("set-pl-button")),
        ]),
        "panel",
    )];
    for q in module["manifest"]["questions"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let (key, label) = match q {
            Value::String(s) => (s.as_str(), s.as_str()),
            Value::Object(_) => {
                let key = q.get("key").or_else(|| q.get("id")).and_then(Value::as_str);
                let Some(key) = key else { continue };
                (key, q.get("label").and_then(Value::as_str).unwrap_or(key))
            }
            _ => continue,
        };
        if key.is_empty() || key.len() > 40 || key.contains(':') {
            continue;
        }
        v.push(entity(
            &format!("m:{id}:cfg:{key}"),
            &doc,
            &format!("cfg:{key}"),
            label.to_string(),
            Ty::Text(200),
            "",
        ));
    }
    v
}

/// Value of a record field from the core lists.
pub fn show_entity(s: &Spec, m: &CoreModel) -> String {
    if let Some(key) = s.doc.strip_prefix("persona:") {
        let persona = key.parse::<i64>().ok().and_then(|id| m.persona(id));
        if let Some(aid) = s.path.strip_prefix("acc:") {
            let has = persona
                .and_then(|p| p.get("account_ids"))
                .and_then(Value::as_array)
                .is_some_and(|a| {
                    a.iter()
                        .any(|x| x.as_i64().map(|n| n.to_string()).as_deref() == Some(aid))
                });
            return has.to_string();
        }
        return match persona.and_then(|p| p.get(&s.path)) {
            Some(Value::String(t)) => t.clone(),
            Some(Value::Number(n)) => n.to_string(),
            _ => s.default.clone(),
        };
    }
    if let Some(id) = s.doc.strip_prefix("module:") {
        let module = m.modules.iter().find(|x| text_of(x, "id") == id);
        if let Some(key) = s.path.strip_prefix("cfg:") {
            return module
                .and_then(|x| x["config"].get(key))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
        }
        return module
            .map(|x| text_of(x, &s.path).to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| s.default.clone());
    }
    s.default.clone()
}

/// Spec of a record field by id.
pub fn entity_spec(st: &State, id: &str) -> Option<Spec> {
    let mut parts = id.splitn(3, ':');
    let (kind, key) = (parts.next()?, parts.next().unwrap_or(""));
    let found = |specs: Vec<Spec>| specs.into_iter().find(|s| s.id == id);
    match kind {
        "p" => found(persona_specs(key, &st.model)),
        "a" => found(account_new_specs()),
        "c" => found(mcp_new_specs()),
        "t" => found(tunnel_new_specs()),
        "m" if key == "url" => Some(module_url_spec()),
        "m" => {
            let module = st.model.modules.iter().find(|x| text_of(x, "id") == key)?;
            found(module_specs(module))
        }
        "f" if key == "text" => Some(file_text_spec()),
        _ => None,
    }
}
