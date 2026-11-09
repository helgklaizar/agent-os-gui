//! S6-07, S6-08: user terminal (session folder, one per session, environment without tokens) and the agent output panel.

mod tree_common;

use std::path::{Path, PathBuf};
use std::time::Instant;

use controllers::controllers::terminal::TerminalController;
use controllers::terminal_view::{
    AGENT_OUTPUT_LIMIT, AGENT_RUNS_KEPT, TermMsg, term_cwd, user_shell, user_terminal_env,
};
use controllers::tool_types::{TermFrame, parse_term_frame};
use controllers::tree_types::TreeFrame;
use controllers::{Controller, Effect};
use serde_json::json;
use tree_common::session;

fn home() -> PathBuf {
    PathBuf::from("/home/user")
}

fn ctl() -> TerminalController {
    TerminalController::new(home())
}

fn send(c: &mut TerminalController, m: TermMsg) -> Vec<Effect> {
    c.update(m, Instant::now())
}

fn with_path(id: i64, path: Option<&str>) -> controllers::tree_types::SessionItem {
    let mut s = session(id, None, "s");
    s.path = path.map(str::to_string);
    s
}

#[test]
fn terminal_cwd_is_session_path_or_home() {
    let h = home();
    assert_eq!(
        term_cwd(Some("/work/proj"), &h),
        PathBuf::from("/work/proj")
    );
    assert_eq!(term_cwd(None, &h), h);
    assert_eq!(term_cwd(Some(""), &h), h);
    assert_eq!(term_cwd(Some("   "), &h), h);
    assert_eq!(
        term_cwd(Some("relative"), &h),
        h,
        "a relative path is not trusted"
    );
    assert_eq!(
        term_cwd(Some("/a/../etc"), &h),
        h,
        "directory traversal is not accepted"
    );

    let mut c = ctl();
    send(
        &mut c,
        TermMsg::Snapshot(vec![with_path(1, Some("/work/proj")), with_path(2, None)]),
    );
    send(&mut c, TermMsg::Session(Some(1)));
    assert_eq!(
        c.view().user.map(|u| u.cwd),
        Some(PathBuf::from("/work/proj"))
    );
    send(&mut c, TermMsg::Session(Some(2)));
    assert_eq!(c.view().user.map(|u| u.cwd), Some(home()));
}

#[test]
fn terminal_path_change_applies_at_next_launch() {
    let mut c = ctl();
    send(
        &mut c,
        TermMsg::Snapshot(vec![with_path(1, Some("/work/a"))]),
    );
    send(&mut c, TermMsg::Session(Some(1)));
    assert!(c.view().user.as_ref().is_some_and(|u| u.needs_spawn));
    send(&mut c, TermMsg::Spawned(1));
    let u = c.view().user.unwrap_or_else(|| panic!("no terminal"));
    assert!(u.running && !u.needs_spawn);
    send(
        &mut c,
        TermMsg::Tree(TreeFrame::SessionUpdated(with_path(1, Some("/work/b")))),
    );
    assert_eq!(
        c.view().user.map(|u| u.cwd),
        Some(PathBuf::from("/work/a")),
        "a running terminal stays in its folder"
    );
    send(&mut c, TermMsg::Exited(1));
    let u = c.view().user.unwrap_or_else(|| panic!("no terminal"));
    assert!(u.needs_spawn && !u.running);
    assert_eq!(
        u.cwd,
        PathBuf::from("/work/b"),
        "the next launch is in the new folder"
    );
    // until the terminal is started, the folder follows the session
    send(
        &mut c,
        TermMsg::Tree(TreeFrame::SessionUpdated(with_path(1, None))),
    );
    assert_eq!(c.view().user.map(|u| u.cwd), Some(home()));
}

#[test]
fn terminal_one_pty_per_session_kept_alive() {
    let mut c = ctl();
    send(
        &mut c,
        TermMsg::Snapshot(vec![with_path(1, None), with_path(2, None)]),
    );
    send(&mut c, TermMsg::Session(Some(1)));
    send(&mut c, TermMsg::Spawned(1));
    send(&mut c, TermMsg::Session(Some(2)));
    assert!(
        c.view()
            .user
            .as_ref()
            .is_some_and(|u| u.session_id == 2 && u.needs_spawn),
        "the second session has its own terminal, not started yet"
    );
    send(&mut c, TermMsg::Spawned(2));
    assert_eq!(c.view().live, vec![1, 2], "both processes are alive");
    send(&mut c, TermMsg::Session(Some(1)));
    let u = c.view().user.unwrap_or_else(|| panic!("no terminal"));
    assert_eq!(u.session_id, 1);
    assert!(
        u.running && !u.needs_spawn,
        "returning does not start a second process"
    );
    send(&mut c, TermMsg::Session(None));
    assert_eq!(
        c.view().live,
        vec![1, 2],
        "closing the tab does not kill processes"
    );
    send(&mut c, TermMsg::Tree(TreeFrame::SessionDeleted(2)));
    assert_eq!(
        c.view().live,
        vec![1],
        "a deleted session takes its terminal with it"
    );
}

fn cmd(c: &mut TerminalController, s: i64, id: &str, text: &str) -> Vec<Effect> {
    send(
        c,
        TermMsg::Agent {
            session_id: s,
            frame: TermFrame::Command {
                request_id: id.into(),
                command: text.into(),
                cwd: "~".into(),
            },
        },
    )
}

fn out(c: &mut TerminalController, s: i64, id: &str, seq: u64, data: &str) {
    send(
        c,
        TermMsg::Agent {
            session_id: s,
            frame: TermFrame::Output {
                request_id: id.into(),
                seq,
                stderr: false,
                data: data.into(),
            },
        },
    );
}

fn status(c: &mut TerminalController, s: i64, id: &str, st: &str, code: Option<i64>) {
    send(
        c,
        TermMsg::Agent {
            session_id: s,
            frame: TermFrame::Status {
                request_id: id.into(),
                status: st.into(),
                exit_code: code,
                duration_ms: Some(12),
            },
        },
    );
}

#[test]
fn user_terminal_does_not_receive_agent_commands() {
    let mut c = ctl();
    send(&mut c, TermMsg::Session(Some(1)));
    send(&mut c, TermMsg::Spawned(1));
    let before = c.view().user;
    let fx = cmd(&mut c, 1, "r1", "rm -rf /tmp/x");
    out(&mut c, 1, "r1", 0, "done\n");
    status(&mut c, 1, "r1", "exited", Some(0));
    assert!(
        fx.is_empty(),
        "agent frames spawn neither core commands nor processes: {fx:?}"
    );
    assert_eq!(c.view().user, before, "the user's VTE is untouched");
    let run = &c.view().agent[0];
    assert_eq!(
        run.command, "rm -rf /tmp/x",
        "the command is visible in the agent panel"
    );
    assert_eq!(run.output, "done\n");
    assert_eq!((run.status.as_str(), run.exit_code), ("exited", Some(0)));
}

#[test]
fn agent_output_panel_follows_frames_and_ignores_repeats() {
    let mut c = ctl();
    send(&mut c, TermMsg::Session(Some(1)));
    cmd(&mut c, 1, "r1", "ls");
    assert_eq!(c.view().agent[0].status, "queued");
    status(&mut c, 1, "r1", "started", None);
    out(&mut c, 1, "r1", 0, "a\n");
    out(&mut c, 1, "r1", 1, "b\n");
    out(&mut c, 1, "r1", 1, "b\n");
    out(&mut c, 1, "r1", 0, "a\n");
    assert_eq!(
        c.view().agent[0].output,
        "a\nb\n",
        "redelivery of a chunk does not duplicate output"
    );
    cmd(&mut c, 1, "r1", "ls");
    assert_eq!(
        c.view().agent.len(),
        1,
        "a repeated command frame is not a second entry"
    );
    status(&mut c, 1, "r1", "killed", None);
    assert_eq!(c.view().agent[0].status, "killed");
    // output without command (the subscription caught up mid-stream): the entry is created
    out(&mut c, 1, "r9", 0, "tail");
    assert_eq!(c.view().agent.len(), 2);
    // the panel shows only the active session
    cmd(&mut c, 2, "r2", "pwd");
    assert_eq!(c.view().agent.len(), 2);
    send(&mut c, TermMsg::Session(Some(2)));
    assert_eq!(c.view().agent.len(), 1);
    assert_eq!(c.view().agent[0].command, "pwd");
}

#[test]
fn agent_denied_status_is_shown() {
    let mut c = ctl();
    send(&mut c, TermMsg::Session(Some(1)));
    cmd(&mut c, 1, "r1", "git push --force");
    status(&mut c, 1, "r1", "denied", None);
    let run = &c.view().agent[0];
    assert_eq!(run.status, "denied");
    assert_eq!(run.output, "");
}

#[test]
fn agent_output_is_capped_and_runs_are_bounded() {
    let mut c = ctl();
    send(&mut c, TermMsg::Session(Some(1)));
    cmd(&mut c, 1, "big", "yes");
    let chunk = "é".repeat(60_000);
    for seq in 0..10 {
        out(&mut c, 1, "big", seq, &chunk);
    }
    let run = &c.view().agent[0];
    assert!(run.truncated);
    assert!(
        run.output.len() <= AGENT_OUTPUT_LIMIT,
        "{} bytes",
        run.output.len()
    );
    assert!(
        run.output.len() > AGENT_OUTPUT_LIMIT - 4,
        "filled up to the limit: {}",
        run.output.len()
    );
    assert_eq!(
        run.output.len(),
        262_144,
        "api.md 4.4: accumulated agent output is 256 KiB"
    );
    for n in 0..(AGENT_RUNS_KEPT + 5) {
        cmd(&mut c, 1, &format!("n{n}"), "x");
    }
    let runs = c.view().agent;
    assert_eq!(runs.len(), AGENT_RUNS_KEPT, "old entries are dropped");
    assert_eq!(
        runs.last().map(|r| r.request_id.clone()),
        Some(format!("n{}", AGENT_RUNS_KEPT + 4))
    );
}

#[test]
fn agent_frames_parse_from_wire_and_reject_garbage() {
    let ok = parse_term_frame(
        &json!({"kind": "terminal.output", "request_id": "r", "seq": 3, "stream": "stderr", "data": "e"}),
    );
    assert_eq!(
        ok,
        Some(TermFrame::Output {
            request_id: "r".into(),
            seq: 3,
            stderr: true,
            data: "e".into()
        })
    );
    assert!(
        parse_term_frame(
            &json!({"kind": "terminal.output", "request_id": "r", "stream": "stdout", "data": "x"})
        )
        .is_none(),
        "no seq"
    );
    assert!(
        parse_term_frame(&json!({"kind": "terminal.status", "status": "exited"})).is_none(),
        "no request_id"
    );
    assert!(parse_term_frame(&json!({"kind": "message.delta", "request_id": "r"})).is_none());
    assert!(
        parse_term_frame(
            &json!({"kind": "terminal.command", "request_id": "x".repeat(300), "command": "ls"})
        )
        .is_none()
    );
}

fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn user_terminal_env_has_no_admin_token() {
    let parent = env(&[
        ("HOME", "/home/user"),
        ("PATH", "/usr/bin"),
        ("LANG", "ru_RU.UTF-8"),
        ("SHELL", "/bin/bash"),
        ("AGENTD_TOKEN", "<token>"),
        ("AGENTD_TOKEN_FILE", "/run/x"),
        ("AGENTD_ADMIN_TOKEN", "<token>"),
        ("AGENTD_TOKEN_FD", "3"),
        ("AGENT_TOKEN", "<token>"),
        ("GITHUB_TOKEN", "<token>"),
        ("OPENAI_API_KEY", "<key>"),
        ("SSH_AUTH_SOCK", "/tmp/ssh"),
        ("WAYLAND_DISPLAY", "wayland-0"),
        ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ("LD_PRELOAD", "/tmp/x.so"),
        ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
    ]);
    let got = user_terminal_env(&parent, Path::new("/work/p"));
    let names: Vec<&str> = got.iter().map(|(k, _)| k.as_str()).collect();
    for secret in [
        "AGENTD_TOKEN",
        "AGENTD_TOKEN_FILE",
        "AGENTD_ADMIN_TOKEN",
        "AGENTD_TOKEN_FD",
        "AGENT_TOKEN",
        "GITHUB_TOKEN",
        "OPENAI_API_KEY",
        "SSH_AUTH_SOCK",
        "LD_PRELOAD",
        "DBUS_SESSION_BUS_ADDRESS",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
    ] {
        assert!(
            !names.contains(&secret),
            "{secret} leaked into the user terminal: {names:?}"
        );
    }
    assert!(
        !got.iter()
            .any(|(_, v)| v.contains("<token>") || v.contains("<key>")),
        "{got:?}"
    );
    let get = |k: &str| got.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    assert_eq!(get("HOME"), Some("/home/user"));
    assert_eq!(get("PATH"), Some("/usr/bin"));
    assert_eq!(get("LANG"), Some("ru_RU.UTF-8"));
    assert_eq!(get("TERM"), Some("xterm-256color"));
    assert_eq!(get("PWD"), Some("/work/p"));
    assert!(
        get("PS1").is_some_and(|p| p.contains("\\w")),
        "the prompt shows the path"
    );
    assert_eq!(got.iter().filter(|(k, _)| k == "TERM").count(), 1);
}

#[test]
fn user_terminal_env_without_home_and_path_is_safe() {
    let got = user_terminal_env(&env(&[("AGENTD_TOKEN", "<token>")]), Path::new("/w"));
    let names: Vec<&str> = got.iter().map(|(k, _)| k.as_str()).collect();
    assert!(!names.contains(&"AGENTD_TOKEN"));
    assert!(names.contains(&"PATH"), "PATH is always present: {names:?}");
}

#[test]
fn user_shell_accepts_only_absolute_paths() {
    assert_eq!(user_shell(Some("/bin/zsh")), "/bin/zsh");
    assert_eq!(user_shell(Some("zsh")), "/bin/sh");
    assert_eq!(user_shell(Some("")), "/bin/sh");
    assert_eq!(user_shell(None), "/bin/sh");
    assert_eq!(
        user_shell(Some("/bin/zsh -c evil")),
        "/bin/sh",
        "arguments in the shell name are not allowed"
    );
}
