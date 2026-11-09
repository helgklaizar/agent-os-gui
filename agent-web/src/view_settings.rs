//! Settings window: core pages `GET /v1/settings/{page}`, editing safe pages via `PATCH` with `If-Match`.
//! The core never returns secret values, the client neither shows nor writes them; pages outside the web token scope
//! stay view-only with an explanation. Text enters the page only as text nodes and field values.

use std::collections::BTreeMap;
use std::rc::Rc;

use leptos::prelude::*;
use serde_json::Value;
use wasm_bindgen_futures::spawn_local;

use crate::core::Core;
use crate::settings::{self, Field, FieldErr, Kind};

const PAGES: [(&str, &str); 12] = [
    ("about", "About"),
    ("account", "Account"),
    ("personas", "Personas"),
    ("models", "Models"),
    ("memory", "Memory"),
    ("connections", "Connections"),
    ("permissions", "Permissions"),
    ("security", "Security"),
    ("schedule", "Schedule"),
    ("tunnels", "Tunnels"),
    ("indicators", "Indicators"),
    ("theme", "Theme"),
];

type Drafts = RwSignal<BTreeMap<String, String>, LocalStorage>;

#[allow(clippy::too_many_lines)]
#[component]
pub fn Settings(core: Rc<Core>, on_close: impl Fn() + Copy + 'static) -> impl IntoView {
    let core = StoredValue::new_local(core);
    let page = RwSignal::new_local(PAGES[0].0);
    let doc = RwSignal::new_local(Option::<Value>::None);
    let rev = RwSignal::new_local(Option::<i64>::None);
    let drafts: Drafts = RwSignal::new_local(BTreeMap::new());
    let errors = RwSignal::new_local(Vec::<FieldErr>::new());
    let msg = RwSignal::new_local(String::from("Loading…"));
    let busy = RwSignal::new_local(false);

    let load = move || {
        let p = page.get_untracked();
        let c = core.get_value();
        errors.set(Vec::new());
        spawn_local(async move {
            match c.get_doc(&format!("/v1/settings/{p}")).await {
                Ok((d, r)) => {
                    if page.get_untracked() != p {
                        return;
                    }
                    let f = settings::fields(p, &d);
                    drafts.set(f.into_iter().map(|x| (x.key, x.text)).collect());
                    rev.set(r);
                    doc.set(Some(d));
                    msg.set(String::new());
                }
                Err(e) => {
                    if page.get_untracked() == p {
                        doc.set(None);
                        msg.set(format!("Page unavailable: {e}"));
                    }
                }
            }
        });
    };
    Effect::new(move |_| {
        page.track();
        doc.set(None);
        msg.set("Loading…".into());
        load();
    });

    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let p = page.get_untracked();
        let (Some(d), Some(r)) = (doc.get_untracked(), rev.get_untracked()) else {
            return;
        };
        let edits: Vec<(String, String)> =
            drafts.with_untracked(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        let patch = match settings::build_patch(&d, &edits) {
            Ok(p) => p,
            Err(e) => {
                errors.set(e);
                msg.set("Fix the marked fields".into());
                return;
            }
        };
        if patch.as_object().is_some_and(serde_json::Map::is_empty) {
            errors.set(Vec::new());
            msg.set("No changes".into());
            return;
        }
        errors.set(Vec::new());
        busy.set(true);
        msg.set("Saving…".into());
        let c = core.get_value();
        spawn_local(async move {
            let res = c.patch_doc(&format!("/v1/settings/{p}"), r, &patch).await;
            busy.set(false);
            if page.get_untracked() != p {
                return;
            }
            match res {
                Ok((nd, nr)) => {
                    let f = settings::fields(p, &nd);
                    drafts.set(f.into_iter().map(|x| (x.key, x.text)).collect());
                    rev.set(nr);
                    doc.set(Some(nd));
                    msg.set("Saved".into());
                }
                Err(f) => {
                    errors.set(f.fields);
                    msg.set(f.text);
                    if f.status == 412 {
                        load();
                    }
                }
            }
        });
    };

    let fields_view = move || {
        let p = page.get();
        let can_edit = settings::editable(p);
        doc.get()
            .map(|d| settings::fields(p, &d))
            .unwrap_or_default()
            .into_iter()
            .map(|f| field_view(f, can_edit, drafts, errors))
            .collect::<Vec<_>>()
    };

    view! {
        <div class="modal" role="dialog" aria-modal="true" aria-label="Settings">
            <div class="modal-body">
                <header>
                    <h2>"Settings"</h2>
                    <button type="button" aria-label="Close" on:click=move |_| on_close()>"✕"</button>
                </header>
                <div class="tabs" role="tablist" aria-label="Settings pages">
                    {PAGES
                        .iter()
                        .map(|(id, label)| {
                            let id = *id;
                            view! {
                                <button
                                    type="button"
                                    role="tab"
                                    aria-selected=move || (page.get() == id).to_string()
                                    class:active=move || page.get() == id
                                    on:click=move |_| page.set(id)
                                >{*label}</button>
                            }
                        })
                        .collect::<Vec<_>>()}
                </div>
                <p class="hint">
                    {move || if settings::editable(page.get()) {
                        "Editable via API: the core applies changes. Secrets are set locally and are not shown here."
                    } else {
                        "View only: this page is edited locally (native GUI or CLI)."
                    }}
                </p>
                <form class="settings" on:submit=save aria-label="Page fields">
                    {fields_view}
                    <div class="card-actions" hidden=move || !settings::editable(page.get()) || doc.with(Option::is_none)>
                        <button type="submit" class="card-btn" prop:disabled=move || busy.get()>"Save"</button>
                        <button type="button" class="card-btn" on:click=move |_| load()>"Reset"</button>
                    </div>
                </form>
                <p class="hint" role="status">{move || msg.get()}</p>
            </div>
        </div>
    }
}

fn field_view(
    f: Field,
    can_edit: bool,
    drafts: Drafts,
    errors: RwSignal<Vec<FieldErr>, LocalStorage>,
) -> AnyView {
    let key = f.key.clone();
    let id = format!("f-{}", f.key);
    let disabled = !can_edit || f.locked;
    let value = {
        let key = key.clone();
        move || drafts.with(|m| m.get(&key).cloned().unwrap_or_default())
    };
    let set = {
        let key = key.clone();
        move |v: String| drafts.update(|m| drop(m.insert(key.clone(), v)))
    };
    let err = {
        let key = key.clone();
        move || {
            errors.with(|e| {
                e.iter()
                    .find(|x| {
                        x.field == key
                            || x.field.starts_with(&format!("{key}."))
                            || x.field.starts_with(&format!("{key}["))
                    })
                    .map(|x| x.message.clone())
            })
        }
    };
    let err2 = err.clone();
    let err_id = format!("e-{}", f.key);
    let control: AnyView = if f.locked {
        let id = id.clone();
        view! { <input id=id type="text" value="••• set locally" disabled=true /> }.into_any()
    } else {
        match f.kind {
            Kind::Bool => {
                let v = value.clone();
                let s = set.clone();
                view! {
                    <input
                        id=id.clone()
                        type="checkbox"
                        disabled=disabled
                        prop:checked=move || v() == "true"
                        on:change=move |ev| s(event_target_checked(&ev).to_string())
                    />
                }
                .into_any()
            }
            Kind::Choice(opts) => {
                let v = value.clone();
                let s = set.clone();
                view! {
                    <select
                        id=id.clone()
                        disabled=disabled
                        prop:value=value.clone()
                        on:change=move |ev| s(event_target_value(&ev))
                    >
                        {opts
                            .into_iter()
                            .map(|o| {
                                let v = v.clone();
                                view! { <option value=o selected=move || v() == o>{o}</option> }
                            })
                            .collect::<Vec<_>>()}
                    </select>
                }
                .into_any()
            }
            Kind::Json => {
                let s = set.clone();
                view! {
                    <textarea
                        id=id.clone()
                        rows="6"
                        disabled=disabled
                        aria-describedby=err_id.clone()
                        prop:value=value.clone()
                        on:input=move |ev| s(event_target_value(&ev))
                    ></textarea>
                }
                .into_any()
            }
            Kind::Number | Kind::Text => {
                let s = set.clone();
                view! {
                    <input
                        id=id.clone()
                        type="text"
                        disabled=disabled
                        aria-describedby=err_id.clone()
                        prop:value=value.clone()
                        on:input=move |ev| s(event_target_value(&ev))
                    />
                }
                .into_any()
            }
        }
    };
    view! {
        <div class="field">
            <label for=id>{f.key.clone()}</label>
            {control}
            <p class="field-err" id=err_id role="alert" hidden=move || err2().is_none()>{move || err().unwrap_or_default()}</p>
        </div>
    }
    .into_any()
}
