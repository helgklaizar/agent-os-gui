//! User terminal (VTE) and agent output panel in real GTK under headless Wayland.

use std::time::Duration;

use controllers::Tab;
use gtk4::prelude::*;
use serde_json::json;

use crate::ctx::{Ctx, Scenario, WAIT, settle};
use crate::tools::{open_tab, two_sessions};
use crate::tree::{open_wide as open, wait_loaded};

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![
        (
            "ui_terminal_runs_in_session_folder_without_token",
            runs_in_folder,
        ),
        ("ui_terminal_is_kept_per_session", kept_per_session),
        ("ui_terminal_agent_panel_shows_kernel_output", agent_panel),
    ]
}

fn read(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok().filter(|t| !t.is_empty())
}

fn runs_in_folder(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, _) = two_sessions(ctx, &rig)?;
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = work.path().to_string_lossy().into_owned();
    rig.call(ctx, rig.http.patch_session(a, json!({ "path": path })))
        .map_err(|e| e.to_string())?;
    // the session folder reaches the controllers in a `sessions` frame; the terminal starts when the tab is shown
    open_tab(&rig, a, Tab::Chat)?;
    settle("session path in the frame", WAIT, || {
        rig.shell.settings_view().path == path
    })?;
    open_tab(&rig, a, Tab::Terminal)?;
    let pane = rig.shell.terminal_pane();
    settle("shell started", Duration::from_secs(15), || {
        rig.shell.terminal_view().user.is_some_and(|u| u.running)
    })?;
    let shown = pane.cwd.text().to_string();
    let user = rig.shell.terminal_view().user.ok_or("no terminal")?;
    if user.cwd.to_string_lossy() != path && shown != path {
        return Err(format!("terminal folder {:?}, expected {path}", user.cwd));
    }
    let (pwd, env) = (work.path().join("pwd.txt"), work.path().join("env.txt"));
    pane.type_text(
        a,
        &format!("pwd > {}; env > {}\n", pwd.display(), env.display()),
    );
    settle(
        "command executed in the terminal",
        Duration::from_secs(15),
        || read(&pwd).is_some() && read(&env).is_some_and(|t| t.contains("PATH=")),
    )?;
    let got = read(&pwd).unwrap_or_default();
    if got.trim() != path {
        return Err(format!("working folder {:?}, expected {path}", got.trim()));
    }
    let env = read(&env).unwrap_or_default();
    for secret in [
        "AGENTD",
        "TOKEN",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        if env
            .lines()
            .any(|l| l.starts_with(secret) || l.contains(&format!("{secret}=")))
        {
            return Err(format!("the user terminal environment contains {secret}"));
        }
    }
    if !env.contains("HOME=") {
        return Err("no HOME in the environment".into());
    }
    rig.shell.window().destroy();
    Ok(())
}

fn kept_per_session(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, b) = two_sessions(ctx, &rig)?;
    open_tab(&rig, a, Tab::Terminal)?;
    settle("first session shell", Duration::from_secs(15), || {
        rig.shell.terminal_view().live == vec![a]
    })?;
    open_tab(&rig, b, Tab::Terminal)?;
    settle("second session shell", Duration::from_secs(15), || {
        rig.shell.terminal_view().live.len() == 2
    })?;
    open_tab(&rig, a, Tab::Terminal)?;
    settle("first session in place", WAIT, || {
        rig.shell
            .terminal_view()
            .user
            .is_some_and(|u| u.session_id == a && u.running && !u.needs_spawn)
    })?;
    if !rig.shell.terminal_pane().has_terminal(a) || !rig.shell.terminal_pane().has_terminal(b) {
        return Err("the session terminal was not kept".into());
    }
    rig.shell.window().destroy();
    Ok(())
}

fn agent_panel(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, _) = two_sessions(ctx, &rig)?;
    rig.call(ctx, rig.http.put_permission("global", 0, "shell", "allow"))
        .map_err(|e| e.to_string())?;
    open_tab(&rig, a, Tab::Terminal)?;
    let pane = rig.shell.terminal_pane();
    settle("user shell", Duration::from_secs(15), || {
        rig.shell.terminal_view().user.is_some_and(|u| u.running)
    })?;
    let cid = uuid::Uuid::now_v7().to_string();
    let evil = "echo <b>x</b> <img src=x onerror=alert(1)>";
    rig.call(
        ctx,
        rig.http
            .send_message(a, &cid, &format!("/ask shell {evil}"), &[]),
    )
    .map_err(|e| e.to_string())?;
    settle("agent output in the panel", WAIT, || {
        let t = pane.agent_buffer_text();
        t.contains("done") && t.contains(evil)
    })
    .map_err(|e| format!("{e}; panel: {:?}", pane.agent_buffer_text()))?;
    let view = rig.shell.terminal_view();
    if view.agent.first().map(|r| r.status.as_str()) != Some("exited") {
        return Err(format!(
            "command status {:?}",
            view.agent.first().map(|r| &r.status)
        ));
    }
    if pane.agent_text.is_editable() {
        return Err("the agent panel must be read-only".into());
    }
    if !view.user.is_some_and(|u| u.running) {
        return Err("the agent command affected the user terminal".into());
    }
    rig.shell.window().destroy();
    Ok(())
}
