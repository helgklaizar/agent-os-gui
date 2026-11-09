//! Creation dialogs in the "Sessions" column in real GTK under headless Wayland.

use controllers::TreeButton;
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::ctx::{Ctx, Scenario as Run, WAIT, settle};
use crate::tree::{Rig, find_all, open, press_escape, tooltip_button, wait_loaded};

pub fn scenarios() -> Vec<(&'static str, Run)> {
    vec![("ui_create_dialogs", create_dialogs)]
}

fn type_name(rig: &Rig, text: &str) -> Result<(), String> {
    let layer = rig.shell.dialog_layer();
    let entry = find_all::<gtk::Entry>(layer.upcast_ref())
        .into_iter()
        .next()
        .ok_or("no name field in the dialog")?;
    entry.set_text(text);
    Ok(())
}

fn submit_button(rig: &Rig, label: &str) -> Result<gtk::Button, String> {
    rig.shell
        .dialog_button(label)
        .ok_or_else(|| format!("no button '{label}'"))
}

fn create_dialogs(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    // session: an empty name blocks "Create"
    rig.shell.tree().new_session.emit_clicked();
    settle("session dialog open", WAIT, || rig.shell.dialog_open())?;
    let create = submit_button(&rig, "Create")?;
    if create.is_sensitive() {
        return Err("'Create' is available with an empty name".into());
    }
    let texts = rig.shell.dialog_texts();
    for want in [
        "New session",
        "Name",
        "Folder",
        "No folder",
        "Persona",
        "Same as folder / default",
    ] {
        if !texts.iter().any(|t| t == want) {
            return Err(format!("the session dialog has no '{want}': {texts:?}"));
        }
    }
    type_name(&rig, "From dialog")?;
    settle("'Create' available", WAIT, || {
        submit_button(&rig, "Create").is_ok_and(|b| b.is_sensitive())
    })?;
    submit_button(&rig, "Create")?.emit_clicked();
    settle("dialog closed", WAIT, || !rig.shell.dialog_open())?;
    settle("session in the tree", WAIT, || {
        rig.node("From dialog").is_some()
    })?;
    settle("focus returned to the button", WAIT, || {
        rig.shell.tree().new_session.is_focus()
    })?;
    // folder
    rig.shell.tree().new_folder.emit_clicked();
    settle("folder dialog open", WAIT, || rig.shell.dialog_open())?;
    if !rig.shell.dialog_texts().iter().any(|t| t == "New folder") {
        return Err("no 'New folder' title".into());
    }
    if submit_button(&rig, "Create")?.is_sensitive() {
        return Err("'Create' is available for a folder with an empty name".into());
    }
    type_name(&rig, "Folder from dialog")?;
    submit_button(&rig, "Create")?.emit_clicked();
    settle("folder in the tree", WAIT, || {
        rig.node("Folder from dialog").is_some()
    })?;
    // thread via a row action
    let session = rig.node("Release 1.2").ok_or("no 'Release 1.2'")?;
    tooltip_button(&rig.row_widget(session)?, "New thread")?.emit_clicked();
    settle("thread dialog open", WAIT, || rig.shell.dialog_open())?;
    let texts = rig.shell.dialog_texts();
    if texts.iter().any(|t| t == "Persona" || t == "Folder") {
        return Err(format!(
            "the thread dialog has a folder or persona choice: {texts:?}"
        ));
    }
    type_name(&rig, "Thread from dialog")?;
    submit_button(&rig, "Create")?.emit_clicked();
    settle("thread in the tree", WAIT, || {
        rig.node("Thread from dialog").is_some()
    })?;
    // Esc closes without changes, focus returns to the row
    let rows = rig.rows();
    tooltip_button(&rig.row_widget(session)?, "Rename")?.emit_clicked();
    settle("rename dialog", WAIT, || rig.shell.dialog_open())?;
    type_name(&rig, "Must not be saved")?;
    press_escape(&rig.shell)?;
    settle("dialog closed by Esc", WAIT, || !rig.shell.dialog_open())?;
    settle("focus returned to the row", WAIT, || {
        rig.row_widget(session).is_ok_and(|w| w.is_focus())
    })?;
    if rig.rows() != rows || rig.node("Must not be saved").is_some() {
        return Err("Esc changed the tree".into());
    }
    // deletion: a confirmation with a list
    tooltip_button(&rig.row_widget(session)?, "Delete")?.emit_clicked();
    settle("delete dialog", WAIT, || rig.shell.dialog_open())?;
    let texts = rig.shell.dialog_texts();
    if !texts.iter().any(|t| t.contains("Release 1.2"))
        || !texts.iter().any(|t| t.contains("3 threads"))
    {
        return Err(format!(
            "the confirmation lacks the session and the thread count: {texts:?}"
        ));
    }
    submit_button(&rig, "Delete")?.emit_clicked();
    settle("session deleted", WAIT, || {
        rig.node("Release 1.2").is_none() && rig.node("Thread from dialog").is_none()
    })?;
    let _ = TreeButton::NewSession;
    rig.shell.window().destroy();
    Ok(())
}
