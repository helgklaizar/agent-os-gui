//! A7-02: the `shell` executor in the core. The agent command goes through the agent runner (a socket connection; in the test, a real
//! `agentd runner`); the `terminal.command`, `terminal.output`, `terminal.status` frames reach `session.<id>` subscribers.
//! Runner stopped: the command is not executed (fail-closed), status `denied`.

mod common;

use std::path::Path;
use std::time::Duration;

use common::{Env, agent_log, finished, is_kind, row};
use core_client::api::Frame;
use serde_json::{Value, json};

/// Session with a scripted model, a project directory and `shell` allowed.
async fn shell_session(e: &Env, project: &Path) -> i64 {
    std::fs::create_dir_all(project).unwrap();
    let (_, s) = e.model_session().await;
    let (st, b) = e
        .chat
        .patch(&format!("/sessions/{s}"), json!({"path": project}))
        .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(
        e.admin
            .put("/permissions/global/0/shell", json!({"mode": "allow"}))
            .await
            .0,
        200
    );
    s
}

fn terminal(frames: &[Frame]) -> Vec<&Value> {
    frames
        .iter()
        .filter(|f| {
            f.data["kind"]
                .as_str()
                .is_some_and(|k| k.starts_with("terminal."))
        })
        .map(|f| &f.data)
        .collect()
}

fn is_end(f: &Frame) -> bool {
    is_kind(f, "terminal.status") && f.data["status"] != "started"
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_runs_through_runner_and_streams_frames() {
    let e = Env::start().await;
    let project = e.home.join("project");
    let s = shell_session(&e, &project).await;
    let _runner = e.start_runner(&[&project]);
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(
        s,
        "c1",
        "#scn:action_shellrun echo hello; echo err >&2; touch made.txt",
    )
    .await;
    let frames = ws.until(Duration::from_secs(20), is_end);
    let t = terminal(&frames);
    let kinds: Vec<&str> = t.iter().filter_map(|d| d["kind"].as_str()).collect();
    assert_eq!(kinds.first(), Some(&"terminal.command"), "{kinds:?}");
    assert_eq!(t[0]["command"], "echo hello; echo err >&2; touch made.txt");
    assert_eq!(t[0]["cwd"], project.to_string_lossy().as_ref());
    let rid = t[0]["request_id"].as_str().unwrap().to_string();
    assert!(!rid.is_empty());
    assert!(t.iter().all(|d| d["request_id"] == rid.as_str()), "{t:?}");
    assert_eq!(t[1]["kind"], "terminal.status");
    assert_eq!(t[1]["status"], "started");
    let out: Vec<&&Value> = t
        .iter()
        .filter(|d| d["kind"] == "terminal.output")
        .collect();
    let seqs: Vec<u64> = out.iter().map(|d| d["seq"].as_u64().unwrap()).collect();
    assert!(
        !seqs.is_empty() && seqs.windows(2).all(|w| w[0] < w[1]),
        "{seqs:?}"
    );
    let stdout: String = out
        .iter()
        .filter(|d| d["stream"] == "stdout")
        .map(|d| d["data"].as_str().unwrap())
        .collect();
    assert_eq!(stdout, "hello\n");
    assert!(
        out.iter()
            .any(|d| d["stream"] == "stderr" && d["data"] == "err\n"),
        "{out:?}"
    );
    let end = t.last().unwrap();
    assert_eq!(
        (end["status"].as_str(), end["exit_code"].as_i64()),
        (Some("exited"), Some(0))
    );
    assert!(end["duration_ms"].is_u64(), "{end}");
    assert!(
        project.join("made.txt").exists(),
        "command ran in the project"
    );
    // The result goes to the model as untrusted tool output (A4-09).
    let (agent, tool) = finished(&e, s).await;
    assert!(
        agent[1].contains("hello") && agent[1].contains("\"untrusted\":true"),
        "{agent:?}"
    );
    assert_eq!(tool.unwrap()["meta"]["code"], "ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_frames_order_and_truncate() {
    let e = Env::start().await;
    let project = e.home.join("project");
    let s = shell_session(&e, &project).await;
    let _runner = e.start_runner(&[&project]);
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(
        s,
        "c1",
        "#scn:action_shellrun head -c 400000 /dev/zero | tr '\\0' x",
    )
    .await;
    let frames = ws.until(Duration::from_secs(30), is_end);
    let t = terminal(&frames);
    let total: usize = t
        .iter()
        .filter(|d| d["kind"] == "terminal.output")
        .map(|d| d["data"].as_str().unwrap().len())
        .sum();
    assert!(total <= 256 * 1024, "256 KiB limit: {total}");
    assert!(total > 200 * 1024, "output reached the limit: {total}");
    let seqs: Vec<u64> = t
        .iter()
        .filter(|d| d["kind"] == "terminal.output")
        .map(|d| d["seq"].as_u64().unwrap())
        .collect();
    assert!(
        seqs.windows(2).all(|w| w[0] + 1 == w[1]),
        "seq is consecutive"
    );
    assert_eq!(t.last().unwrap()["truncated"], true, "{:?}", t.last());
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_stop_kills_process() {
    let e = Env::start().await;
    let project = e.home.join("project");
    let s = shell_session(&e, &project).await;
    let _runner = e.start_runner(&[&project]);
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(
        s,
        "c1",
        "#scn:action_shellrun echo $$ > pid.txt; sleep 30; touch after.txt",
    )
    .await;
    ws.until(Duration::from_secs(20), |f| {
        is_kind(f, "terminal.status") && f.data["status"] == "started"
    });
    for _ in 0..100 {
        if project.join("pid.txt").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let pid = std::fs::read_to_string(project.join("pid.txt"))
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(
        e.chat
            .post(&format!("/sessions/{s}/stop"), json!({}))
            .await
            .0,
        204
    );
    let frames = ws.until(Duration::from_secs(10), is_end);
    assert_eq!(terminal(&frames).last().unwrap()["status"], "killed");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let alive = std::process::Command::new("kill")
        .args(["-0", &pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!alive, "process {pid} killed");
    assert!(!project.join("after.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_timeout() {
    let e = Env::start().await;
    let project = e.home.join("project");
    let s = shell_session(&e, &project).await;
    let _runner = e.start_runner(&[&project]);
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(s, "c1", "#scn:action_shellrun:1 sleep 20").await;
    let frames = ws.until(Duration::from_secs(20), is_end);
    assert_eq!(terminal(&frames).last().unwrap()["status"], "timeout");
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "timeout");
}

#[tokio::test(flavor = "multi_thread")]
async fn runner_stopped_fail_closed() {
    let mut e = Env::start().await;
    let project = e.home.join("project");
    let s = shell_session(&e, &project).await;
    // No runner: the socket is not listening.
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(s, "c1", "#scn:action_shellrun touch ran.txt").await;
    let frames = ws.until(Duration::from_secs(20), is_end);
    let t = terminal(&frames);
    assert_eq!(t.last().unwrap()["status"], "denied", "{t:?}");
    assert!(t.iter().all(|d| d["status"] != "started"));
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "executor_unavailable");
    assert!(!project.join("ran.txt").exists());
    assert_eq!(
        agent_log(&mut e, "shell").await,
        vec![row("shell", "allow", "executor_unavailable")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_dangerous_blocked_before_run() {
    let e = Env::start().await;
    let project = e.home.join("project");
    let s = shell_session(&e, &project).await;
    let _runner = e.start_runner(&[&project]);
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(s, "c1", "#scn:action_shellrun rm -rf / && touch ran.txt")
        .await;
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "blocked_by_observer");
    let frames = ws.drain(Duration::from_millis(500));
    assert!(
        terminal(&frames).is_empty(),
        "process was not started: {frames:?}"
    );
    assert!(!project.join("ran.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_without_session_path_is_denied() {
    let e = Env::start().await;
    let project = e.home.join("project");
    let s = shell_session(&e, &project).await;
    let (st, b) = e
        .chat
        .patch(&format!("/sessions/{s}"), json!({"path": null}))
        .await;
    assert_eq!(st, 200, "{b}");
    let _runner = e.start_runner(&[&project]);
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(s, "c1", "#scn:action_shellrun touch ran.txt").await;
    let frames = ws.until(Duration::from_secs(20), is_end);
    assert_eq!(terminal(&frames).last().unwrap()["status"], "denied");
    assert!(!project.join("ran.txt").exists());
}
