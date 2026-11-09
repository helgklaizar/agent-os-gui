//! Bar: connection, AI account quotas, clock.

use controllers::ConnState;
use controllers::level::Level;
use leptos::prelude::*;

use crate::core::Sig;
use crate::indicators::state_class;

pub fn level_class(l: Level) -> &'static str {
    match l {
        Level::Neutral => "l-neutral",
        Level::Ok => "l-ok",
        Level::Warn => "l-warn",
        Level::Bad => "l-bad",
        Level::NoData => "l-nodata",
    }
}

fn conn_text(s: ConnState) -> (&'static str, &'static str) {
    match s {
        ConnState::Connected => ("Connected", "l-ok"),
        ConnState::Connecting => ("Connecting…", "l-warn"),
        ConnState::Reconnecting => ("Reconnecting…", "l-warn"),
        ConnState::Offline => ("No connection", "l-bad"),
        ConnState::CoreUnavailable => ("Core unavailable", "l-bad"),
    }
}

#[component]
pub fn Bar(sig: Sig) -> impl IntoView {
    let items = move || sig.bar.get().quotas.items;
    view! {
        <header class="bar" role="banner">
            <span class="bar-title"><span class="logo" aria-hidden="true"></span>"Agent OS"</span>
            <div class="bar-mid">
            <ul class="quotas" aria-label="Quotas">
                <For
                    each=items
                    key=|q| (q.account_id, q.label.clone(), q.remaining_pct, q.load_pct, q.rate_text.clone())
                    children=move |q| {
                        let cls = format!("quota {}", level_class(q.level));
                        let value = q
                            .remaining_pct
                            .map(|p| format!("{p}%"))
                            .or_else(|| q.load_pct.map(|p| format!("{p}%")))
                            .unwrap_or_else(|| "—".to_string());
                        let fill = q.remaining_pct.map(|p| format!("{}%", p.min(100)));
                        view! {
                            <li class=cls title=q.tooltip aria-label=q.accessible_name>
                                <span class="q-label">{q.label}</span>
                                <span class="q-value">{value}</span>
                                {fill.map(|w| view! { <span class="q-bar" aria-hidden="true"><i style:width=w></i></span> })}
                                {q.rate_text.map(|r| view! { <span class="q-rate">{r}</span> })}
                            </li>
                        }
                    }
                />
            </ul>
            <ul class="metrics" aria-label="System">
                <For
                    each=move || sig.bar.get().metrics.items
                    key=|m| (m.caption.clone(), m.text.clone(), level_class(m.level))
                    children=move |m| view! {
                        <li class=format!("metric {}", level_class(m.level)) title=m.label aria-label=m.accessible_name>
                            <span class="q-label">{m.caption}</span>
                            <span class="q-value">{m.text}</span>
                        </li>
                    }
                />
            </ul>
            <ul class="inds" aria-label="Indicators">
                <For
                    each=move || sig.inds.get()
                    key=|i| (i.id.clone(), i.state.clone(), i.text.clone())
                    children=move |i| view! {
                        <li class=format!("ind {}", state_class(&i.state)) aria-label=format!("{}: {}", i.id, i.state)>
                            <span class="q-label">{i.id.clone()}</span>
                            {(!i.text.is_empty()).then(|| view! { <span class="q-value">{i.text.clone()}</span> })}
                        </li>
                    }
                />
            </ul>
            </div>
            <div class="bar-right">
            <span class="weather" title=move || sig.bar.get().bar.weather.tooltip>
                {move || { let w = sig.bar.get().bar.weather; format!("{} {}", w.temp, w.text) }}
            </span>
            <span class="bar-clock">{move || sig.bar.get().bar.time}</span>
            <span class="l-warn" hidden=move || sig.queued.get() == 0>
                {move || format!("Queued: {}", sig.queued.get())}
            </span>
            <span class=move || format!("conn {}", conn_text(sig.conn.get()).1) role="status">
                {move || conn_text(sig.conn.get()).0}
            </span>
            </div>
        </header>
    }
}
