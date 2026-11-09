//! `ui_bar_accessibility_tree`: names and roles of indicators and menu items in the AT-SPI tree of a real `agent-shell` process
//! (with the token from a file and `fake-agentd` data; the test build enables the `test-token-file` feature, the product build does not).

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use fake_agentd::{Config, FakeAgentd};

use crate::ctx::Ctx;

struct Node {
    role: String,
    name: String,
}

fn dump(ctx: &Ctx, d: &FakeAgentd, token_file: &Path, cfg: &Path) -> Result<Vec<Node>, String> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenarios/a11y_dump.py");
    let mut cmd = Command::new("dbus-run-session");
    cmd.args(["--", "python3", "-I"])
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_agent-shell"))
        .arg("--menu");
    ctx.weston.apply_env(&mut cmd);
    cmd.env("GTK_A11Y", "atspi")
        .env("AGENTD_SOCKET", d.socket_path())
        .env("AGENTD_TOKEN_FILE", token_file)
        .env("XDG_CONFIG_HOME", cfg);
    let out = cmd
        .output()
        .map_err(|e| format!("dbus-run-session did not start: {e}"))?;
    let log: String = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.starts_with("do_action") || l.contains("not found"))
        .collect::<Vec<_>>()
        .join("; ");
    println!("    a11y_dump: {log}");
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
            let mut p = l.splitn(3, '\t');
            let (_depth, role, name) = (p.next()?, p.next()?, p.next()?);
            Some(Node {
                role: role.to_string(),
                name: name.to_string(),
            })
        })
        .collect())
}

pub fn run(ctx: &Ctx) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let token_file = dir.path().join("gui.token");
    let config = Config::temp()
        .bar_sys_period(Duration::from_millis(300))
        .token_file(token_file.clone());
    let d = ctx
        .rt
        .block_on(FakeAgentd::start(config))
        .map_err(|e| e.to_string())?;
    let nodes = dump(ctx, &d, &token_file, dir.path())?;
    let find = |role: &str, name: &str| nodes.iter().any(|n| n.role == role && n.name == name);
    let starts = |role: &str, prefix: &str| {
        nodes
            .iter()
            .any(|n| n.role == role && n.name.starts_with(prefix))
    };
    let mut missing = Vec::new();
    for item in ["About", "User", "Settings", "Logout"] {
        if !find("menu-item", item) {
            missing.push(format!("menu button '{item}'"));
        }
    }
    for name in [
        "Quota Claude · key-1: 38% left",
        "Quota Claude · key-2: 82% left",
        "Quota Ollama · local: no limits, load 22%",
    ] {
        if !find("push-button", name) {
            missing.push(format!("icon '{name}'"));
        }
    }
    for prefix in ["CPU: ", "GPU: ", "RAM: ", "Disk: ", "Temperature: "] {
        if !starts("label", prefix) {
            missing.push(format!("metric '{prefix}…'"));
        }
    }
    if !nodes.iter().any(|n| n.name == "Agent OS menu") {
        missing.push("menu button 'Agent OS menu'".into());
    }
    if !starts("filler", "Weather: ") && !nodes.iter().any(|n| n.name.starts_with("Weather: ")) {
        missing.push("weather 'Weather: …'".into());
    }
    if missing.is_empty() {
        return Ok(());
    }
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
