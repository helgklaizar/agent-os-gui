//! Settings dialog in real GTK: 1024×640 geometry, hostile text, deleting a persona with confirmation.

use controllers::SettingsPage;
use controllers::i18n::tr;
use gtk4 as gtk;
use gtk4::prelude::*;
use serde_json::json;

use crate::ctx::{Ctx, Scenario, WAIT, settle};
use crate::settings::{click, open_dialog, select};
use crate::tree::{find_all, open_sized};

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![
        ("ui_settings_dialog_buttons_reachable_1024x640", geometry),
        ("ui_settings_text_is_escaped", escaped),
        ("ui_settings_persona_delete_needs_confirm", persona_delete),
    ]
}

fn geometry(ctx: &Ctx) -> Result<(), String> {
    let d = ctx
        .rt
        .block_on(fake_agentd::FakeAgentd::start(
            fake_agentd::Config::temp().no_ticker(),
        ))
        .map_err(|e| e.to_string())?;
    let rig = open_sized(ctx, d, 1024, 640)?;
    let dialog = open_dialog(&rig, SettingsPage::Personas)?;
    let window = rig.shell.window().clone();
    settle("window 1024x640", WAIT, || {
        window.width() >= 1024 && window.height() >= 640
    })?;
    settle("card laid out", WAIT, || dialog.card.height() > 0)?;
    let (w, h) = (f64::from(window.width()), f64::from(window.height()));
    let card = dialog
        .card
        .compute_bounds(&window)
        .ok_or("no card bounds")?;
    if f64::from(card.height()) > h * 0.9 + 1.0 {
        return Err(format!(
            "card {} taller than 90% of the window {h}",
            card.height()
        ));
    }
    if f64::from(card.width()) > 960.0 + 40.0 {
        return Err(format!("card {} wider than 960 + margins", card.width()));
    }
    for (name, b) in [("Save", &dialog.save), ("Close", &dialog.close)] {
        let r = b.compute_bounds(&window).ok_or("no button bounds")?;
        let inside =
            r.y() >= 0.0 && f64::from(r.y() + r.height()) <= h && f64::from(r.x() + r.width()) <= w;
        if !inside || !b.is_visible() || !b.is_mapped() {
            return Err(format!("button '{name}' is outside the window: {r:?}"));
        }
    }
    let scroll = dialog.scroll_widget();
    let adj = scroll.vadjustment();
    if adj.upper() <= adj.page_size() {
        return Err("the personas page does not scroll at 1024x640".into());
    }
    adj.set_value(adj.upper() - adj.page_size());
    let r = dialog
        .save
        .compute_bounds(&window)
        .ok_or("no button bounds")?;
    if f64::from(r.y() + r.height()) > h {
        return Err("after scrolling the 'Save' button went off the window".into());
    }
    rig.shell.window().destroy();
    Ok(())
}

fn escaped(ctx: &Ctx) -> Result<(), String> {
    let rig = crate::tree::open(ctx)?;
    for (i, evil) in controllers::testkit::evil_strings().into_iter().enumerate() {
        // the core does not accept control characters in names: the rest (markup, quotes, RTL, length) goes through as is
        let evil: String = evil
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let name: String = evil.chars().take(70).collect();
        ctx.rt
            .block_on(
                rig.http
                    .create_persona(json!({"name": format!("p{i}{name}")})),
            )
            .map_err(|e| e.to_string())?;
        ctx.rt
            .block_on(rig.http.create_tunnel(json!({"type": "ssh", "addr": format!("h{i}"), "descr": evil.chars().take(200).collect::<String>()})))
            .map_err(|e| e.to_string())?;
    }
    let dialog = open_dialog(&rig, SettingsPage::Personas)?;
    select(&rig, SettingsPage::Tunnels)?;
    select(&rig, SettingsPage::Personas)?;
    for l in find_all::<gtk::Label>(dialog.card.upcast_ref()) {
        if l.uses_markup() {
            return Err(format!("label with markup: {}", l.text()));
        }
    }
    let texts = dialog.rows.texts().join("\n");
    if !texts.contains("<img src=x onerror=alert(1)>") || !texts.contains("<b>x</b>") {
        return Err("hostile strings are not shown literally".into());
    }
    select(&rig, SettingsPage::Tunnels)?;
    for l in find_all::<gtk::Label>(dialog.card.upcast_ref()) {
        if l.uses_markup() {
            return Err(format!("label with markup on 'Tunnels': {}", l.text()));
        }
    }
    rig.shell.window().destroy();
    Ok(())
}

fn persona_delete(ctx: &Ctx) -> Result<(), String> {
    let rig = crate::tree::open(ctx)?;
    let dialog = open_dialog(&rig, SettingsPage::Personas)?;
    let before = ctx
        .rt
        .block_on(rig.http.list_personas())
        .map_err(|e| e.to_string())?
        .len();
    let delete = dialog
        .find_button(&tr("set-delete"))
        .ok_or("no delete button")?;
    delete.emit_clicked();
    settle("confirmation shown", WAIT, || dialog.confirm_visible())?;
    let after = ctx
        .rt
        .block_on(rig.http.list_personas())
        .map_err(|e| e.to_string())?
        .len();
    if after != before {
        return Err("the persona was deleted without confirmation".into());
    }
    click(
        &dialog
            .find_button(&tr("ss-confirm-yes"))
            .ok_or("no 'Confirm'")?,
    );
    settle("persona deleted after confirmation", WAIT, || {
        ctx.rt
            .block_on(rig.http.list_personas())
            .is_ok_and(|l| l.len() + 1 == before)
    })?;
    rig.shell.window().destroy();
    Ok(())
}
