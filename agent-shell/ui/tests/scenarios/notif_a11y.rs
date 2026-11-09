//! `ui_notifications_accessibility`: names and roles of filters, badges and feed cards in the AT-SPI tree of a real
//! `agent-shell` process (token from a file, `fake-agentd` data with event fixtures).

use fake_agentd::Config;

use crate::ctx::{Ctx, Scenario};
use crate::tree_a11y::{Node, dump};

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![("ui_notifications_accessibility", run)]
}

fn run(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let token_file = dir.path().join("gui.token");
    let config = Config::temp().no_ticker().token_file(token_file.clone());
    let d = ctx
        .rt
        .block_on(fake_agentd::FakeAgentd::start(config))
        .map_err(|e| e.to_string())?;
    let nodes = dump(ctx, &d, &token_file, dir.path())?;
    let mut missing = Vec::new();
    let button = |name: &str, nodes: &[Node]| {
        nodes
            .iter()
            .any(|n| n.role == "push-button" && n.name == name)
    };
    for name in [
        "All: 11 unread",
        "AI: 4 unread",
        "Tasks: 3 unread",
        "Messages: 4 unread",
    ] {
        if !button(name, &nodes) {
            missing.push(format!("filter button '{name}'"));
        }
    }
    if !nodes
        .iter()
        .any(|n| n.name == "Agent: Reply ready: release 1.2 build finished")
    {
        missing.push("card 'Agent: Reply ready…' with the subtype and text name".to_string());
    }
    let open = nodes
        .iter()
        .filter(|n| n.role == "push-button" && n.name == "Open chat")
        .count();
    let ok = nodes
        .iter()
        .filter(|n| n.role == "push-button" && n.name == "OK")
        .count();
    if open == 0 || ok == 0 {
        missing.push(format!("card buttons 'Open chat' ({open}) and 'OK' ({ok})"));
    }
    for n in nodes.iter().filter(|n| n.name.contains("unread")) {
        if !n.states.contains("showing") && !n.states.contains("visible") {
            missing.push(format!("'{}' is not visible: {}", n.name, n.states));
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
