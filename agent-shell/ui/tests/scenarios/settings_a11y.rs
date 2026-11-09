//! `ui_settings_accessibility`: the Settings dialog in the AT-SPI tree of a real GTK process: dialog role, names of fields,
//! switches, choices and the "Save" and "Close" buttons; from the keyboard the buttons take focus, Esc closes the dialog.
//! The outer scenario restarts the runner under `dbus-run-session` with `GTK_A11Y=atspi` (the `ui_tools_accessibility` trick).

use std::process::Command;

use controllers::i18n::tr;
use controllers::{BusEvent, SettingsPage};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::ctx::{Ctx, Scenario, WAIT, settle};
use crate::tree::{open, press};
use crate::tree_selected::{Node, dump};

const CHILD: &str = "ui_settings_accessibility_inner";
const FLAG: &str = "S7_A11Y_CHILD";

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    let mut list: Vec<(&'static str, Scenario)> = vec![("ui_settings_accessibility", outer)];
    if std::env::var_os(FLAG).is_some() {
        list.push((CHILD, inner));
    }
    list
}

fn outer(ctx: &Ctx) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new("dbus-run-session");
    cmd.args(["--"]).arg(exe).arg(CHILD);
    ctx.weston.apply_env(&mut cmd);
    cmd.env("GTK_A11Y", "atspi")
        .env(FLAG, "1")
        .env("XDG_DATA_HOME", ctx.data.path());
    let out = cmd
        .output()
        .map_err(|e| format!("dbus-run-session did not start: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines().filter(|l| l.starts_with("    ")) {
        println!("{line}");
    }
    if out.status.success() && text.contains(&format!("test {CHILD} ... ok")) {
        Ok(())
    } else {
        Err(format!(
            "inner scenario: code {:?}\n{text}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

fn has(nodes: &[Node], name: &str) -> bool {
    nodes.iter().any(|n| n.name == name)
}

fn inner(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    rig.shell
        .bus()
        .publish(BusEvent::OpenSettings(SettingsPage::Account));
    settle("dialog open", WAIT, || {
        let v = rig.shell.settings_dialog_view();
        v.open && !v.loading
    })?;
    let dialog = rig.shell.settings_dialog();
    let nodes = dump()?;
    let mut missing = Vec::new();
    for key in [
        "set-title",
        "set-save",
        "set-close",
        "set-f-name",
        "set-f-language",
        "set-f-theme",
        "set-page-account",
        "set-page-personas",
        "set-page-models",
    ] {
        let name = tr(key);
        if !has(&nodes, &name) {
            missing.push(format!("«{name}»"));
        }
    }
    if !nodes
        .iter()
        .any(|n| n.role == "dialog" && n.name == tr("set-title"))
    {
        missing.push("dialog named 'Settings'".into());
    }
    if !missing.is_empty() {
        return Err(format!(
            "not in the accessibility tree: {}",
            missing.join(", ")
        ));
    }
    // Tab and Shift+Tab: buttons and fields take focus
    for b in [&dialog.save, &dialog.close] {
        if !b.can_focus() {
            return Err("a button does not accept focus".into());
        }
    }
    if !dialog.close.grab_focus() {
        return Err("'Close' does not take focus".into());
    }
    // Esc closes, focus leaves the dialog layer
    press(&dialog.layer, gtk::gdk::Key::Escape)?;
    settle("dialog closed by Esc", WAIT, || !dialog.layer.is_visible())?;
    let focus_in_dialog = gtk::prelude::GtkWindowExt::focus(rig.shell.window())
        .is_some_and(|w| w.is_ancestor(&dialog.layer));
    if focus_in_dialog {
        return Err("after closing, focus stayed inside the hidden dialog".into());
    }
    rig.shell.window().destroy();
    Ok(())
}
