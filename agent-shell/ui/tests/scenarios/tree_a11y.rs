//! `ui_tree_accessibility`: roles, names and states of session tree rows in the AT-SPI tree of a real `agent-shell` process
//! (token from a file, `fake-agentd` data; the test build enables the `test-token-file` feature).

use std::path::Path;
use std::process::Command;

use controllers::i18n::tr;
use fake_agentd::{Config, FakeAgentd};

use crate::ctx::{Ctx, Scenario};

pub(crate) struct Node {
    pub(crate) role: String,
    pub(crate) name: String,
    pub(crate) states: String,
}

pub(crate) fn dump(
    ctx: &Ctx,
    d: &FakeAgentd,
    token_file: &Path,
    cfg: &Path,
) -> Result<Vec<Node>, String> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenarios/a11y_dump.py");
    let mut cmd = Command::new("dbus-run-session");
    cmd.args(["--", "python3", "-I"])
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_agent-shell"))
        .arg("--states");
    ctx.weston.apply_env(&mut cmd);
    cmd.env("GTK_A11Y", "atspi")
        .env("AGENTD_SOCKET", d.socket_path())
        .env("AGENTD_TOKEN_FILE", token_file)
        .env("XDG_CONFIG_HOME", cfg);
    let out = cmd
        .output()
        .map_err(|e| format!("dbus-run-session did not start: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "a11y_dump.py: code {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut p = l.splitn(4, '\t');
            let (_depth, role, name, states) = (p.next()?, p.next()?, p.next()?, p.next()?);
            Some(Node {
                role: role.to_string(),
                name: name.to_string(),
                states: states.to_string(),
            })
        })
        .collect())
}

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![("ui_tree_accessibility", run)]
}

fn run(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let token_file = dir.path().join("gui.token");
    let config = Config::temp().no_ticker().token_file(token_file.clone());
    let d = ctx
        .rt
        .block_on(FakeAgentd::start(config))
        .map_err(|e| e.to_string())?;
    let nodes = dump(ctx, &d, &token_file, dir.path())?;
    let mut missing = Vec::new();
    let items: Vec<&Node> = nodes.iter().filter(|n| n.role == "tree-item").collect();
    if !nodes
        .iter()
        .any(|n| n.role == "tree" && n.name == tr("column-sessions"))
    {
        missing.push("tree 'Sessions' with the tree role".to_string());
    }
    let item = |prefix: &str| items.iter().find(|n| n.name.starts_with(prefix));
    let folder = format!("{} Clients", tr("tree-a11y-folder"));
    match item(&folder) {
        Some(n) if n.name.contains(&tr("tree-a11y-expanded")) => {}
        other => missing.push(format!(
            "folder '{folder}, expanded': {:?}",
            other.map(|n| &n.name)
        )),
    }
    let unread = format!("{} Vacation", tr("tree-a11y-session"));
    match item(&unread) {
        Some(n) if n.name.contains("unread: 2") => {}
        other => missing.push(format!(
            "session '{unread}' with an unread count of 2: {:?}",
            other.map(|n| &n.name)
        )),
    }
    let thread = format!("{} Review", tr("tree-a11y-thread"));
    if item(&thread).is_none() {
        missing.push(format!("thread '{thread}'"));
    }
    for n in &items {
        println!("    tree-item {:?} states: {}", n.name, n.states);
    }
    if items.len() < 10 {
        missing.push(format!("10 tree-item rows, found {}", items.len()));
    }
    for button in ["New session", "New folder"] {
        if !nodes
            .iter()
            .any(|n| n.role == "push-button" && n.name == button)
        {
            missing.push(format!("button '{button}'"));
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let tree: Vec<String> = nodes
        .iter()
        .map(|n| format!("{} {:?} [{}]", n.role, n.name, n.states))
        .collect();
    Err(format!(
        "not in the accessibility tree: {}\ntree ({} nodes):\n{}",
        missing.join("; "),
        nodes.len(),
        tree.join("\n")
    ))
}
