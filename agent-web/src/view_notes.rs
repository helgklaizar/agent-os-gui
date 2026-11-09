//! Notifications column (`events` feed) and the Settings window (view of settings pages without secrets).

use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::core::{Core, Sig, request_popup_permission};
use crate::icons::Icon;
use crate::notes::{Filter, apply, kind_icon, parse_subtypes, stripe, unread_count};

fn kind_label(k: &str) -> String {
    match k {
        "ai" => "AI".to_string(),
        "task" => "Tasks".to_string(),
        "msg" => "Messages".to_string(),
        other => other.to_string(),
    }
}

fn unread_of(sig: Sig, kind: &str) -> usize {
    sig.notes
        .with(|l| l.iter().filter(|n| n.unread && n.kind == kind).count())
}

#[component]
pub fn Notes(sig: Sig, core: Rc<Core>) -> impl IntoView {
    let core = StoredValue::new_local(core);
    let unread = move || sig.notes.with(|l| unread_count(l));
    let kind = RwSignal::new_local(String::new());
    let only_unread = RwSignal::new_local(false);
    let kinds = RwSignal::new_local(vec![
        "ai".to_string(),
        "task".to_string(),
        "msg".to_string(),
    ]);
    // Subtypes are core data: they build the list of filter kinds (docs/reference/api.md section 5.3).
    spawn_local(async move {
        if let Ok(v) = core.get_value().get_json("/v1/events/subtypes").await {
            let mut ks: Vec<String> = Vec::new();
            for s in parse_subtypes(&v) {
                if !ks.contains(&s.kind) {
                    ks.push(s.kind);
                }
            }
            if !ks.is_empty() {
                kinds.set(ks);
            }
        }
    });
    let shown = move || {
        let f = Filter {
            kind: Some(kind.get()).filter(|k| !k.is_empty()),
            only_unread: only_unread.get(),
        };
        sig.notes.with(|l| apply(l, &f))
    };
    view! {
        <aside class="notes" aria-label="Notifications">
            <header class="col-head">
                <span class="head-ic"><Icon name="bell" size=20 /></span>
                <h2>"Notifications"</h2>
                <button
                    type="button"
                    class="ico-btn"
                    title="Mark all read"
                    aria-label="Mark all read"
                    prop:disabled=move || unread() == 0
                    on:click=move |_| core.get_value().read_all_notes()
                ><Icon name="check" /></button>
            </header>
            <div class="filter-all">
                <button
                    type="button"
                    class="fall"
                    class:on=move || kind.get().is_empty()
                    aria-pressed=move || kind.get().is_empty().to_string()
                    on:click=move |_| kind.set(String::new())
                >
                    "All"
                    <span class="badge bad" hidden=move || unread() == 0>{unread}</span>
                </button>
            </div>
            <div class="filter-kinds" role="group" aria-label="Event kind">
                {move || kinds.get().into_iter().map(|k| {
                    let (k2, k4) = (k.clone(), k.clone());
                    let ic = kind_icon(&k);
                    let label = kind_label(&k);
                    let (kc1, kc2) = (k.clone(), k.clone());
                    view! {
                        <button
                            type="button"
                            class="kindbtn"
                            class:on=move || kind.get() == k2
                            aria-pressed=move || (kind.get() == k4).to_string()
                            title=label.clone()
                            aria-label=label
                            on:click=move |_| kind.set(k.clone())
                        >
                            <Icon name=ic />
                            <span class="badge bad" hidden=move || unread_of(sig, &kc1) == 0>{move || unread_of(sig, &kc2)}</span>
                        </button>
                    }
                }).collect::<Vec<_>>()}
            </div>
            <div class="filters">
                <label class="chip-check">
                    <input type="checkbox" on:change=move |ev| only_unread.set(event_target_checked(&ev)) />
                    " Unread"
                </label>
                <label class="chip-check">
                    <input type="checkbox" prop:checked=move || sig.popups.get()
                        on:change=move |ev| {
                            if event_target_checked(&ev) {
                                request_popup_permission(sig);
                            } else {
                                sig.popups.set(false);
                            }
                        } />
                    " System notifications"
                </label>
            </div>
            <p class="empty" hidden=move || !shown().is_empty()>"No events"</p>
            <ul class="evs">
                <For
                    each=shown
                    key=|n| (n.id, n.unread)
                    children=move |n| {
                        let (id, sid) = (n.id, n.session_id);
                        let cls = format!("nt {} {}", stripe(&n.kind, &n.level), if n.unread { "unread" } else { "" });
                        let kind_text = kind_label(&n.kind);
                        view! {
                            <li>
                                <article class=cls>
                                    <span class="kind" data-kind=n.kind.clone()>{kind_text}</span>
                                    <span class="text">{n.text}</span>
                                    <div class="act">
                                        {n.unread.then(|| view! {
                                            <button type="button" class="ok-btn" aria-label="OK, mark read"
                                                on:click=move |_| core.get_value().read_note(id)>"OK"</button>
                                            <span class="tag-new">"new"</span>
                                        })}
                                        {sid.map(|_| view! {
                                            <button type="button" class="open" on:click=move |_| core.get_value().open_note(id, sid)>"Open chat"</button>
                                        })}
                                    </div>
                                </article>
                            </li>
                        }
                    }
                />
            </ul>
        </aside>
    }
}
