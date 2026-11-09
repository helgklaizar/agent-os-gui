//! `ui_tree_selected_state`: the `selected` state of the selected row in the accessibility tree (AT-SPI) of a real GTK process.
//! The outer scenario restarts the same runner under `dbus-run-session` with `GTK_A11Y=atspi`; the inner one selects a row
//! with a click, spins the main loop and reads its own tree via `a11y_pid.py` (the process must answer AT-SPI).

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use controllers::controllers::session_tree::TreeMsg;
use controllers::i18n::tr;
use controllers::tree_types::NodeId;
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::ctx::{Ctx, Scenario, WAIT, settle};
use crate::tree::{open, wait_loaded};

const CHILD: &str = "ui_tree_selected_state_inner";
const FLAG: &str = "S3_A11Y_CHILD";

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    let mut list: Vec<(&'static str, Scenario)> = vec![("ui_tree_selected_state", outer)];
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
    cmd.env("GTK_A11Y", "atspi").env(FLAG, "1");
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

pub(crate) struct Node {
    pub(crate) role: String,
    pub(crate) name: String,
    pub(crate) states: Vec<String>,
}

fn inner(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    let Some(NodeId::Session(id)) = rig.node("Release 1.2") else {
        return Err("no 'Release 1.2'".into());
    };
    let before = dump()?;
    if before
        .iter()
        .any(|n| n.role == "tree-item" && n.states.iter().any(|s| s == "selected"))
    {
        return Err("before selection no row may be selected".into());
    }
    // selection the same way as with the mouse: the `pressed` signal of the row's click gesture
    let row = rig.row_widget(NodeId::Session(id))?;
    let model = row.observe_controllers();
    let click = (0..model.n_items())
        .filter_map(|i| model.item(i))
        .find_map(|o| o.downcast::<gtk::GestureClick>().ok())
        .ok_or("the row has no click gesture")?;
    click.emit_by_name::<()>("pressed", &[&1i32, &4.0f64, &4.0f64]);
    settle("session selected", WAIT, || {
        rig.shell.tree_view().selected == Some(id)
    })?;
    let _ = TreeMsg::Select(id);
    let after = dump()?;
    let items: Vec<&Node> = after.iter().filter(|n| n.role == "tree-item").collect();
    let selected: Vec<&&Node> = items
        .iter()
        .filter(|n| n.states.iter().any(|s| s == "selected"))
        .collect();
    for n in &items {
        println!("    tree-item {:?} states: {}", n.name, n.states.join(","));
    }
    match selected.as_slice() {
        [one]
            if one.name.contains("Release 1.2") && one.name.contains(&tr("tree-a11y-selected")) => {
        }
        other => {
            return Err(format!(
                "expected one selected row 'Release 1.2, selected', found: {:?}",
                other.iter().map(|n| &n.name).collect::<Vec<_>>()
            ));
        }
    }
    rig.shell.window().destroy();
    Ok(())
}

/// The accessibility tree of the same process via `a11y_pid.py`; the main loop spins while the script reads.
pub(crate) fn dump() -> Result<Vec<Node>, String> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenarios/a11y_pid.py");
    let mut child = Command::new("python3")
        .args(["-I"])
        .arg(script)
        .arg(std::process::id().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("python3 did not start: {e}"))?;
    let ctxm = gtk::glib::MainContext::default();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let status = loop {
        while ctxm.iteration(false) {}
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            return Err("a11y_pid.py did not finish within 60 s".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!(
            "a11y_pid.py: code {:?}: {}",
            status.code(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut p = l.splitn(4, '\t');
            let (_d, role, name, states) = (p.next()?, p.next()?, p.next()?, p.next()?);
            Some(Node {
                role: role.to_string(),
                name: name.to_string(),
                states: states.split(',').map(str::to_string).collect(),
            })
        })
        .collect())
}
