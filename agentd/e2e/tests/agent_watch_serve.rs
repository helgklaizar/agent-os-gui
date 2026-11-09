//! A-core-100 (A7-03): the `agentd serve` core with the observer as a separate process (`AGENTD_WATCH_SOCKET`): a dangerous command
//! is blocked by the process rules; an unreachable process closes dangerous actions (fail-closed) and sets `/health` to `degraded`.
//! Socket activation is emulated by a test thread: each connection gets a real `agentd watch` with stdin on that connection.

mod common;

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use common::{Env, agent_log, agentd_bin, finished, row};
use serde_json::json;

/// The `agent-watch` socket next to the test home directory; a thread listens and spawns an `agentd watch` process per connection.
fn activation(dir: &Path) -> PathBuf {
    let path = dir.join("agent-watch.sock");
    let l = UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        for conn in l.incoming().map_while(Result::ok) {
            let child = Command::new(agentd_bin())
                .arg("watch")
                .env_clear()
                .stdin(Stdio::from(OwnedFd::from(conn)))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(mut c) = child {
                std::thread::spawn(move || drop(c.wait()));
            }
        }
    });
    path
}

async fn caps(e: &Env) -> Vec<String> {
    let (_, h) = e.read.get("/health").await;
    h["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_process_blocks_dangerous_command_in_serve() {
    let dir = tempfile::tempdir().unwrap();
    let sock = activation(dir.path());
    let mut e = Env::start_with_env(vec![(
        "AGENTD_WATCH_SOCKET".into(),
        sock.to_string_lossy().into_owned(),
    )])
    .await;
    assert!(caps(&e).await.iter().any(|c| c == "observer-process"));
    let (_, h) = e.read.get("/health").await;
    assert_eq!(h["status"], "ok", "the process answers the ping: {h}");
    let (_, s) = e.model_session().await;
    assert_eq!(
        e.admin
            .put("/permissions/global/0/shell", json!({"mode": "allow"}))
            .await
            .0,
        200
    );
    e.say(s, "c1", "#scn:action_danger clean up").await;
    let (agent, tool) = finished(&e, s).await;
    assert!(agent[1].contains("blocked_by_observer"), "{agent:?}");
    assert_eq!(tool.unwrap()["meta"]["code"], "blocked_by_observer");
    assert_eq!(
        agent_log(&mut e, "shell").await,
        vec![row("shell", "deny", "observer_blocked")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_process_down_is_fail_closed_and_degraded() {
    let dir = tempfile::tempdir().unwrap();
    // No socket: the observer process is unreachable from the start.
    let e = Env::start_with_env(vec![(
        "AGENTD_WATCH_SOCKET".into(),
        dir.path()
            .join("missing.sock")
            .to_string_lossy()
            .into_owned(),
    )])
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_, h) = e.read.get("/health").await;
    assert_eq!(h["status"], "degraded", "{h}");
    let (_, s) = e.model_session().await;
    assert_eq!(
        e.admin
            .put("/permissions/global/0/shell", json!({"mode": "allow"}))
            .await
            .0,
        200
    );
    e.say(s, "c1", "#scn:action_shell run").await;
    let (id, card) = e.wait_card(s).await;
    let r = &card["meta"]["request"];
    assert_eq!(r["source"], "observer", "{r}");
    assert_eq!(e.decide(id, json!({"decision": "deny"})).await.0, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_watch_socket_means_in_process_rules() {
    let e = Env::start().await;
    assert!(!caps(&e).await.iter().any(|c| c == "observer-process"));
}
