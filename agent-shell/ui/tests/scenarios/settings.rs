//! Settings dialog in real GTK under headless Wayland: entering and leaving, saving to the core and after a GUI restart,
//! save error, secrets, 1024×640 geometry, theme without restart, hostile text.

use std::rc::Rc;

use controllers::i18n::tr;
use controllers::settings_view::SettingsMsg;
use controllers::{BusEvent, SettingsPage, Theme};
use gtk4 as gtk;
use gtk4::prelude::*;
use ui::settings_widgets::SettingsDialog;

use crate::ctx::{Ctx, Scenario, WAIT, settle};
use crate::tree::{Rig, open_with, press};

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![
        ("ui_settings_value_persists_across_gui_restart", persists),
        ("ui_settings_close_esc_button_backdrop", close_ways),
        (
            "ui_settings_save_error_is_visible_and_keeps_value",
            save_error,
        ),
        ("ui_settings_secret_field_is_write_only", secret_field),
        ("ui_settings_theme_change_applies_live", theme_live),
    ]
}

pub(crate) fn open_dialog(rig: &Rig, page: SettingsPage) -> Result<Rc<SettingsDialog>, String> {
    rig.shell.bus().publish(BusEvent::OpenSettings(page));
    settle("dialog open, data loaded", WAIT, || {
        let v = rig.shell.settings_dialog_view();
        v.open && !v.loading && rig.shell.settings_dialog().layer.is_visible()
    })?;
    Ok(rig.shell.settings_dialog())
}

pub(crate) fn select(rig: &Rig, page: SettingsPage) -> Result<(), String> {
    rig.shell.settings_send(SettingsMsg::Select(page));
    settle("page selected", WAIT, || {
        rig.shell.settings_dialog_view().page == page
    })
}

pub(crate) fn entry(d: &SettingsDialog, id: &str) -> Result<gtk::Entry, String> {
    d.rows
        .control(id)
        .and_then(|w| w.downcast::<gtk::Entry>().ok())
        .ok_or_else(|| format!("no input field {id}"))
}

pub(crate) fn click(b: &gtk::Button) {
    b.emit_clicked();
}

fn restart(ctx: &Ctx, rig: Rig) -> Result<Rig, String> {
    let Rig { d, shell, .. } = rig;
    shell.window().destroy();
    ctx.close_windows();
    open_with(ctx, d)
}

fn persists(ctx: &Ctx) -> Result<(), String> {
    let rig = crate::tree::open(ctx)?;
    let dialog = open_dialog(&rig, SettingsPage::Account)?;
    entry(&dialog, "d:account:name")?.set_text("Agent UI");
    settle("'Save' available", WAIT, || dialog.save.is_sensitive())?;
    click(&dialog.save);
    settle("value in the core", WAIT, || {
        ctx.rt
            .block_on(rig.http.settings_page("account"))
            .is_ok_and(|p| p.doc["name"] == "Agent UI")
    })?;
    settle("'Saved'", WAIT, || {
        dialog.banner_text().is_some_and(|t| t == tr("set-saved"))
    })?;
    // GUI restart: the same fake-agentd, a new window; the value is in place
    let rig = restart(ctx, rig)?;
    let dialog = open_dialog(&rig, SettingsPage::Account)?;
    settle("name after restart", WAIT, || {
        entry(&dialog, "d:account:name").is_ok_and(|e| e.text() == "Agent UI")
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn press_backdrop(d: &SettingsDialog) -> Result<(), String> {
    let model = d.layer.observe_controllers();
    for i in 0..model.n_items() {
        if let Some(obj) = model.item(i)
            && let Ok(g) = obj.downcast::<gtk::GestureClick>()
        {
            g.emit_by_name::<()>("pressed", &[&1i32, &2.0f64, &2.0f64]);
            return Ok(());
        }
    }
    Err("the layer has no click gesture".into())
}

fn close_ways(ctx: &Ctx) -> Result<(), String> {
    let rig = crate::tree::open(ctx)?;
    for way in ["esc", "button", "backdrop"] {
        let d = open_dialog(&rig, SettingsPage::Root)?;
        settle("layer stretched", WAIT, || d.layer.width() > 400)?;
        match way {
            "esc" => press(&d.layer, gtk::gdk::Key::Escape)?,
            "button" => click(&d.close),
            _ => press_backdrop(&d)?,
        }
        settle(&format!("dialog closed ({way})"), WAIT, || {
            !d.layer.is_visible() && !rig.shell.settings_dialog_view().open
        })?;
    }
    rig.shell.window().destroy();
    Ok(())
}

fn save_error(ctx: &Ctx) -> Result<(), String> {
    let rig = crate::tree::open(ctx)?;
    let dialog = open_dialog(&rig, SettingsPage::Account)?;
    rig.d.fail_next("PATCH /v1/settings/account", 500, 1);
    entry(&dialog, "d:account:name")?.set_text("Keep Me");
    settle("button available", WAIT, || dialog.save.is_sensitive())?;
    click(&dialog.save);
    settle("error visible in the dialog", WAIT, || {
        dialog
            .banner_text()
            .is_some_and(|t| t == tr("set-err-unavailable"))
    })?;
    if entry(&dialog, "d:account:name")?.text() != "Keep Me" {
        return Err("input lost after an error".into());
    }
    if !dialog.save.is_sensitive() {
        return Err("saving again is impossible".into());
    }
    click(&dialog.save);
    settle("retry saved", WAIT, || {
        ctx.rt
            .block_on(rig.http.settings_page("account"))
            .is_ok_and(|p| p.doc["name"] == "Keep Me")
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn secret_field(ctx: &Ctx) -> Result<(), String> {
    let rig = crate::tree::open(ctx)?;
    let dialog = open_dialog(&rig, SettingsPage::Models)?;
    let accounts = ctx
        .rt
        .block_on(rig.http.list_accounts())
        .map_err(|e| e.to_string())?;
    let ai = accounts
        .iter()
        .find(|a| {
            !["wa", "tg", "gmail", "mail", "imap", "whapi"]
                .contains(&a["type"].as_str().unwrap_or(""))
        })
        .ok_or("no AI account in the fixture")?;
    let reference = format!("account.{}", ai["id"]);
    let id = format!("s:{reference}");
    let field = dialog
        .rows
        .control(&id)
        .and_then(|w| w.downcast::<gtk::PasswordEntry>().ok())
        .ok_or("no key field")?;
    field.set_text("sk-ui-TOPSECRET");
    settle("'Save' available", WAIT, || dialog.save.is_sensitive())?;
    click(&dialog.save);
    settle("secret written in the core", WAIT, || {
        rig.d.secret_value(&reference).as_deref() == Some("sk-ui-TOPSECRET")
    })?;
    // after saving the page is rebuilt (the key became "set"): check the field currently on screen
    settle("field cleared", WAIT, || {
        dialog
            .rows
            .control(&id)
            .and_then(|w| w.downcast::<gtk::PasswordEntry>().ok())
            .is_some_and(|f| f.text().is_empty())
    })?;
    let _ = &field;
    let shown = dialog.rows.texts().join("\n") + &dialog.banner_text().unwrap_or_default();
    if shown.contains("TOPSECRET") {
        return Err("the secret value is shown in the dialog".into());
    }
    rig.shell.window().destroy();
    Ok(())
}

fn theme_live(ctx: &Ctx) -> Result<(), String> {
    let rig = crate::tree::open(ctx)?;
    let dialog = open_dialog(&rig, SettingsPage::Account)?;
    let theme = dialog
        .rows
        .control("theme")
        .and_then(|w| w.downcast::<gtk::DropDown>().ok())
        .ok_or("no theme choice")?;
    theme.set_selected(1);
    settle("theme applied without restart", WAIT, || {
        rig.shell.selected_theme() == Theme::Dark
    })?;
    settle("theme written to the core", WAIT, || {
        ctx.rt
            .block_on(rig.http.theme())
            .is_ok_and(|t| t.theme == core_client::api::Theme::Dark)
    })?;
    rig.shell.window().destroy();
    Ok(())
}
