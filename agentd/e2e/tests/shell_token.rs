//! A7-02: a `kind = agent` token for the `shell` command via a descriptor. The command (a real `agentd runner`) reads the token from
//! `AGENTD_TOKEN_FD` and calls the real core over the socket with it: own messages are accessible, another session and settings are closed
//! (`allowlist`), the value is absent from the environment and the journal, and the token is revoked after the command, including when the runner refuses.

mod common;

use std::path::Path;
use std::time::Duration;

use common::{Env, finished, is_kind};
use core_client::api::Frame;
use serde_json::json;

const PROBE: &str = r#"
{ printf 'Authorization: Bearer '; eval "cat <&$AGENTD_TOKEN_FD"; } > hdr.txt
code() { curl -s -o /dev/null -w '%{http_code}' --unix-socket "$SOCK" -H @hdr.txt "http://localhost/v1$1"; }
echo "own=$(code /sessions/$SESSION/messages)"
echo "other=$(code /sessions/$OTHER/messages)"
echo "settings=$(code /settings/appearance)"
echo "env_has_token=$(env | grep -c AGENTD_TOKEN=)"
"#;

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

fn is_end(f: &Frame) -> bool {
    is_kind(f, "terminal.status") && f.data["status"] != "started"
}

fn stdout(frames: &[Frame]) -> String {
    frames
        .iter()
        .filter(|f| is_kind(f, "terminal.output") && f.data["stream"] == "stdout")
        .map(|f| f.data["data"].as_str().unwrap_or_default())
        .collect()
}

/// Number of active agent tokens of session `s` (label `agent:<s>`).
async fn live_agent_tokens(e: &mut Env, s: i64) -> i64 {
    e.inspect(move |c| {
        c.query_row(
            "SELECT count(*) FROM api_token WHERE kind = 'agent' AND label = ?1 AND revoked_at IS NULL",
            [format!("agent:{s}")],
            |r| r.get(0),
        )
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_command_gets_agent_token_through_fd_and_it_is_scoped() {
    let mut e = Env::start().await;
    let project = e.home.join("project");
    let s = shell_session(&e, &project).await;
    let (_, other) = e.model_session().await;
    std::fs::write(project.join("probe.sh"), PROBE).unwrap();
    let _runner = e.start_runner(&[&project]);
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    let cmd = format!(
        "SOCK={} SESSION={s} OTHER={other} sh probe.sh",
        e.sock.display()
    );
    e.say(s, "c1", &format!("#scn:action_shellrun {cmd}")).await;
    let frames = ws.until(Duration::from_secs(30), is_end);
    let out = stdout(&frames);
    assert!(out.contains("own=200\n"), "own messages accessible: {out}");
    assert!(
        out.contains("other=403\n"),
        "another session is closed: {out}"
    );
    assert!(out.contains("settings=403\n"), "settings are closed: {out}");
    assert!(
        out.contains("env_has_token=0\n"),
        "token is not in the environment: {out}"
    );
    let (_, tool) = finished(&e, s).await;
    assert_eq!(tool.unwrap()["meta"]["code"], "ok");

    // The token lives only in a file the command created itself; it is revoked after the command.
    let header = std::fs::read_to_string(project.join("hdr.txt")).unwrap();
    let token = header
        .trim()
        .strip_prefix("Authorization: Bearer ")
        .unwrap();
    assert!(!token.is_empty());
    let (st, body) = e
        .api_with(token)
        .get(&format!("/sessions/{s}/messages"))
        .await;
    assert_eq!(st, 401, "token revoked after the command: {body}");
    let log = std::fs::read_to_string(&e.log).unwrap_or_default();
    assert!(!log.contains(token), "token value leaked into the core log");
    assert_eq!(live_agent_tokens(&mut e, s).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_token_is_revoked_when_runner_is_down() {
    let mut e = Env::start().await;
    let project = e.home.join("project");
    let s = shell_session(&e, &project).await;
    // No runner: the socket is not listening, the command is not executed, and no token remains after the refusal.
    let mut ws = e.ws();
    ws.subscribe(&format!("session.{s}"));
    e.say(s, "c1", "#scn:action_shellrun touch ran.txt").await;
    let frames = ws.until(Duration::from_secs(20), is_end);
    assert!(frames.iter().any(|f| f.data["status"] == "denied"));
    assert_eq!(live_agent_tokens(&mut e, s).await, 0);
    assert!(!project.join("ran.txt").exists());
}
