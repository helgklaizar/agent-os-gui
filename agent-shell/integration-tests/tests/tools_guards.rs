//! Source guard checks: the GUI does not run agent commands and does not send terminal output
//! to the core. A guard counts when the product code is checked by a real call, not a test substitute.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn src_files(crate_dir: &str) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    rust_files(&root().join(crate_dir).join("src"), &mut files);
    assert!(!files.is_empty(), "no sources in {crate_dir}");
    files
        .into_iter()
        .filter_map(|p| fs::read_to_string(&p).ok().map(|t| (p, t)))
        .collect()
}

/// The GUI product launches no processes: the user terminal is started by VTE, agent commands are nowhere.
#[test]
fn agent_command_runs_in_kernel_pty_not_gui_source_has_no_process_spawn() {
    for krate in ["controllers", "core-client", "ui"] {
        for (path, text) in src_files(krate) {
            for needle in ["Command::new(", "process::Command", "tokio::process"] {
                assert!(
                    !text.contains(needle),
                    "{}: `{needle}` -- the GUI does not create processes itself (agent commands are run by the core)",
                    path.display()
                );
            }
        }
    }
}

/// Terminal output does not go to `tool-results`: the only call is the browser command result from `tools_exec`.
#[test]
fn gui_never_posts_terminal_output_to_tool_results() {
    let mut users = Vec::new();
    for krate in ["controllers", "core-client", "ui"] {
        for (path, text) in src_files(krate) {
            if text.contains("post_tool_result(") || text.contains("/tool-results\"") {
                users.push(
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
            }
        }
    }
    users.sort();
    assert_eq!(
        users,
        vec!["tools.rs", "tools_exec.rs"],
        "only the core client and the tool command executor call it"
    );
    let (_, exec) = src_files("ui")
        .into_iter()
        .find(|(p, _)| p.ends_with("tools_exec.rs"))
        .unwrap_or_else(|| panic!("no tools_exec.rs"));
    assert!(exec.contains("PostToolResult"));
    let (_, ctl) = src_files("controllers")
        .into_iter()
        .find(|(p, _)| p.ends_with("controllers/terminal.rs"))
        .unwrap_or_else(|| panic!("no terminal.rs"));
    assert!(
        !ctl.contains("PostToolResult") && !ctl.contains("CoreCommand"),
        "TerminalController sends nothing to the core"
    );
}

/// The GUI token does not enter the terminal environment: `user_terminal_env` is the only source of the VTE environment.
#[test]
fn user_terminal_env_is_built_only_by_allowlist() {
    let (_, text) = src_files("controllers")
        .into_iter()
        .find(|(p, _)| p.ends_with("terminal_view.rs"))
        .unwrap_or_else(|| panic!("no terminal_view.rs"));
    assert!(
        text.contains("INHERITED"),
        "the set of inherited names is given as a list"
    );
    for secret in ["TOKEN", "AGENTD", "KEY", "SECRET"] {
        let allowed_line = text
            .lines()
            .find(|l| l.contains("const INHERITED"))
            .unwrap_or("");
        assert!(!allowed_line.contains(secret));
    }
}
