//! `ui_tools_accessibility`: names of tabs and browser buttons, the permission-weakening field as unavailable, the AT-SPI tree
//! of a real GTK process. The outer scenario restarts the runner under `dbus-run-session` with `GTK_A11Y=atspi`
//! (the same trick as `ui_tree_selected_state`); the inner one reads its own tree via `a11y_pid.py`.

use std::process::Command;

use controllers::Tab;
use controllers::i18n::tr;
use gtk4::prelude::*;

use crate::ctx::{Ctx, Scenario, WAIT, settle};
use crate::tools::{open_tab, two_sessions};
use crate::tree::{open_wide as open, wait_loaded};
use crate::tree_selected::{Node, dump};

const CHILD: &str = "ui_tools_accessibility_inner";
const FLAG: &str = "S6_A11Y_CHILD";

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    let mut list: Vec<(&'static str, Scenario)> = vec![("ui_tools_accessibility", outer)];
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

fn named<'a>(nodes: &'a [Node], role: &str, name: &str) -> Option<&'a Node> {
    nodes.iter().find(|n| n.role == role && n.name == name)
}

fn inner(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, _) = two_sessions(ctx, &rig)?;
    open_tab(&rig, a, Tab::Browser)?;
    settle("column shown", WAIT, || {
        rig.shell.column(ui::shell::ColumnId::Tools).is_mapped()
    })?;
    let nodes = dump()?;
    let mut missing = Vec::new();
    let region = tr("column-tools");
    if !nodes.iter().any(|n| n.name == region) {
        missing.push(format!("region '{region}'"));
    }
    for key in [
        "tools-tab-browser",
        "tools-tab-terminal",
        "tools-tab-settings",
    ] {
        let name = tr(key);
        if named(&nodes, "toggle-button", &name).is_none() {
            missing.push(format!("tab '{name}'"));
        }
    }
    for key in [
        "browser-back",
        "browser-forward",
        "browser-reload",
        "browser-home",
        "browser-bookmark-add",
    ] {
        let name = tr(key);
        if named(&nodes, "push-button", &name).is_none() {
            missing.push(format!("button '{name}'"));
        }
    }
    if !nodes.iter().any(|n| n.name == tr("browser-bookmarks")) {
        missing.push(format!("«{}»", tr("browser-bookmarks")));
    }
    if !nodes.iter().any(|n| n.name == tr("browser-address-name")) {
        missing.push(format!("field '{}'", tr("browser-address-name")));
    }
    if !missing.is_empty() {
        return Err(format!(
            "not in the accessibility tree: {}",
            missing.join(", ")
        ));
    }
    // from the keyboard: tabs and buttons take focus
    for b in rig
        .shell
        .tools_widgets()
        .buttons
        .iter()
        .map(|(_, b)| b.clone())
    {
        if !b.grab_focus() {
            return Err("a tab does not take focus".into());
        }
    }
    // the permission-weakening field is unavailable in the tree
    rig.call(ctx, rig.http.put_permission("global", 0, "shell", "deny"))
        .map_err(|e| e.to_string())?;
    open_tab(&rig, a, Tab::SessionSettings)?;
    let allow = rig
        .shell
        .settings_pane()
        .perms
        .mode_button("shell", controllers::permissions::Mode::Allow)
        .ok_or("no allow button")?;
    settle("field locked", WAIT, || !allow.is_sensitive())?;
    // for a button with a caption the tree name is its text ("allow"); fields follow PERM_ACTIONS order: send, delete, shell
    let nodes = dump()?;
    let allow = tr("chat-mode-allow");
    let allows: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.role == "toggle-button" && n.name == allow)
        .collect();
    let [send, _delete, shell] = allows.as_slice() else {
        return Err(format!(
            "'{allow}' buttons {}, expected 3 (one field per permission)",
            allows.len()
        ));
    };
    if shell.states.iter().any(|s| s == "sensitive") {
        return Err(format!(
            "a weakening value under a global prohibition is not marked unavailable: {:?}",
            shell.states
        ));
    }
    if !send.states.iter().any(|s| s == "sensitive") {
        return Err(format!(
            "the send permission without a higher prohibition must be available: {:?}",
            send.states
        ));
    }
    rig.shell.window().destroy();
    Ok(())
}
