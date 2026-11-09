//! Bar in real GTK under headless Wayland: menu closing, logout, live values, width, contrast, accessibility.

use std::path::PathBuf;

use controllers::MenuItem;
use controllers::timezone::Zone;
use fake_agentd::{Config, CoreMode, Scenario};
use gtk4 as gtk;
use gtk4::prelude::*;
use ui::shell::Shell;

use crate::bar::{
    Rig, bar_texts, click, fixed_clock, focus_on_menu_button, has_text, open, open_menu_and_click,
    open_w, press_escape, wait_loaded,
};
use crate::ctx::{Ctx, Scenario as Run, WAIT, settle};

pub fn scenarios() -> Vec<(&'static str, Run)> {
    vec![
        ("ui_bar_menu_closes_by_esc_and_outside_click", menu_dismiss),
        ("ui_bar_logout_cancel_and_confirm", logout_flow),
        ("ui_bar_live_values", live_values),
        ("ui_bar_clock_follows_injected_clock", clock_injected),
        ("ui_bar_evil_label_literal", evil_label),
        ("ui_bar_offline_banner_and_grey", offline_grey),
        ("ui_bar_core_unavailable_banner", core_unavailable),
        ("ui_bar_1024", width_1024),
        ("ui_bar_contrast", contrast),
        ("ui_bar_accessibility_tree", accessibility_tree),
    ]
}

fn menu_dismiss(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx, Config::temp().no_ticker(), |_| {})?;
    wait_loaded(&rig)?;
    click(&rig.shell.menu_button());
    settle("menu open", WAIT, || rig.shell.menu_open())?;
    press_escape(&rig.shell)?;
    settle("menu closed by Esc", WAIT, || !rig.shell.menu_open())?;
    settle("focus on the menu button", WAIT, || {
        focus_on_menu_button(&rig.shell)
    })?;
    click(&rig.shell.menu_button());
    settle("menu open again", WAIT, || rig.shell.menu_open())?;
    let layer = rig.shell.menu_layer();
    settle("layer stretched", WAIT, || layer.width() > 400)?;
    let model = layer.observe_controllers();
    let gesture = (0..model.n_items())
        .filter_map(|i| model.item(i))
        .find_map(|o| o.downcast::<gtk::GestureClick>().ok())
        .ok_or("the menu layer has no click gesture")?;
    // a click on the menu card does not close it
    gesture.emit_by_name::<()>("pressed", &[&1i32, &20.0f64, &40.0f64]);
    if !rig.shell.menu_open() {
        return Err("a click on the menu card closed it".into());
    }
    // a click outside the card closes it
    let (x, y) = (
        f64::from(layer.width()) - 10.0,
        f64::from(layer.height()) - 10.0,
    );
    gesture.emit_by_name::<()>("pressed", &[&1i32, &x, &y]);
    settle("menu closed by a click outside", WAIT, || {
        !rig.shell.menu_open()
    })?;
    // a second click on the button closes the open menu
    click(&rig.shell.menu_button());
    settle("menu open", WAIT, || rig.shell.menu_open())?;
    click(&rig.shell.menu_button());
    settle("menu closed by the button", WAIT, || !rig.shell.menu_open())?;
    rig.shell.window().destroy();
    Ok(())
}

fn logout_requests(rig: &Rig) -> usize {
    rig.d
        .requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path == "/v1/logout" && r.status == 204)
        .count()
}

fn logout_flow(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx, Config::temp().no_ticker(), |_| {})?;
    wait_loaded(&rig)?;
    open_menu_and_click(&rig.shell, MenuItem::Logout)?;
    settle("confirmation prompt", WAIT, || rig.shell.dialog_open())?;
    rig.shell
        .dialog_button("Cancel")
        .ok_or("no 'Cancel' button")?
        .emit_clicked();
    settle("dialog closed", WAIT, || !rig.shell.dialog_open())?;
    for _ in 0..20 {
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if logout_requests(&rig) != 0 {
        return Err("cancel sent Logout to the core".into());
    }
    open_menu_and_click(&rig.shell, MenuItem::Logout)?;
    settle("confirmation prompt", WAIT, || rig.shell.dialog_open())?;
    rig.shell
        .dialog_button("Log out")
        .ok_or("no 'Log out' button")?
        .emit_clicked();
    settle("core received Logout", WAIT, || logout_requests(&rig) == 1)?;
    rig.shell.window().destroy();
    Ok(())
}

fn live_values(ctx: &Ctx) -> Result<(), String> {
    let rig = open(
        ctx,
        Config::temp().bar_sys_period(std::time::Duration::from_millis(300)),
        |_| {},
    )?;
    wait_loaded(&rig)?;
    let t = bar_texts(&rig.shell);
    for want in [
        "Disk", "CPU", "GPU", "RAM", "Temp.", "1.4×", "0.4×", "22%", "82%",
    ] {
        if !t.iter().any(|x| x.contains(want)) {
            return Err(format!("the bar has no '{want}': {t:?}"));
        }
    }
    // the disk is live: the caption value changes over time
    let disk = |s: &Shell| bar_texts(s).into_iter().find(|x| x.starts_with("Disk"));
    let first = disk(&rig.shell);
    settle("disk value changed", WAIT, || disk(&rig.shell) != first)?;
    rig.shell.window().destroy();
    Ok(())
}

fn clock_injected(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx, Config::temp().no_ticker(), |o| {
        o.unix_clock = Some(fixed_clock);
        o.zone = Some(Zone::fixed(7200));
    })?;
    settle("clock and date from the injected clock", WAIT, || {
        has_text(&rig.shell, "16:05:09") && has_text(&rig.shell, "Mon, Nov 2")
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn evil_label(ctx: &Ctx) -> Result<(), String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fake-agentd/scenarios/bar-evil-label.toml");
    let sc = Scenario::from_file(&path).map_err(|e| e.to_string())?;
    let rig = open(ctx, Config::temp().no_ticker().scenario(sc), |_| {})?;
    let evil = "<img src=x onerror=alert(1)>";
    settle("quota with a hostile name", WAIT, || {
        let q = rig.shell.quota_button(1);
        q.is_some_and(|b| b.tooltip_text().is_some_and(|t| t.contains(evil)))
    })?;
    // no caption of the bar or dialogs parses markup
    fn markup_labels(w: &gtk::Widget, bad: &mut Vec<String>) {
        if let Ok(l) = w.clone().downcast::<gtk::Label>()
            && l.uses_markup()
        {
            bad.push(l.text().to_string());
        }
        let mut c = w.first_child();
        while let Some(x) = c {
            markup_labels(&x, bad);
            c = x.next_sibling();
        }
    }
    let mut bad = Vec::new();
    markup_labels(&rig.shell.bar(), &mut bad);
    if !bad.is_empty() {
        return Err(format!("captions with markup parsing: {bad:?}"));
    }
    rig.shell.window().destroy();
    Ok(())
}

fn offline_grey(ctx: &Ctx) -> Result<(), String> {
    let rig = open(
        ctx,
        Config::temp().bar_sys_period(std::time::Duration::from_millis(300)),
        |_| {},
    )?;
    wait_loaded(&rig)?;
    rig.d.drop_ws();
    settle("'No connection' banner visible", WAIT, || {
        crate::bar_find::label_with_text(&rig.shell.bar(), "No connection")
            .is_some_and(|l| l.is_visible())
    })?;
    settle("metrics greyed out", WAIT, || {
        crate::bar_find::labels_with_class(&rig.shell.bar(), "metric")
            .iter()
            .all(|l| l.has_css_class("lvl-nodata"))
    })?;
    rig.d.restore();
    settle("data came back", WAIT, || {
        has_text(&rig.shell, "38%")
            && !crate::bar_find::label_with_text(&rig.shell.bar(), "No connection")
                .is_some_and(|l| l.is_visible())
    })?;
    rig.shell.window().destroy();
    Ok(())
}

/// The core does not start (`core_down`): a separate "The core does not start" banner, "No connection" is not shown.
fn core_unavailable(ctx: &Ctx) -> Result<(), String> {
    let rig = open(
        ctx,
        Config::temp().bar_sys_period(std::time::Duration::from_millis(300)),
        |_| {},
    )?;
    wait_loaded(&rig)?;
    let visible = |text: &str| {
        crate::bar_find::label_with_text(&rig.shell.bar(), text).is_some_and(|l| l.is_visible())
    };
    rig.d.set_mode(CoreMode::Down);
    rig.d.drop_ws();
    settle("'The core does not start' banner visible", WAIT, || {
        visible("The core does not start")
    })?;
    if visible("No connection") {
        return Err("'No connection' shown on core_down".into());
    }
    rig.d.set_mode(CoreMode::Ok);
    rig.d.restore();
    settle("banner gone, data came back", WAIT, || {
        has_text(&rig.shell, "38%")
            && !visible("The core does not start")
            && !visible("No connection")
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn width_1024(ctx: &Ctx) -> Result<(), String> {
    let rig = open_w(ctx, Config::temp().no_ticker(), 1024, |_| {})?;
    // the minimum width is computed before the window gets its size: the window cannot be narrower than the minimum
    settle("quotas and metrics in the bar", WAIT, || {
        has_text(&rig.shell, "38%") && has_text(&rig.shell, "Disk")
    })?;
    let bar = rig.shell.bar();
    let (min, _, _, _) = bar.measure(gtk::Orientation::Horizontal, -1);
    if min > 1024 {
        let mut parts = Vec::new();
        let mut c = bar.first_child();
        while let Some(w) = c {
            let (m, _, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
            parts.push(format!("{}:{m}", w.css_name()));
            c = w.next_sibling();
        }
        return Err(format!(
            "minimum bar width {min} > 1024; children: {}",
            parts.join(" ")
        ));
    }
    settle("window width 1024", WAIT, || {
        rig.shell.window().width() == 1024
    })?;
    if bar.width() != 1024 {
        return Err(format!("bar width {} with a 1024 window", bar.width()));
    }
    // all content is inside the bar: no visible child goes beyond the width
    let mut child = bar.first_child();
    while let Some(w) = child {
        if w.is_visible() {
            let b = w.compute_bounds(&bar).ok_or("no child bounds")?;
            if b.x() < -0.5 || b.x() + b.width() > 1024.5 {
                return Err(format!(
                    "bar child outside the width: x {} w {}",
                    b.x(),
                    b.width()
                ));
            }
        }
        child = w.next_sibling();
    }
    rig.shell.window().destroy();
    Ok(())
}

fn contrast(ctx: &Ctx) -> Result<(), String> {
    crate::bar_contrast::run(ctx)
}

fn accessibility_tree(ctx: &Ctx) -> Result<(), String> {
    crate::bar_a11y::run(ctx)
}
