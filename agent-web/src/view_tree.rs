//! Sessions column: tree, selection, New session.

use std::rc::Rc;

use controllers::controllers::session_tree::TreeMsg;
use controllers::tree_types::NodeId;
use controllers::tree_view::RowKind;
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use crate::core::{Core, Sig};
use crate::icons::Icon;

#[derive(Clone, Copy)]
enum Act {
    Folder,
    Rename,
    Thread,
    Clear,
    Delete,
    Move,
    FolderRename(i64),
    FolderDelete(i64),
}

fn folder_name(sig: Sig, id: i64) -> String {
    sig.tree.with(|t| {
        t.rows
            .iter()
            .find(|r| matches!(r.node, NodeId::Folder(f) if f == id))
            .map(|r| r.name.clone())
            .unwrap_or_default()
    })
}

#[component]
pub fn Tree(sig: Sig, core: Rc<Core>) -> impl IntoView {
    let rows = move || sig.tree.get().rows;
    let core = StoredValue::new_local(core);
    // Selected session and its name (from the tree rows).
    let current = move || {
        let id = sig.chat.with(|v| v.session_id)?;
        let name = sig
            .tree
            .with(|t| t.rows.iter().find(|r| r.selected).map(|r| r.name.clone()))
            .unwrap_or_default();
        Some((id, name))
    };
    let act = move |a: Act| {
        let c = core.get_value();
        let win = web_sys::window();
        let ask = |text: &str, def: &str| {
            win.as_ref()
                .and_then(|w| w.prompt_with_message_and_default(text, def).ok().flatten())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let yes = |text: &str| {
            win.as_ref()
                .and_then(|w| w.confirm_with_message(text).ok())
                .unwrap_or(false)
        };
        let (method, path, body) = match (a, current()) {
            (Act::Folder, _) => match ask("New folder name", "") {
                Some(n) => ("POST", "/v1/folders".to_string(), json!({"name": n})),
                None => return,
            },
            (Act::Rename, Some((id, name))) => match ask("New session name", &name) {
                Some(n) => ("PATCH", format!("/v1/sessions/{id}"), json!({"name": n})),
                None => return,
            },
            (Act::Thread, Some((id, _))) => match ask("Thread name", "Thread") {
                Some(n) => (
                    "POST",
                    format!("/v1/sessions/{id}/threads"),
                    json!({"name": n}),
                ),
                None => return,
            },
            (Act::Clear, Some((id, name))) => {
                if !yes(&format!(
                    "Clear the history of session “{name}”? All its messages will be lost."
                )) {
                    return;
                }
                ("POST", format!("/v1/sessions/{id}/clear"), json!({}))
            }
            (Act::Delete, Some((id, name))) => {
                if !yes(&format!(
                    "Delete session “{name}”? Its threads, messages, bookmarks and browser profile will be lost; events and the action log will remain."
                )) {
                    return;
                }
                (
                    "DELETE",
                    format!("/v1/sessions/{id}"),
                    serde_json::Value::Null,
                )
            }
            (Act::Move, Some((id, _))) => {
                let folders: Vec<(i64, String)> = sig.tree.with(|t| {
                    t.rows
                        .iter()
                        .filter_map(|r| match r.node {
                            NodeId::Folder(f) => Some((f, r.name.clone())),
                            NodeId::Session(_) => None,
                        })
                        .collect()
                });
                let names: Vec<&str> = folders.iter().map(|(_, n)| n.as_str()).collect();
                let Some(answer) = win.as_ref().and_then(|w| {
                    w.prompt_with_message_and_default(
                        &format!(
                            "Destination folder: {} (empty for the root)",
                            names.join(", ")
                        ),
                        "",
                    )
                    .ok()
                    .flatten()
                }) else {
                    return;
                };
                let answer = answer.trim();
                let target = if answer.is_empty() {
                    serde_json::Value::Null
                } else if let Some((f, _)) = folders.iter().find(|(_, n)| n == answer) {
                    json!(f)
                } else {
                    c.notice(&format!("No folder “{answer}”"));
                    return;
                };
                (
                    "PATCH",
                    format!("/v1/sessions/{id}"),
                    json!({"folder_id": target}),
                )
            }
            (Act::FolderRename(f), _) => {
                let old = folder_name(sig, f);
                match ask("New folder name", &old) {
                    Some(n) => ("PATCH", format!("/v1/folders/{f}"), json!({"name": n})),
                    None => return,
                }
            }
            (Act::FolderDelete(f), _) => {
                let name = folder_name(sig, f);
                if !yes(&format!(
                    "Delete folder “{name}”? The folder itself will be lost; its sessions and folders move up one level."
                )) {
                    return;
                }
                (
                    "DELETE",
                    format!("/v1/folders/{f}"),
                    serde_json::Value::Null,
                )
            }
            _ => return,
        };
        let cleared = match (a, current()) {
            (Act::Clear, Some((id, _))) => Some(id),
            _ => None,
        };
        spawn_local(async move {
            match c.call(method, &path, Some(body)).await {
                Ok(()) => {
                    if let Some(id) = cleared {
                        c.resync_session(id);
                    }
                }
                Err(e) => c.notice(&format!("Action failed: {e}")),
            }
        });
    };
    let tool = move |a: Act, icon: &'static str, label: &'static str| {
        view! {
            <button type="button" class="ico-btn" title=label aria-label=label
                prop:disabled=move || current().is_none() on:click=move |_| act(a)>
                <Icon name=icon size=16 />
            </button>
        }
    };
    view! {
        <nav class="tree" aria-label="Sessions">
            <header class="col-head">
                <span class="head-ic"><Icon name="chat" size=20 /></span>
                <h2>"Sessions"</h2>
            </header>
            <div class="tree-tools" role="group" aria-label="Session actions">
                {tool(Act::Rename, "pencil", "Rename")}
                {tool(Act::Thread, "thread", "Thread")}
                {tool(Act::Move, "move", "Move")}
                {tool(Act::Clear, "eraser", "Clear")}
                {tool(Act::Delete, "trash", "Delete")}
            </div>
            <ul role="tree" class="tree-list">
                <For
                    each=rows
                    key=|r| (r.node, r.name.clone(), r.selected, r.unread, r.expanded)
                    children=move |r| {
                        let node = r.node;
                        let kind = r.kind;
                        let cls = format!(
                            "row {}{}",
                            match kind {
                                RowKind::Folder => "folder",
                                RowKind::Session => "session",
                                RowKind::Thread => "thread",
                            },
                            if r.selected { " selected" } else { "" }
                        );
                        let pad = format!("{}px", 8 + r.indent);
                        let lead = match (kind, r.expanded) {
                            (RowKind::Folder, Some(true)) => "chev-down",
                            (RowKind::Folder, _) => "chev-right",
                            (RowKind::Session, _) => "chat",
                            (RowKind::Thread, _) => "thread",
                        };
                        view! {
                            <li role="treeitem" aria-selected=r.selected.to_string()>
                                <button
                                    class=cls
                                    style:padding-left=pad
                                    aria-label=r.a11y_name
                                    on:click=move |_| match node {
                                        NodeId::Folder(id) => core.get_value().tree_msg(TreeMsg::ToggleFolder(id)),
                                        NodeId::Session(id) => core.get_value().tree_msg(TreeMsg::Select(id)),
                                    }
                                >
                                    <span class="ic"><Icon name=lead size=16 /></span>
                                    <span class="name">{r.name}</span>
                                    {r.badge.map(|b| view! { <span class="badge">{b}</span> })}
                                </button>
                                {match node {
                                    NodeId::Folder(f) => Some(view! {
                                        <span class="folder-tools">
                                            <button type="button" class="ico-btn sm" aria-label=format!("Rename folder {}", folder_name(sig, f)) on:click=move |_| act(Act::FolderRename(f))><Icon name="pencil" size=14 /></button>
                                            <button type="button" class="ico-btn sm" aria-label=format!("Delete folder {}", folder_name(sig, f)) on:click=move |_| act(Act::FolderDelete(f))><Icon name="x" size=14 /></button>
                                        </span>
                                    }),
                                    NodeId::Session(_) => None,
                                }}
                            </li>
                        }
                    }
                />
            </ul>
            <div class="tree-new">
                <button type="button" class="new" on:click=move |_| core.get_value().create_session()>
                    <span class="ic"><Icon name="plus" size=16 /></span>"New session"
                </button>
                <button type="button" class="new" on:click=move |_| act(Act::Folder)>
                    <span class="ic"><Icon name="folder-plus" size=16 /></span>"New folder"
                </button>
            </div>
        </nav>
    }
}
