//! `ui_a11y_columns`: names of columns and collapse buttons in the accessibility tree of a real process (AT-SPI).

use std::path::Path;
use std::process::Command;

use controllers::i18n::{tr, tr_args};

use crate::ctx::{Ctx, Scenario};
use crate::weston::Weston;

/// How GTK 4.14 shows the `Region` role in AT-SPI (checked on the tree of a real process: the region name is kept).
const REGION_ROLE: &str = "filler";

struct Node {
    role: String,
    name: String,
}

fn dump_tree(weston: &Weston) -> Result<Vec<Node>, String> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenarios/a11y_dump.py");
    let mut cmd = Command::new("dbus-run-session");
    cmd.args(["--", "python3", "-I"])
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_agent-shell"));
    weston.apply_env(&mut cmd);
    cmd.env("GTK_A11Y", "atspi");
    let out = cmd
        .output()
        .map_err(|e| format!("dbus-run-session did not start: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "a11y_dump.py exited with {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let nodes = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut parts = l.splitn(3, '\t');
            let (_depth, role, name) = (parts.next()?, parts.next()?, parts.next()?);
            Some(Node {
                role: role.to_string(),
                name: name.to_string(),
            })
        })
        .collect();
    Ok(nodes)
}

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![("ui_a11y_columns", ui_a11y_columns)]
}

fn ui_a11y_columns(ctx: &Ctx) -> Result<(), String> {
    let nodes = dump_tree(&ctx.weston)?;
    let described = |role: &str, name: &str| nodes.iter().any(|n| n.role == role && n.name == name);
    let mut missing = Vec::new();
    for key in [
        "column-notifications",
        "column-sessions",
        "column-chat",
        // the tools column is visible only with an open session: its region is checked by `ui_tools_a11y`
    ] {
        let name = tr(key);
        if !described(REGION_ROLE, &name) {
            missing.push(format!("region '{name}'"));
        }
        let collapse = tr_args("column-collapse", &[("column", &name)]);
        if key != "column-chat" && !described("push-button", &collapse) {
            missing.push(format!("button '{collapse}'"));
        }
    }
    if !described(REGION_ROLE, &tr("bar-name")) {
        missing.push(format!("bar region '{}'", tr("bar-name")));
    }
    if missing.is_empty() {
        Ok(())
    } else {
        let tree: Vec<String> = nodes
            .iter()
            .map(|n| format!("{} {:?}", n.role, n.name))
            .collect();
        Err(format!(
            "not in the accessibility tree: {}\ntree ({} nodes):\n{}",
            missing.join(", "),
            nodes.len(),
            tree.join("\n")
        ))
    }
}
