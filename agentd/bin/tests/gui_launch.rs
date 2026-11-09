//! A1-06: GUI token via descriptor and `agentd gui-launch` (SP-1.5). Linux only: needs `/proc` and `SCM_RIGHTS`.
//! The "own cage" in tests is a copy of `dash` named `cage` (the process comm is taken from the executable file name).
#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const AGENTD: &str = env!("CARGO_BIN_EXE_agentd");

struct Rig {
    dir: tempfile::TempDir,
    daemon: Option<Child>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(mut d) = self.daemon.take() {
            drop(d.kill());
            drop(d.wait());
        }
    }
}

impl Rig {
    fn p(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}

/// A copy of `dash` under a different name: this gives the process the needed comm.
fn named_shell(dir: &Path, name: &str) -> PathBuf {
    let dest = dir.join(name);
    let src = std::fs::canonicalize("/bin/sh").expect("/bin/sh");
    std::fs::copy(src, &dest).unwrap();
    dest
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    for d in ["run", "out"] {
        std::fs::create_dir(dir.path().join(d)).unwrap();
    }
    named_shell(dir.path(), "cage");
    named_shell(dir.path(), "agent-shell");
    let mut r = Rig { dir, daemon: None };
    let child = Command::new(AGENTD)
        .args(["serve", "--db"])
        .arg(r.p("agentos/sessions.db"))
        .arg("--socket")
        .arg(r.p("run/agentd.sock"))
        .arg("--launch-socket")
        .arg(r.p("run/launch.sock"))
        .env_remove("XDG_RUNTIME_DIR")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    r.daemon = Some(child);
    wait_until(
        || UnixStream::connect(r.p("run/launch.sock")).is_ok(),
        "launch-socket",
    );
    r
}

fn wait_until(mut f: impl FnMut() -> bool, what: &str) {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for: {what}");
}

/// GUI probe: reads the token from fd, saves its process state and lives on.
const PROBE: &str = r#"
set -e
tok=$(cat /proc/self/fd/$AGENTD_TOKEN_FD)
printf '%s' "$tok" > "$OUT/token"
tr '\0' '\n' < /proc/$$/environ > "$OUT/environ"
tr '\0' ' ' < /proc/$$/cmdline > "$OUT/cmdline"
ls -l /proc/$$/fd > "$OUT/fds"
: > "$OUT/ready"
exec sleep 60
"#;

struct Cage {
    child: Child,
    out: PathBuf,
}

impl Drop for Cage {
    fn drop(&mut self) {
        // The whole group: cage, gui-launch (which is also the GUI) and sleep.
        drop(
            Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .status(),
        );
        drop(self.child.wait());
    }
}

/// Starts "cage", and inside it `agentd gui-launch -- agent-shell -c PROBE`.
fn start_cage(r: &Rig, tag: &str) -> Cage {
    let out = r.p(&format!("out-{tag}"));
    std::fs::create_dir(&out).unwrap();
    let probe = r.p(&format!("probe-{tag}.sh"));
    std::fs::write(&probe, PROBE).unwrap();
    let script = format!(
        "{} gui-launch --launch-socket {} -- {} {}",
        AGENTD,
        r.p("run/launch.sock").display(),
        r.p("agent-shell").display(),
        probe.display()
    );
    let child = Command::new(r.p("cage"))
        .arg("-c")
        .arg(script)
        .env("OUT", &out)
        .process_group(0)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    Cage { child, out }
}

fn http(sock: &Path, method: &str, path: &str, token: &str) -> (u16, String) {
    let mut s = UnixStream::connect(sock).unwrap();
    write!(s, "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nAuthorization: Bearer {token}\r\n\r\n").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, out)
}

fn gui_count(r: &Rig) -> (i64, i64) {
    let c = agentd_db::Connection::open(r.p("agentos/sessions.db")).unwrap();
    c.query_row(
        "SELECT count(*), count(revoked_at) FROM api_token WHERE kind = 'gui'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

fn token_of(c: &Cage) -> String {
    wait_until(
        || c.out.join("ready").exists(),
        "GUI probe ran to completion",
    );
    std::fs::read_to_string(c.out.join("token")).unwrap()
}

#[test]
fn gui_token_via_fd_not_in_environ() {
    let r = rig();
    let cage = start_cage(&r, "a");
    let token = token_of(&cage);
    assert!(
        token.starts_with("agt_") && token.len() == 47,
        "value obtained via fd"
    );
    for f in ["environ", "cmdline", "fds"] {
        let text = std::fs::read_to_string(cage.out.join(f)).unwrap();
        assert!(!text.contains(&token), "token found in {f}");
    }
    let fds = std::fs::read_to_string(cage.out.join("fds")).unwrap();
    // Among the GUI's open descriptors there is no regular file with the token (the token descriptor is a memfd, not on disk).
    assert!(!fds.contains("agt_"), "{fds}");
    let env = std::fs::read_to_string(cage.out.join("environ")).unwrap();
    assert!(
        env.contains("AGENTD_TOKEN_FD="),
        "descriptor number passed, not the value"
    );
    // The token is in no file of the runtime directory or the DB directory (except the probe's own copy).
    for entry in walk(r.dir.path()) {
        if entry.starts_with(&cage.out) || entry.ends_with("cage") || entry.ends_with("agent-shell")
        {
            continue;
        }
        let bytes = std::fs::read(&entry).unwrap_or_default();
        assert!(
            !bytes.windows(token.len()).any(|w| w == token.as_bytes()),
            "token in file {}",
            entry.display()
        );
    }
    // The token works and it is admin: the token list is available.
    let (st, body) = http(&r.p("run/agentd.sock"), "GET", "/v1/tokens", &token);
    assert_eq!(st, 200, "{body}");
    assert!(body.contains("\"kind\":\"gui\""));
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.is_file() {
            out.push(p);
        }
    }
    out
}

fn launch_from(r: &Rig, parent: &str) -> std::process::Output {
    // A direct descendant of `parent` (cage or agent-shell) asks for a token.
    Command::new(r.p(parent))
        .arg("-c")
        .arg(format!(
            "{} gui-launch --launch-socket {} -- /bin/true",
            AGENTD,
            r.p("run/launch.sock").display()
        ))
        .output()
        .unwrap()
}

#[test]
fn gui_token_reissue_rejected() {
    let r = rig();
    let cage = start_cage(&r, "a");
    let _ = token_of(&cage);
    assert_eq!(gui_count(&r), (1, 0));
    let second = launch_from(&r, "cage");
    assert_eq!(
        second.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(String::from_utf8_lossy(&second.stderr).contains("denied: issued"));
    assert_eq!(
        gui_count(&r),
        (1, 0),
        "second token not issued, first untouched"
    );
}

#[test]
fn gui_token_issue_is_journaled() {
    // A4-13 item 5: GUI token issuance via `gui-launch` is written to the action journal (without the token value).
    let r = rig();
    let cage = start_cage(&r, "a");
    let token = token_of(&cage);
    let c = agentd_db::Connection::open(r.p("agentos/sessions.db")).unwrap();
    let rows: Vec<(String, String, String, String)> = c
        .prepare("SELECT action, target, actor, result FROM actions_log WHERE action = 'token_issue' ORDER BY id")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        [(
            "token_issue".into(),
            "gui".into(),
            "user".into(),
            "ok".into()
        )]
    );
    let all: String = c
        .query_row(
            "SELECT group_concat(coalesce(args_json,'') || coalesce(target,'') || coalesce(result,'')) FROM actions_log",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!all.contains(&token), "token value in the journal");
    // A denial (second request while the GUI is alive) does not appear in the issuance journal.
    assert_eq!(launch_from(&r, "cage").status.code(), Some(3));
    let n: i64 = c
        .query_row(
            "SELECT count(*) FROM actions_log WHERE action = 'token_issue'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn non_ancestor_cannot_request_gui_token() {
    let r = rig();
    // Directly from the test: the ancestor is not cage.
    let direct = Command::new(AGENTD)
        .args(["gui-launch", "--launch-socket"])
        .arg(r.p("run/launch.sock"))
        .args(["--", "/bin/true"])
        .output()
        .unwrap();
    assert_eq!(
        direct.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&direct.stderr)
    );
    assert!(String::from_utf8_lossy(&direct.stderr).contains("denied: parent"));
    assert_eq!(gui_count(&r), (0, 0), "token not issued");
    // A descendant of the GUI itself (a terminal in VTE) is also refused: agent-shell stands between it and cage.
    let cage = start_cage(&r, "a");
    let _ = token_of(&cage);
    let child = launch_from(&r, "agent-shell");
    assert_eq!(child.status.code(), Some(3));
    assert!(
        String::from_utf8_lossy(&child.stderr).contains("denied: parent"),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(gui_count(&r), (1, 0));
}

#[test]
fn launch_without_daemon_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(AGENTD)
        .args(["gui-launch", "--launch-socket"])
        .arg(dir.path().join("none.sock"))
        .args(["--", "/bin/true"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&out.stderr).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn gui_restart_revokes_old_token() {
    let r = rig();
    let first = start_cage(&r, "a");
    let old = token_of(&first);
    let sock = r.p("run/agentd.sock");
    // The WS of the old token is open.
    let stream = tokio::net::UnixStream::connect(&sock).await.unwrap();
    let mut req = "ws://localhost/v1/ws".into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {old}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::client_async(req, stream).await.unwrap();
    // The GUI crashed: the whole group is killed. Then kiosk starts it again.
    drop(first);
    let second = start_cage(&r, "b");
    let new = token_of(&second);
    assert_ne!(old, new);
    let code = loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("close arrived")
        {
            Some(Ok(Message::Close(Some(f)))) => break u16::from(f.code),
            Some(Ok(_)) => {}
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(code, 4401, "old WS closed when a new token is issued");
    assert_eq!(http(&sock, "GET", "/v1/tokens", &old).0, 401);
    assert_eq!(http(&sock, "GET", "/v1/tokens", &new).0, 200);
    assert_eq!(gui_count(&r), (2, 1), "old revoked, new alive");
}
