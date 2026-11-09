//! Chat column: message flow, cards, input field, Stop.

use std::rc::Rc;

use controllers::chat_view::{
    AskCard, ButtonKind, CardButton, ChatItem, Delivery, ErrorCard, ObserverCard,
};
use controllers::controllers::chat::{ChatMsg, InputKey};
use leptos::html::{Div, Input};
use leptos::prelude::*;

use crate::core::{Core, Sig};
use crate::icons::Icon;
use crate::md_view::blocks_view;
use crate::util::uuid7;

type CoreSv = StoredValue<Rc<Core>, LocalStorage>;

fn buttons_view(request_id: i64, buttons: Vec<CardButton>, core: CoreSv) -> Vec<AnyView> {
    buttons
        .into_iter()
        // "Always" changes the rules and is decided only in the native GUI.
        .filter(|b| b.kind != ButtonKind::Always)
        .map(|b| {
            let kind = b.kind;
            view! {
                <button
                    type="button"
                    class="card-btn"
                    on:click=move |_| core.get_value().chat_msg(ChatMsg::CardButton { request_id, button: kind })
                >{b.label}</button>
            }
            .into_any()
        })
        .collect()
}

fn ask_view(c: AskCard, core: CoreSv) -> AnyView {
    let state = c.state_label.unwrap_or_default();
    let label = c.title.clone();
    let buttons = buttons_view(c.request_id, c.buttons, core);
    view! {
        <div class="card ask" role="group" aria-label=label>
            <strong>{c.title}</strong>
            <code class="target">{c.target}</code>
            {c.detail.map(|d| view! { <p>{d}</p> })}
            {(!state.is_empty()).then(|| view! { <span class="state">{state}</span> })}
            <div class="card-actions">{buttons}</div>
        </div>
    }
    .into_any()
}

fn observer_view(c: ObserverCard, core: CoreSv) -> AnyView {
    let state = c.state_label.unwrap_or_default();
    let buttons = buttons_view(c.request_id, c.buttons, core);
    view! {
        <div class="card observer red" role="alert">
            <strong>{c.kind_label}</strong>
            <code class="target">{c.target}</code>
            {c.detail.map(|d| view! { <p>{d}</p> })}
            <span class="verdict">{c.verdict_label}</span>
            {(!state.is_empty()).then(|| view! { <span class="state">{state}</span> })}
            <div class="card-actions">{buttons}</div>
        </div>
    }
    .into_any()
}

fn error_view(c: ErrorCard) -> AnyView {
    view! {
        <div class="card error" role="alert">
            <strong>{c.title}</strong>
            <pre>{c.text}</pre>
        </div>
    }
    .into_any()
}

fn item_view(it: ChatItem, core: CoreSv) -> AnyView {
    match it {
        ChatItem::Ask(c) => ask_view(c, core),
        ChatItem::Observer(c) => observer_view(c, core),
        ChatItem::Error(c) => error_view(c),
        ChatItem::Bubble(b) => {
            let cls = format!("bubble {}", if b.mine { "mine" } else { "agent" });
            let delivery = match b.delivery {
                Delivery::Sent => None,
                Delivery::Pending => Some("sending…"),
                Delivery::Failed => Some("not sent"),
            };
            view! {
                <article class=cls>
                    <header><span class="author">{b.author_line}</span><time>{b.time}</time></header>
                    <div class="md">{blocks_view(b.blocks)}</div>
                    {(!b.attachments.is_empty()).then(|| view! {
                        <ul class="atts" aria-label="Attachments">
                            {b.attachments.into_iter().map(|a| view! { <li>{format!("{} ({})", a.name, a.size_text)}</li> }).collect::<Vec<_>>()}
                        </ul>
                    })}
                    {b.streaming.then(|| view! { <span class="stream" aria-label="reply in progress">"▍"</span> })}
                    {b.note.map(|n| view! { <p class="note">{n}</p> })}
                    {delivery.map(|d| view! { <p class="note">{d}</p> })}
                </article>
            }
            .into_any()
        }
    }
}

#[component]
pub fn Chat(sig: Sig, core: Rc<Core>) -> impl IntoView {
    let flow = NodeRef::<Div>::new();
    let file = NodeRef::<Input>::new();
    let core_sv: CoreSv = StoredValue::new_local(core.clone());
    // The flow scrolls down when the controller increased `scroll_seq`.
    Effect::new(move |_| {
        sig.chat.with(|v| v.scroll_seq);
        if let Some(el) = flow.get() {
            el.set_scroll_top(el.scroll_height());
        }
    });
    let items = move || {
        sig.chat
            .get()
            .items
            .into_iter()
            .enumerate()
            .collect::<Vec<_>>()
    };
    let (c_send, c_key, c_input, c_stop, c_scroll, c_file) = (
        core.clone(),
        core.clone(),
        core.clone(),
        core.clone(),
        core.clone(),
        core.clone(),
    );
    let staged = move || {
        sig.chat
            .get()
            .attachments
            .into_iter()
            .enumerate()
            .collect::<Vec<_>>()
    };
    view! {
        <section class="chat" aria-label="Chat">
            {move || sig.chat.get().empty.map(|e| view! { <p class="empty">{e}</p> })}
            <div
                class="flow"
                node_ref=flow
                role="log"
                aria-live="polite"
                on:scroll=move |_| {
                    if let Some(el) = flow.get() {
                        let at_bottom = el.scroll_height() - el.scroll_top() - el.client_height() < 40;
                        c_scroll.chat_msg(ChatMsg::Scrolled { at_bottom });
                    }
                }
            >
                <For
                    each=items
                    key=|(i, it)| (*i, format!("{it:?}"))
                    children=move |(_, it)| item_view(it, core_sv)
                />
                {move || sig.chat.get().typing.map(|t| view! { <p class="typing">{t}</p> })}
            </div>
            <ul class="staged" aria-label="Draft attachments" hidden=move || sig.chat.with(|v| v.attachments.is_empty())>
                <For
                    each=staged
                    key=|(i, a)| (*i, a.name.clone())
                    children=move |(i, a)| {
                        view! {
                            <li>
                                <span>{format!("{} ({})", a.name, a.size_text)}</span>
                                <button type="button" class="link" aria-label=format!("Remove attachment {}", a.name)
                                    on:click=move |_| core_sv.get_value().chat_msg(ChatMsg::AttachRemove(i))><Icon name="x" size=12 /></button>
                            </li>
                        }
                    }
                />
            </ul>
            <form class="composer" on:submit=move |ev| ev.prevent_default()>
                <input
                    type="file"
                    class="visually-hidden"
                    node_ref=file
                    tabindex="-1"
                    aria-hidden="true"
                    on:change=move |_| {
                        if let Some(inp) = file.get() {
                            if let Some(f) = inp.files().and_then(|l| l.get(0)) {
                                c_file.upload_file(f);
                            }
                            inp.set_value("");
                        }
                    }
                />
                <button
                    type="button"
                    class="ico-btn attach"
                    title="Attach a file"
                    aria-label="Attach a file"
                    prop:disabled=move || sig.chat.get().session_id.is_none()
                    on:click=move |_| {
                        if let Some(inp) = file.get() {
                            inp.click();
                        }
                    }
                ><Icon name="clip" /></button>
                <textarea
                    rows="1"
                    placeholder="Write to the agent…"
                    title="Enter to send, Shift+Enter for a new line"
                    aria-label="Message"
                    prop:value=move || sig.chat.get().draft
                    prop:disabled=move || sig.chat.get().session_id.is_none()
                    on:input=move |ev| c_input.chat_msg(ChatMsg::DraftChanged(event_target_value(&ev)))
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Enter" && !ev.shift_key() && !ev.is_composing() {
                            ev.prevent_default();
                            c_key.chat_msg(ChatMsg::Key { key: InputKey::Enter, client_id: uuid7() });
                        }
                    }
                ></textarea>
                <button
                    type="button"
                    class="send"
                    title="Send"
                    aria-label="Send"
                    prop:disabled=move || !sig.chat.get().can_send
                    on:click=move |_| c_send.chat_msg(ChatMsg::Send { client_id: uuid7() })
                ><Icon name="send" size=16 /></button>
                <button
                    type="button"
                    class="send stop"
                    title="Stop"
                    aria-label="Stop"
                    prop:hidden=move || !sig.chat.get().can_stop
                    on:click=move |_| c_stop.chat_msg(ChatMsg::Stop)
                ><Icon name="stop" size=16 /></button>
            </form>
        </section>
    }
}
