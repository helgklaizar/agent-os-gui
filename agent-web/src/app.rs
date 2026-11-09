//! Root: token sign-in, then the shell (bar, sessions, chat).

use std::rc::Rc;

use leptos::prelude::*;

use crate::core::{Core, Sig};
use crate::util::{forget_token, store_token, stored_token};
use crate::view_bar::Bar;
use crate::view_chat::Chat;
use crate::view_notes::Notes;
use crate::view_rail::Tools;
use crate::view_settings::Settings;
use crate::view_tools::{Journal, Search, Tasks, Terminal};
use crate::view_tree::Tree;

/// Window open over the shell.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Settings,
    Tasks,
    Journal,
    Search,
    Terminal,
}

/// Visible column on a narrow screen (all are visible on a wide one).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Tree,
    Chat,
    Notes,
}

impl Pane {
    fn name(self) -> &'static str {
        match self {
            Pane::Tree => "tree",
            Pane::Chat => "chat",
            Pane::Notes => "notes",
        }
    }
}

#[component]
pub fn App() -> impl IntoView {
    let sig = Sig::new();
    let core: RwSignal<Option<Rc<Core>>, LocalStorage> = RwSignal::new_local(None);
    let connect = move |token: String| {
        if let Some(old) = core.get_untracked() {
            old.stop();
        }
        sig.denied.set(None);
        let c = Core::new(token, sig);
        c.start();
        core.set(Some(c));
    };
    if let Some(t) = stored_token() {
        connect(t);
    }
    // Core rejection: the token is forgotten, the form with the reason is shown.
    Effect::new(move |_| {
        if sig.denied.get().is_some() {
            forget_token();
            core.set(None);
        }
    });
    view! {
        {move || match core.get() {
            Some(c) => view! { <Shell sig=sig core=c /> }.into_any(),
            None => view! { <Login sig=sig on_token=connect /> }.into_any(),
        }}
    }
}

#[component]
fn Shell(sig: Sig, core: Rc<Core>) -> impl IntoView {
    let panel = RwSignal::new_local(Option::<Panel>::None);
    let pane = RwSignal::new_local(Pane::Chat);
    // Selecting a session (or opening one from an event) moves a narrow screen to the chat.
    Effect::new(move |_| {
        if sig.chat.with(|v| v.session_id).is_some() {
            pane.set(Pane::Chat);
        }
    });
    let panel_core = StoredValue::new_local(core.clone());
    // Ctrl+K or Cmd+K opens search.
    window_event_listener(leptos::ev::keydown, move |ev| {
        // Escape closes the open window.
        if ev.key() == "Escape" && panel.get_untracked().is_some() {
            panel.set(None);
        }
        if (ev.ctrl_key() || ev.meta_key()) && ev.key().eq_ignore_ascii_case("k") {
            ev.prevent_default();
            panel.set(Some(Panel::Search));
        }
    });
    view! {
        <div class="shell">
            <Bar sig=sig />
            {move || sig.notice.get().map(|t| view! {
                <p class="notice" role="status">
                    {t}
                    <button type="button" aria-label="Dismiss" on:click=move |_| sig.notice.set(None)>"✕"</button>
                </p>
            })}
            {move || sig.toast.get().map(|t| view! { <p class="toast" role="status">{t}</p> })}
            <nav class="panes" aria-label="Sections">
                <button type="button" class:active=move || pane.get() == Pane::Tree aria-pressed=move || (pane.get() == Pane::Tree).to_string()
                    on:click=move |_| pane.set(Pane::Tree)>"Sessions"</button>
                <button type="button" class:active=move || pane.get() == Pane::Chat aria-pressed=move || (pane.get() == Pane::Chat).to_string()
                    on:click=move |_| pane.set(Pane::Chat)>"Chat"</button>
                <button type="button" class:active=move || pane.get() == Pane::Notes aria-pressed=move || (pane.get() == Pane::Notes).to_string()
                    on:click=move |_| pane.set(Pane::Notes)>"Events"</button>
            </nav>
            <main class="cols" data-pane=move || pane.get().name()>
                <Notes sig=sig core=core.clone() />
                <Tree sig=sig core=core.clone() />
                <Chat sig=sig core=core />
                <Tools panel=panel />
            </main>
            {move || panel.get().map(|p| {
                let close = move || panel.set(None);
                match p {
                    Panel::Settings => view! { <Settings core=panel_core.get_value() on_close=close /> }.into_any(),
                    Panel::Tasks => view! { <Tasks core=panel_core.get_value() on_close=close /> }.into_any(),
                    Panel::Journal => view! { <Journal core=panel_core.get_value() on_close=close /> }.into_any(),
                    Panel::Search => view! { <Search core=panel_core.get_value() on_close=close /> }.into_any(),
                    Panel::Terminal => view! { <Terminal sig=sig on_close=close /> }.into_any(),
                }
            })}
        </div>
    }
}

#[component]
fn Login(sig: Sig, on_token: impl Fn(String) + Copy + 'static) -> impl IntoView {
    let (token, set_token) = signal(String::new());
    let (remember, set_remember) = signal(false);
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let t = token.get_untracked().trim().to_string();
        if t.is_empty() {
            return;
        }
        if remember.get_untracked() {
            store_token(&t);
        }
        set_token.set(String::new());
        on_token(t);
    };
    view! {
        <main class="login">
            <form on:submit=submit autocomplete="off">
                <h1>"Agent OS"</h1>
                <label for="tok">"Access token"</label>
                <input
                    id="tok"
                    type="password"
                    autocomplete="off"
                    prop:value=move || token.get()
                    on:input=move |ev| set_token.set(event_target_value(&ev))
                />
                <label class="remember">
                    <input
                        type="checkbox"
                        on:change=move |ev| set_remember.set(event_target_checked(&ev))
                    />
                    " Remember on this tab"
                </label>
                <button type="submit">"Sign in"</button>
                {move || sig.denied.get().map(|e| view! { <p class="error" role="alert">{e}</p> })}
            </form>
        </main>
    }
}
