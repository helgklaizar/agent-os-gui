//! Tools column: tab icons that open windows (search, schedule, log, terminal, settings).

use leptos::prelude::*;

use crate::app::Panel;
use crate::icons::Icon;

#[component]
pub fn Tools(panel: RwSignal<Option<Panel>, LocalStorage>) -> impl IntoView {
    let tool = move |p: Panel, icon: &'static str, label: &'static str| {
        view! {
            <button
                type="button"
                class="ico-btn tool"
                class:active=move || panel.get() == Some(p)
                title=label
                aria-label=label
                on:click=move |_| panel.set(Some(p))
            >
                <Icon name=icon size=20 />
                <span class="tool-label" aria-hidden="true">{label}</span>
            </button>
        }
    };
    view! {
        <aside class="tools" aria-label="Tools">
            <div class="tools-row" role="group" aria-label="Tool windows">
                {tool(Panel::Terminal, "term", "Terminal")}
                {tool(Panel::Search, "search", "Search")}
                {tool(Panel::Tasks, "clock", "Schedule")}
                {tool(Panel::Journal, "journal", "Log")}
                {tool(Panel::Settings, "settings", "Settings")}
            </div>
        </aside>
    }
}
