//! Schedule (tasks), Search (Ctrl+K) and Agent terminal (view) windows. Text goes only into text nodes.

use std::rc::Rc;

use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use crate::core::{Core, Sig};
use crate::term::Span;
use crate::tools::{
    ActionRow, Hit, RunRow, TaskRow, build_task_create, build_task_patch, parse_actions,
    parse_runs, parse_search, parse_tasks, toggled_state,
};
use crate::util::fmt_ts;

#[component]
fn Modal(
    title: &'static str,
    on_close: impl Fn() + Copy + 'static,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="modal" role="dialog" aria-modal="true" aria-label=title>
            <div class="modal-body">
                <header>
                    <h2>{title}</h2>
                    <button type="button" aria-label="Close" on:click=move |_| on_close()>"✕"</button>
                </header>
                {children()}
            </div>
        </div>
    }
}

fn state_label(s: &str) -> &'static str {
    match s {
        "active" => "active",
        "paused" => "paused",
        "done" => "done",
        _ => "—",
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    Closed,
    Create,
    Edit(i64),
}

fn field_msg(errs: &[(String, String)], name: &str) -> Option<String> {
    errs.iter().find(|(f, _)| f == name).map(|(_, m)| m.clone())
}

#[allow(clippy::too_many_lines)]
#[component]
pub fn Tasks(core: Rc<Core>, on_close: impl Fn() + Copy + 'static) -> impl IntoView {
    let core = StoredValue::new_local(core);
    let rows = RwSignal::new_local(Vec::<TaskRow>::new());
    let runs = RwSignal::new_local(Option::<(i64, Vec<RunRow>)>::None);
    let msg = RwSignal::new_local(String::from("Loading…"));
    let form = RwSignal::new_local(Form::Closed);
    let f_kind = RwSignal::new_local(String::from("sched"));
    let f_title = RwSignal::new_local(String::new());
    let f_spec = RwSignal::new_local(String::new());
    let f_cron = RwSignal::new_local(String::new());
    let f_errs = RwSignal::new_local(Vec::<(String, String)>::new());
    let reload = move || {
        let c = core.get_value();
        spawn_local(async move {
            match c.get_json("/v1/tasks").await {
                Ok(v) => {
                    let r = parse_tasks(&v);
                    msg.set(if r.is_empty() {
                        "No tasks".into()
                    } else {
                        String::new()
                    });
                    rows.set(r);
                }
                Err(e) => msg.set(format!("Schedule unavailable: {e}")),
            }
        });
    };
    reload();
    let act = move |id: i64, method: &'static str, path: String, body: serde_json::Value| {
        let c = core.get_value();
        spawn_local(async move {
            if let Err(e) = c.call(method, &path, Some(body)).await {
                c.notice(&format!("Task {id} action failed: {e}"));
            }
            reload();
        });
    };
    let show_runs = move |id: i64| {
        let c = core.get_value();
        spawn_local(async move {
            let list = c
                .get_json(&format!("/v1/tasks/{id}/runs"))
                .await
                .map(|v| parse_runs(&v))
                .unwrap_or_default();
            runs.set(Some((id, list)));
        });
    };
    let open_create = move |_| {
        f_kind.set("sched".into());
        f_title.set(String::new());
        f_spec.set(String::new());
        f_cron.set(String::new());
        f_errs.set(Vec::new());
        form.set(Form::Create);
    };
    let open_edit = move |t: TaskRow| {
        f_kind.set(t.kind.clone());
        f_title.set(t.title.clone());
        f_spec.set(t.spec.clone());
        f_cron.set(t.cron.clone().unwrap_or_default());
        f_errs.set(Vec::new());
        form.set(Form::Edit(t.id));
    };
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let (kind, title, spec, cron) = (
            f_kind.get_untracked(),
            f_title.get_untracked(),
            f_spec.get_untracked(),
            f_cron.get_untracked(),
        );
        let c = core.get_value();
        match form.get_untracked() {
            Form::Closed => {}
            Form::Create => {
                let Some(sid) = c.selected() else {
                    f_errs.set(vec![(
                        "kind".into(),
                        "Open a session first: the task is created in it".into(),
                    )]);
                    return;
                };
                match build_task_create(&kind, &title, &spec, &cron) {
                    Err(e) => f_errs.set(e),
                    Ok(body) => spawn_local(async move {
                        match c
                            .call("POST", &format!("/v1/sessions/{sid}/tasks"), Some(body))
                            .await
                        {
                            Ok(()) => {
                                form.set(Form::Closed);
                                reload();
                            }
                            Err(e) => f_errs.set(vec![("title".into(), e)]),
                        }
                    }),
                }
            }
            Form::Edit(id) => {
                let Some(task) = rows.get_untracked().into_iter().find(|t| t.id == id) else {
                    return;
                };
                match build_task_patch(&task, &title, &spec, &cron) {
                    Err(e) => f_errs.set(e),
                    Ok(body) if body.as_object().is_some_and(serde_json::Map::is_empty) => {
                        form.set(Form::Closed);
                    }
                    Ok(body) => spawn_local(async move {
                        match c
                            .call("PATCH", &format!("/v1/tasks/{id}"), Some(body))
                            .await
                        {
                            Ok(()) => {
                                form.set(Form::Closed);
                                reload();
                            }
                            Err(e) => f_errs.set(vec![("title".into(), e)]),
                        }
                    }),
                }
            }
        }
    };
    view! {
        <Modal title="Schedule" on_close=on_close>
            <div class="card-actions">
                <button type="button" class="card-btn" on:click=open_create>"New task"</button>
            </div>
            <form class="taskform" hidden=move || form.get() == Form::Closed on:submit=submit aria-label="Task">
                <label for="t-kind">"Kind"</label>
                <select id="t-kind" prop:value=move || f_kind.get() prop:disabled=move || form.get() != Form::Create
                    on:change=move |ev| f_kind.set(event_target_value(&ev))>
                    <option value="sched">"Scheduled"</option>
                    <option value="deleg">"Delegated"</option>
                </select>
                <p class="field-err" role="alert" hidden=move || field_msg(&f_errs.get(), "kind").is_none()>{move || field_msg(&f_errs.get(), "kind").unwrap_or_default()}</p>
                <label for="t-title">"Title"</label>
                <input id="t-title" type="text" prop:value=move || f_title.get() on:input=move |ev| f_title.set(event_target_value(&ev)) />
                <p class="field-err" role="alert" hidden=move || field_msg(&f_errs.get(), "title").is_none()>{move || field_msg(&f_errs.get(), "title").unwrap_or_default()}</p>
                <label for="t-spec">"Description"</label>
                <textarea id="t-spec" rows="3" prop:value=move || f_spec.get() on:input=move |ev| f_spec.set(event_target_value(&ev))></textarea>
                <p class="field-err" role="alert" hidden=move || field_msg(&f_errs.get(), "spec").is_none()>{move || field_msg(&f_errs.get(), "spec").unwrap_or_default()}</p>
                <label for="t-cron" hidden=move || f_kind.get() != "sched">"Schedule (cron)"</label>
                <input id="t-cron" type="text" placeholder="0 8 * * *" hidden=move || f_kind.get() != "sched"
                    prop:value=move || f_cron.get() on:input=move |ev| f_cron.set(event_target_value(&ev)) />
                <p class="field-err" role="alert" hidden=move || field_msg(&f_errs.get(), "cron").is_none()>{move || field_msg(&f_errs.get(), "cron").unwrap_or_default()}</p>
                <div class="card-actions">
                    <button type="submit" class="card-btn">"Save"</button>
                    <button type="button" class="card-btn" on:click=move |_| form.set(Form::Closed)>"Cancel"</button>
                </div>
            </form>
            <p class="hint" role="status">{move || msg.get()}</p>
            <ul class="tasks">
                <For
                    each=move || rows.get()
                    key=|t| (t.id, t.title.clone(), t.spec.clone(), t.cron.clone(), t.state.clone(), t.next_run)
                    children=move |t| {
                        let id = t.id;
                        let next = t.state.clone();
                        let toggle = toggled_state(&t.state);
                        let edit_row = t.clone();
                        view! {
                            <li class="task">
                                <div>
                                    <strong>{t.title}</strong>
                                    <span class="meta">{format!(" · {} · {}", t.cron.unwrap_or_else(|| t.kind.clone()), state_label(&next))}</span>
                                    {t.next_run.map(|n| view! { <span class="meta">{format!(" · next {}", fmt_ts(n))}</span> })}
                                </div>
                                <div class="card-actions">
                                    {toggle.map(|to| view! {
                                        <button type="button" class="card-btn"
                                            on:click=move |_| act(id, "PATCH", format!("/v1/tasks/{id}"), json!({"state": to}))>
                                            {if to == "paused" { "Pause" } else { "Resume" }}
                                        </button>
                                    })}
                                    <button type="button" class="card-btn"
                                        on:click=move |_| act(id, "POST", format!("/v1/tasks/{id}/run"), json!({}))>"Run"</button>
                                    <button type="button" class="card-btn" on:click=move |_| open_edit(edit_row.clone())>"Edit"</button>
                                    <button type="button" class="card-btn" on:click=move |_| show_runs(id)>"History"</button>
                                    <button type="button" class="card-btn"
                                        aria-label=format!("Delete task {id} together with its run history")
                                        on:click=move |_| {
                                            let ok = web_sys::window()
                                                .and_then(|w| w.confirm_with_message("Delete the task and its run history?").ok())
                                                .unwrap_or(false);
                                            if ok {
                                                act(id, "DELETE", format!("/v1/tasks/{id}"), serde_json::Value::Null);
                                            }
                                        }>"Delete"</button>
                                </div>
                            </li>
                        }
                    }
                />
            </ul>
            {move || runs.get().map(|(id, list)| view! {
                <h3>{format!("Runs of task {id}")}</h3>
                <p class="hint" hidden=!list.is_empty()>"No runs"</p>
                <ul class="runs">
                    {list.into_iter().map(|r| view! {
                        <li><span class="meta">{format!("{} · {}", fmt_ts(r.started_at), r.status)}</span><pre class="doc">{r.output}</pre></li>
                    }).collect::<Vec<_>>()}
                </ul>
            })}
        </Modal>
    }
}

/// Agent action log (`GET /v1/actions`). If the core does not publish the route (`404`), the window says so plainly.
#[component]
pub fn Journal(core: Rc<Core>, on_close: impl Fn() + Copy + 'static) -> impl IntoView {
    let core = StoredValue::new_local(core);
    let rows = RwSignal::new_local(Vec::<ActionRow>::new());
    let cursor = RwSignal::new_local(Option::<String>::None);
    let msg = RwSignal::new_local(String::from("Loading…"));
    let actor = RwSignal::new_local(String::new());
    let load = move |more: bool| {
        let c = core.get_value();
        let mut path = String::from("/v1/actions?");
        let a = actor.get_untracked();
        if !a.is_empty() {
            path.push_str(&format!("actor={a}&"));
        }
        if more && let Some(cur) = cursor.get_untracked() {
            let enc: String = js_sys::encode_uri_component(&cur).into();
            path.push_str(&format!("cursor={enc}&"));
        }
        spawn_local(async move {
            match c.get_json(path.trim_end_matches(['?', '&'])).await {
                Ok(v) => {
                    let (list, next) = parse_actions(&v);
                    msg.set(if list.is_empty() && !more {
                        "No entries".into()
                    } else {
                        String::new()
                    });
                    rows.update(|r| {
                        if more {
                            r.extend(list);
                        } else {
                            *r = list;
                        }
                    });
                    cursor.set(next);
                }
                Err(e) => {
                    rows.set(Vec::new());
                    cursor.set(None);
                    msg.set(format!("Log unavailable: {e}"));
                }
            }
        });
    };
    load(false);
    view! {
        <Modal title="Action log" on_close=on_close>
            <div class="search">
                <label for="j-actor">"Actor"</label>
                <select id="j-actor" prop:value=move || actor.get()
                    on:change=move |ev| { actor.set(event_target_value(&ev)); load(false); }>
                    <option value="">"all"</option>
                    <option value="agent">"agent"</option>
                    <option value="user">"user"</option>
                    <option value="cron">"schedule"</option>
                    <option value="observer">"observer"</option>
                    <option value="module">"module"</option>
                </select>
            </div>
            <p class="hint" role="status">{move || msg.get()}</p>
            <ul class="tasks">
                <For
                    each=move || rows.get()
                    key=|r| (r.id, r.decision.clone(), r.result.clone())
                    children=move |r| view! {
                        <li class="task">
                            <div>
                                <strong>{r.action}</strong>
                                <code class="target">{r.target}</code>
                                <span class="meta">{format!("{} · {} · {} {}", fmt_ts(r.ts), r.actor, r.decision, r.result)}</span>
                            </div>
                        </li>
                    }
                />
            </ul>
            <div class="card-actions" hidden=move || cursor.get().is_none()>
                <button type="button" class="card-btn" on:click=move |_| load(true)>"More"</button>
            </div>
        </Modal>
    }
}

#[component]
pub fn Search(core: Rc<Core>, on_close: impl Fn() + Copy + 'static) -> impl IntoView {
    let core = StoredValue::new_local(core);
    let q = RwSignal::new_local(String::new());
    let hits = RwSignal::new_local(Vec::<Hit>::new());
    let msg = RwSignal::new_local(String::new());
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let text = q.get_untracked().trim().to_string();
        if text.is_empty() {
            return;
        }
        let enc: String = js_sys::encode_uri_component(&text).into();
        let c = core.get_value();
        msg.set("Searching…".into());
        spawn_local(async move {
            match c.get_json(&format!("/v1/search?q={enc}&limit=20")).await {
                Ok(v) => {
                    let h = parse_search(&v);
                    msg.set(if h.is_empty() {
                        "Nothing found".into()
                    } else {
                        String::new()
                    });
                    hits.set(h);
                }
                Err(e) => msg.set(format!("Search unavailable: {e}")),
            }
        });
    };
    let close_sv = StoredValue::new_local(on_close);
    let hit_list = move || hits.get().into_iter().enumerate().collect::<Vec<_>>();
    view! {
        <Modal title="Search" on_close=on_close>
            <form class="search" on:submit=submit role="search">
                <input
                    type="search"
                    aria-label="Query"
                    placeholder="Sessions, messages, memory"
                    autofocus
                    prop:value=move || q.get()
                    on:input=move |ev| q.set(event_target_value(&ev))
                />
                <button type="submit" class="card-btn">"Find"</button>
            </form>
            <p class="hint">{move || msg.get()}</p>
            <ul class="hits">
                <For
                    each=hit_list
                    key=|(i, h)| (*i, h.title.clone(), h.snippet.clone())
                    children=move |(_, h)| {
                        let sid = h.session_id;
                        view! {
                            <li>
                                <button type="button" class="nt"
                                    on:click=move |_| {
                                        if let Some(s) = sid {
                                            core.get_value().goto_session(s);
                                            close_sv.with_value(|f| f());
                                        }
                                    }>
                                    <span class="kind">{h.kind}</span>
                                    <span class="text">{h.title}</span>
                                    <span class="text snippet">{h.snippet}</span>
                                </button>
                            </li>
                        }
                    }
                />
            </ul>
        </Modal>
    }
}

fn span_class(s: &Span) -> String {
    let mut c = String::from("ts");
    if let Some(n) = s.style.fg {
        c.push_str(&format!(" fg{n}"));
    }
    if s.style.bold {
        c.push_str(" b");
    }
    if s.style.underline {
        c.push_str(" u");
    }
    if s.stderr {
        c.push_str(" err");
    }
    c
}

#[component]
pub fn Terminal(sig: Sig, on_close: impl Fn() + Copy + 'static) -> impl IntoView {
    view! {
        <Modal title="Agent terminal" on_close=on_close>
            <p class="hint">"View only: the core runs agent commands, output arrives as session frames."</p>
            <p class="hint" hidden=move || !sig.term.with(|t| t.is_empty())>"No output yet"</p>
            <p class="hint" hidden=move || !sig.term.with(|t| t.truncated)>"Start of output discarded (limit 256 KiB)"</p>
            <pre class="term" role="log" aria-live="polite" tabindex="0" aria-label="Agent command output">
                {move || sig.term.get().spans.iter().map(|s| view! { <span class=span_class(s)>{s.text.clone()}</span> }).collect::<Vec<_>>()}
            </pre>
        </Modal>
    }
}
