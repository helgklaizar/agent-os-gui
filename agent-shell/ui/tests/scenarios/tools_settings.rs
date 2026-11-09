//! Session settings in real GTK under headless Wayland: values reach the core, permissions are not weakened,
//! clearing and deletion only with confirmation, tasks, hostile text.

use std::time::Duration;

use controllers::Tab;
use controllers::permissions::Mode;
use gtk4 as gtk;
use gtk4::prelude::*;
use serde_json::json;

use crate::ctx::{Ctx, Scenario, WAIT, settle};
use crate::tools::{open_tab, two_sessions};
use crate::tree::{Rig, find_all, labels, open_wide as open, wait_loaded};

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![
        ("ui_session_settings_values_reach_core", values_reach_core),
        ("ui_session_perms_cannot_loosen", perms_cannot_loosen),
        ("ui_session_settings_confirmations", confirmations),
        ("ui_session_tasks_create_pause_run", tasks),
        ("ui_session_settings_text_is_literal", text_is_literal),
    ]
}

fn session_cfg(ctx: &Ctx, rig: &Rig, id: i64) -> serde_json::Value {
    let list = rig.call(ctx, rig.http.list_sessions()).unwrap_or_default();
    list.into_iter().find(|s| s["id"] == id).unwrap_or_default()
}

fn button_named(rig: &Rig, label: &str) -> Result<gtk::Button, String> {
    find_all::<gtk::Button>(rig.shell.settings_pane().root.upcast_ref())
        .into_iter()
        .find(|b| b.label().is_some_and(|l| l == label))
        .ok_or_else(|| format!("no button '{label}'"))
}

fn values_reach_core(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, _) = two_sessions(ctx, &rig)?;
    open_tab(&rig, a, Tab::SessionSettings)?;
    let pane = rig.shell.settings_pane();
    settle("session name in settings", WAIT, || {
        !rig.shell.settings_view().name.is_empty()
    })?;
    pane.mem.set_active(true);
    pane.notif.set_selected(2);
    pane.path.set_text("/work/ui-test");
    pane.path.emit_activate();
    settle("values in the core", WAIT, || {
        let s = session_cfg(ctx, &rig, a);
        s["cfg"]["mem"] == true && s["cfg"]["notif"] == "off" && s["path"] == "/work/ui-test"
    })?;
    // the core frame returns to the view without echo and without losing values
    settle("view matches the core", WAIT, || {
        let v = rig.shell.settings_view();
        v.mem && v.path == "/work/ui-test"
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn perms_cannot_loosen(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, _) = two_sessions(ctx, &rig)?;
    rig.call(ctx, rig.http.put_permission("global", 0, "shell", "deny"))
        .map_err(|e| e.to_string())?;
    open_tab(&rig, a, Tab::SessionSettings)?;
    let perms = rig.shell.settings_pane().perms.clone();
    let allow = perms
        .mode_button("shell", Mode::Allow)
        .ok_or("no allow button")?;
    let deny = perms
        .mode_button("shell", Mode::Deny)
        .ok_or("no deny button")?;
    settle("field locked", WAIT, || {
        !allow.is_sensitive() && !deny.is_sensitive()
    })?;
    // bypassing the view: programmatically enabling an unavailable value does not reach the core
    allow.set_active(true);
    settle("reason shown", WAIT, || {
        rig.shell.settings_view().notice.is_some()
    })?;
    std::thread::sleep(Duration::from_millis(300));
    let rows = rig
        .call(ctx, rig.http.list_all_permissions())
        .map_err(|e| e.to_string())?;
    if rows
        .as_array()
        .is_some_and(|a| a.iter().any(|r| r["scope"] == "session"))
    {
        return Err(format!("a session rule was created: {rows}"));
    }
    // a permitted tightening under a looser higher level
    let send_deny = perms
        .mode_button("send", Mode::Deny)
        .ok_or("no send/deny button")?;
    settle("deny available", WAIT, || send_deny.is_sensitive())?;
    send_deny.set_active(true);
    settle("tightening in the core", WAIT, || {
        rig.call(ctx, rig.http.list_all_permissions())
            .is_ok_and(|r| {
                r.as_array().is_some_and(|a| {
                    a.iter().any(|x| {
                        x["scope"] == "session" && x["action"] == "send" && x["mode"] == "deny"
                    })
                })
            })
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn confirmations(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, _) = two_sessions(ctx, &rig)?;
    let cid = uuid::Uuid::now_v7().to_string();
    rig.call(ctx, rig.http.send_message(a, &cid, "message to clear", &[]))
        .map_err(|e| e.to_string())?;
    open_tab(&rig, a, Tab::SessionSettings)?;
    let pane = rig.shell.settings_pane();
    settle("there are messages", WAIT, || {
        rig.call(ctx, rig.http.list_messages(a, None, 50))
            .is_ok_and(|m| !m.is_empty())
    })?;
    pane.clear.emit_clicked();
    settle("clear confirmation", WAIT, || pane.confirm_visible()).map_err(|e| {
        format!(
            "{e}; view: {:?}; page {} stack {} column {} session {:?}",
            rig.shell.settings_view().confirm,
            pane.root.is_visible(),
            rig.shell.tools_widgets().stack.is_visible(),
            rig.shell.column(ui::shell::ColumnId::Tools).is_visible(),
            rig.shell.settings_view().session_id,
        ) + &{
            let mut chain = String::new();
            let mut w: Option<gtk::Widget> = Some(pane.root.clone().upcast());
            while let Some(x) = w {
                chain.push_str(&format!(" [{} v={}]", x.type_().name(), x.is_visible()));
                w = x.parent();
            }
            format!(
                "; collapsed={}; chain:{chain}",
                rig.shell.tools_view().collapsed
            )
        }
    })?;
    let lines = pane.confirm_text();
    if lines.len() < 3 {
        return Err(format!("the confirmation has no list: {lines:?}"));
    }
    if rig
        .call(ctx, rig.http.list_messages(a, None, 50))
        .map_or(true, |m| m.is_empty())
    {
        return Err("clearing was done before confirmation".into());
    }
    button_named(&rig, "Cancel")?.emit_clicked();
    settle("confirmation closed", WAIT, || !pane.confirm_visible())?;
    if rig
        .call(ctx, rig.http.list_messages(a, None, 50))
        .map_or(true, |m| m.is_empty())
    {
        return Err("cancel cleared the history".into());
    }
    pane.clear.emit_clicked();
    settle("clear confirmation", WAIT, || pane.confirm_visible())?;
    button_named(&rig, "Confirm")?.emit_clicked();
    settle("history cleared", WAIT, || {
        rig.call(ctx, rig.http.list_messages(a, None, 50))
            .is_ok_and(|m| m.is_empty())
    })?;
    pane.delete.emit_clicked();
    settle("delete confirmation", WAIT, || pane.confirm_visible())?;
    if !pane.confirm_text().iter().any(|l| l.contains("threads")) {
        return Err(format!(
            "the delete list has no threads: {:?}",
            pane.confirm_text()
        ));
    }
    button_named(&rig, "Confirm")?.emit_clicked();
    settle(
        "session deleted, tools moved to the neighbouring one",
        WAIT,
        || {
            rig.shell.tools_view().session != Some(a)
                && rig
                    .call(ctx, rig.http.list_sessions())
                    .is_ok_and(|l| l.iter().all(|s| s["id"] != a))
        },
    )
    .map_err(|e| {
        let ids: Vec<i64> = rig
            .call(ctx, rig.http.list_sessions())
            .unwrap_or_default()
            .iter()
            .filter_map(|s| s["id"].as_i64())
            .collect();
        format!(
            "{e}; visible {}; sessions in the core {ids:?}, deleted {a}; view {:?}",
            rig.shell.tools_view().visible,
            rig.shell.tools_view().session
        )
    })?;
    rig.shell.window().destroy();
    Ok(())
}

fn tasks(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let (a, _) = two_sessions(ctx, &rig)?;
    open_tab(&rig, a, Tab::SessionSettings)?;
    let tasks = rig.shell.settings_pane().tasks.clone();
    tasks.kind.set_selected(0);
    tasks.title.set_text("Morning report");
    tasks.cron.set_text("0 9 * * *");
    tasks.create.emit_clicked();
    settle("task in the list", WAIT, || tasks.task_rows().len() == 1)?;
    if !tasks.task_rows()[0].contains("Asia/Jerusalem") {
        return Err(format!("no time zone in the row: {:?}", tasks.task_rows()));
    }
    button_named(&rig, "Pause")?.emit_clicked();
    settle("task paused", WAIT, || {
        rig.shell
            .settings_view()
            .tasks
            .iter()
            .all(|t| t.state == "paused")
    })?;
    button_named(&rig, "Run")?.emit_clicked();
    settle("recent runs", WAIT, || !tasks.runs_text().is_empty())?;
    // a title with markup is not parsed
    tasks.kind.set_selected(1);
    tasks.title.set_text("<b>bold</b>");
    tasks.cron.set_text("");
    tasks.create.emit_clicked();
    settle("second task", WAIT, || tasks.task_rows().len() == 2)?;
    if !tasks.task_rows().iter().any(|r| r.contains("<b>bold</b>")) {
        return Err(format!("title distorted: {:?}", tasks.task_rows()));
    }
    rig.shell.window().destroy();
    Ok(())
}

fn text_is_literal(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let evil = "<img src=x onerror=alert(1)>";
    let made = rig
        .call(ctx, rig.http.create_session(evil, None, None))
        .map_err(|e| e.to_string())?;
    let id = made["id"].as_i64().ok_or("no id")?;
    rig.call(
        ctx,
        rig.http.patch_session(id, json!({"path": "/tmp/<b>x</b>"})),
    )
    .map_err(|e| e.to_string())?;
    open_tab(&rig, id, Tab::SessionSettings)?;
    let pane = rig.shell.settings_pane();
    settle("name literal", WAIT, || pane.name.text() == evil)?;
    if pane.name.uses_markup() {
        return Err("the name label parses markup".into());
    }
    for l in find_all::<gtk::Label>(pane.root.upcast_ref()) {
        if l.uses_markup() {
            return Err(format!("label {:?} parses markup", l.text()));
        }
    }
    if !labels(&pane.root).iter().any(|t| t == evil) {
        return Err("name not shown".into());
    }
    pane.delete.emit_clicked();
    settle("confirmation", WAIT, || pane.confirm_visible())?;
    if !pane.confirm_text().iter().any(|l| l.contains(evil)) {
        return Err("the name in the confirmation is distorted".into());
    }
    rig.shell.window().destroy();
    Ok(())
}
