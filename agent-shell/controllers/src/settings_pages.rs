//! Settings page rows: Account, About, Models, Permissions, Security, Schedule, Indicators.
//! Pages with record lists (personas, connections, modules, tunnels, files) are in `settings_lists`.

use serde_json::Value;

use crate::bar_types::QuotaItem;
use crate::i18n::{tr, tr_args};
use crate::settings_fields::{Spec, page_specs};
use crate::settings_lists;
use crate::settings_model::{id_of, text_of};
use crate::settings_state::{State, field_row};
use crate::settings_types::Action;
use crate::settings_view::{FieldRow, ItemRow, Kind, Row, RowAction};
use crate::types::{SettingsPage, Theme};

pub(crate) fn heading(text: &str) -> Row {
    Row::Heading(text.to_string())
}

pub(crate) fn specs_rows(st: &State, page: SettingsPage) -> Vec<Row> {
    page_specs(page, &st.model)
        .iter()
        .map(|s| field_row(st, s))
        .collect()
}

fn theme_row(st: &State) -> Row {
    let options = [
        (Theme::Auto, tr("theme-auto")),
        (Theme::Dark, tr("theme-dark")),
        (Theme::Light, tr("theme-light")),
    ];
    let name = |t: Theme| match t {
        Theme::Auto => "auto",
        Theme::Dark => "dark",
        Theme::Light => "light",
    };
    Row::Field(FieldRow {
        id: "theme".into(),
        label: tr("set-f-theme"),
        kind: Kind::Choice(
            options
                .iter()
                .map(|(t, l)| (name(*t).to_string(), l.clone()))
                .collect(),
        ),
        value: name(st.theme).to_string(),
        error: None,
        dirty: false,
    })
}

fn read_only(label: String, value: String) -> Row {
    Row::Field(FieldRow {
        id: format!("ro:{label}"),
        label,
        kind: Kind::ReadOnly,
        value,
        error: None,
        dirty: false,
    })
}

fn about(st: &State) -> Vec<Row> {
    let doc = |k: &str| st.model.at("about", k).cloned().unwrap_or(Value::Null);
    let shown = |v: Value| match v {
        Value::String(s) => s,
        Value::Null => tr("set-unknown"),
        other => other.to_string(),
    };
    let api = doc("api");
    let api_text = format!(
        "{}.{}",
        api.get("major").and_then(Value::as_i64).unwrap_or(0),
        api.get("minor").and_then(Value::as_i64).unwrap_or(0)
    );
    let caps = doc("capabilities")
        .as_array()
        .map_or(0, Vec::len)
        .to_string();
    vec![
        read_only(tr("about-label-version"), shown(doc("core_version"))),
        read_only(tr("set-about-api"), api_text),
        read_only(tr("set-about-db"), shown(doc("db_version"))),
        read_only(tr("set-about-caps"), caps),
        read_only(tr("about-label-platform"), st.system.clone()),
        read_only(tr("about-label-config"), st.config.clone()),
    ]
}

fn local_model_row(q: &QuotaItem) -> Row {
    let load = q
        .load
        .map_or_else(|| tr("set-unknown"), |l| format!("{:.0}%", l * 100.0));
    Row::Item(ItemRow {
        title: q.label.clone(),
        subtitle: tr_args("set-local-load", &[("load", &load)]),
        tags: vec![tr("set-local-tag")],
        actions: Vec::new(),
        selected: false,
    })
}

fn models(st: &State) -> Vec<Row> {
    let mut rows = vec![heading(&tr("set-h-cloud"))];
    let ai = st.model.ai_accounts();
    if ai.is_empty() {
        rows.push(Row::Note(tr("set-no-providers")));
    }
    for a in ai {
        let Some(id) = id_of(a) else { continue };
        let reference = format!("account.{id}");
        let state = if st.model.secret_set(&reference) {
            tr("set-key-set")
        } else {
            tr("set-key-unset")
        };
        rows.push(Row::Item(ItemRow {
            title: text_of(a, "label").to_string(),
            subtitle: format!("{} · {state}", text_of(a, "type")),
            tags: Vec::new(),
            actions: Vec::new(),
            selected: false,
        }));
        rows.extend(
            crate::settings_fields::provider_specs(id)
                .iter()
                .map(|s| field_row(st, s)),
        );
        rows.push(Row::Field(FieldRow {
            id: format!("s:{reference}"),
            label: tr("set-f-api-key"),
            kind: Kind::Secret,
            value: String::new(),
            error: st.errors.get(&format!("s:{reference}")).cloned(),
            dirty: st.secrets.contains_key(&format!("s:{reference}")),
        }));
    }
    rows.push(heading(&tr("set-h-local")));
    let local = st.model.local_models();
    if local.is_empty() {
        rows.push(Row::Note(tr("set-no-local")));
    }
    rows.extend(local.into_iter().map(local_model_row));
    rows.push(heading(&tr("set-h-hardware")));
    let pct = |v: Option<f64>| v.map_or_else(|| tr("set-unknown"), |x| format!("{x:.0}%"));
    rows.push(read_only(tr("set-hw-cpu"), pct(st.model.sys.cpu)));
    rows.push(read_only(tr("set-hw-ram"), pct(st.model.sys.ram)));
    rows.push(heading(&tr("set-h-roles")));
    let rest: Vec<Spec> = page_specs(SettingsPage::Models, &st.model)
        .into_iter()
        .filter(|s| s.doc != "models" || s.path == "context_budget")
        .collect();
    rows.extend(rest.iter().map(|s| field_row(st, s)));
    rows
}

fn indicator_items(st: &State) -> Vec<Value> {
    st.indicators.clone().unwrap_or_else(|| {
        st.model
            .at("indicators", "items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    })
}

/// Indicator items in `position` order.
pub fn sorted_indicators(st: &State) -> Vec<Value> {
    let mut items = indicator_items(st);
    items.sort_by_key(|i| {
        i.get("position")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX)
    });
    items
}

fn indicators(st: &State) -> Vec<Row> {
    let items = sorted_indicators(st);
    let last = items.len().saturating_sub(1);
    let mut rows = vec![Row::Note(tr("set-ind-note"))];
    for (i, it) in items.iter().enumerate() {
        let id = text_of(it, "id").to_string();
        let on = it.get("enabled").and_then(Value::as_bool).unwrap_or(false);
        let act = |action: Action, label: String| RowAction {
            action,
            label,
            destructive: false,
        };
        let mut actions = vec![act(
            Action::IndicatorToggle(id.clone()),
            if on {
                tr("set-ind-off")
            } else {
                tr("set-ind-on")
            },
        )];
        if i > 0 {
            actions.push(act(Action::IndicatorUp(id.clone()), tr("set-ind-up")));
        }
        if i < last {
            actions.push(act(Action::IndicatorDown(id.clone()), tr("set-ind-down")));
        }
        rows.push(Row::Item(ItemRow {
            title: id,
            subtitle: if on {
                tr("set-ind-state-on")
            } else {
                tr("set-ind-state-off")
            },
            tags: Vec::new(),
            actions,
            selected: false,
        }));
    }
    rows
}

fn security(st: &State) -> Vec<Row> {
    let doc = |k: &str| st.model.at("security", k).cloned();
    let text = |v: Option<Value>| match v {
        Some(Value::String(s)) => s,
        Some(Value::Bool(true)) => tr("set-yes"),
        Some(Value::Bool(false)) => tr("set-no"),
        Some(other) => other.to_string(),
        None => tr("set-unknown"),
    };
    let mut rows = vec![
        read_only(tr("set-sec-ptrace"), text(doc("ptrace_scope"))),
        read_only(tr("set-sec-disk"), text(doc("disk_encryption"))),
        read_only(tr("set-sec-secrets"), text(doc("secrets_present"))),
    ];
    rows.extend(specs_rows(st, SettingsPage::Security));
    rows
}

/// Rows of the current page.
pub fn rows(st: &State) -> Vec<Row> {
    match st.page() {
        SettingsPage::Account => {
            let mut r = specs_rows(st, SettingsPage::Account);
            r.insert(1, theme_row(st));
            r
        }
        SettingsPage::About => about(st),
        SettingsPage::Models => models(st),
        SettingsPage::Permissions => crate::settings_perms::permissions(st),
        SettingsPage::Security => security(st),
        SettingsPage::Schedule => {
            let mut r = vec![Row::Note(tr("set-tz-note"))];
            r.extend(specs_rows(st, SettingsPage::Schedule));
            r
        }
        SettingsPage::Indicators => indicators(st),
        SettingsPage::Personas => settings_lists::personas(st),
        SettingsPage::Connections => settings_lists::connections(st),
        SettingsPage::Modules => settings_lists::modules(st),
        SettingsPage::Tunnels => settings_lists::tunnels(st),
        SettingsPage::Files => settings_lists::files(st),
        SettingsPage::Root => Vec::new(),
    }
}

/// Field specification by identifier across all pages (for input parsing and saving).
pub fn spec_by_id(st: &State, id: &str) -> Option<Spec> {
    use SettingsPage::*;
    let pages = [
        Account,
        Personas,
        Models,
        Security,
        Schedule,
        Permissions,
        Tunnels,
    ];
    pages
        .into_iter()
        .flat_map(|p| page_specs(p, &st.model))
        .find(|s| s.id == id)
        .or_else(|| settings_lists::entity_spec(st, id))
}
