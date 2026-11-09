//! A-core-100 (A1-06, D31, D45.5): `kind = agent` token via descriptor and `agentd agent-launch`. Linux only.
//! The "unit" in tests is a copy of `dash` named `unit`: the core hands out an agent token only to a descendant of such a process
//! (in production this is `systemd --user` that started the agent unit).
#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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

fn named_shell(dir: &Path, name: &str) {
    let src = std::fs::canonicalize("/bin/sh").expect("/bin/sh");
    std::fs::copy(src, dir.join(name)).unwrap();
}

fn wait_until(mut f: impl FnMut() -> bool, what: &str) {
    let end = Instant::now() + Duration::from_secs(60);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for: {what}");
}

/// Last cgroup segment of the test itself: "unit" descendants inherit it, so it plays the role of the agent unit.
fn own_unit() -> String {
    let cg = std::fs::read_to_string("/proc/self/cgroup").unwrap();
    let path = cg.lines().next().unwrap().splitn(3, ':').nth(2).unwrap();
    path.rsplit('/').next().unwrap().to_string()
}

/// Sessions `1..=9` in the DB (an agent token is issued only for an existing session).
fn seed_sessions(r: &Rig) {
    let c = agentd_db::Connection::open(r.p("agentos/sessions.db")).unwrap();
    for i in 1..=9 {
        c.execute(
            "INSERT INTO sessions (id, name, created_at, updated_at) VALUES (?1, 'test', 1, 1)",
            [i],
        )
        .unwrap();
    }
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("run")).unwrap();
    named_shell(dir.path(), "unit");
    let mut r = Rig { dir, daemon: None };
    let child = Command::new(AGENTD)
        .args(["serve", "--db"])
        .arg(r.p("agentos/sessions.db"))
        .arg("--socket")
        .arg(r.p("run/agentd.sock"))
        .arg("--launch-socket")
        .arg(r.p("run/launch.sock"))
        .args(["--agent-ancestor", "unit"])
        .args(["--agent-unit", &own_unit()])
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
    seed_sessions(&r);
    r
}

/// Agent probe: reads the token from the descriptor and saves its process state.
const PROBE: &str = r#"
set -e
tok=$(cat /proc/self/fd/$AGENTD_TOKEN_FD)
printf '%s' "$tok" > "$OUT/token"
tr '\0' '\n' < /proc/$$/environ > "$OUT/environ"
tr '\0' ' ' < /proc/$$/cmdline > "$OUT/cmdline"
: > "$OUT/ready"
"#;

/// The "unit" runs `agentd agent-launch --session N -- sh PROBE`; `tag` is the result directory.
fn launch(r: &Rig, tag: &str, session: &str) -> (std::process::Output, PathBuf) {
    let out = r.p(&format!("out-{tag}"));
    std::fs::create_dir(&out).unwrap();
    let probe = r.p(&format!("probe-{tag}.sh"));
    std::fs::write(&probe, PROBE).unwrap();
    let script = format!(
        "{} agent-launch --launch-socket {} --session {} -- /bin/sh {}",
        AGENTD,
        r.p("run/launch.sock").display(),
        session,
        probe.display()
    );
    let o = Command::new(r.p("unit"))
        .arg("-c")
        .arg(script)
        .env("OUT", &out)
        .output()
        .unwrap();
    (o, out)
}

fn http(sock: &Path, method: &str, path: &str, token: &str) -> (u16, String) {
    let mut s = UnixStream::connect(sock).unwrap();
    write!(s, "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nAuthorization: Bearer {token}\r\n\r\n").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, out)
}

/// `(kind, scope, allowlist_json, label, revoked)` of agent tokens.
fn agent_rows(r: &Rig) -> Vec<(String, String, String, String, bool)> {
    let c = agentd_db::Connection::open(r.p("agentos/sessions.db")).unwrap();
    let mut st = c
        .prepare("SELECT kind, scope, allowlist_json, label, revoked_at IS NOT NULL FROM api_token WHERE kind = 'agent' ORDER BY id")
        .unwrap();
    st.query_map([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn token_in(out: &Path) -> String {
    wait_until(
        || out.join("ready").exists(),
        "agent probe ran to completion",
    );
    std::fs::read_to_string(out.join("token")).unwrap()
}

#[test]
fn agent_token_via_fd_with_chat_scope_and_allowlist() {
    let r = rig();
    let (o, out) = launch(&r, "a", "7");
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let token = token_in(&out);
    assert!(token.starts_with("agt_"), "value obtained via fd");
    for f in ["environ", "cmdline"] {
        let text = std::fs::read_to_string(out.join(f)).unwrap();
        assert!(!text.contains(&token), "token found in {f}");
    }
    assert!(
        std::fs::read_to_string(out.join("environ"))
            .unwrap()
            .contains("AGENTD_TOKEN_FD="),
        "descriptor number passed, not the value"
    );
    let rows = agent_rows(&r);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let (kind, scope, allow, label, revoked) = &rows[0];
    assert_eq!(
        (kind.as_str(), scope.as_str(), *revoked),
        ("agent", "chat", false)
    );
    assert_eq!(label, "agent:7");
    for m in ["GET /sessions/7/messages", "POST /sessions/7/messages"] {
        assert!(allow.contains(m), "{m} in {allow}");
    }
    let sock = r.p("run/agentd.sock");
    // The token works, but it is neither admin nor GUI: the token list (admin) is closed by the chat scope.
    let (st, body) = http(&sock, "GET", "/v1/tokens", &token);
    assert_eq!(st, 403, "{body}");
    assert!(body.contains("insufficient_scope"), "{body}");
    // An allowed method passes the access check: session 7 exists (`seed_sessions`), no messages: 200 and an empty list.
    let (st, body) = http(&sock, "GET", "/v1/sessions/7/messages", &token);
    assert_eq!(st, 200, "{body}");
    assert!(body.ends_with("\r\n\r\n[]"), "{body}");
    // The agent cannot resolve permission requests.
    let (st, _) = http(&sock, "POST", "/v1/permissions/requests/1/decision", &token);
    assert_ne!(st, 200);
}

#[test]
fn agent_token_denied_outside_unit_and_for_bad_session() {
    let r = rig();
    // Directly from the test: the ancestor is not a "unit".
    let direct = Command::new(AGENTD)
        .args(["agent-launch", "--launch-socket"])
        .arg(r.p("run/launch.sock"))
        .args(["--session", "1", "--", "/bin/true"])
        .output()
        .unwrap();
    assert_eq!(direct.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&direct.stderr).contains("denied: parent"));
    for (i, bad) in ["0", "-1", "x", ""].into_iter().enumerate() {
        let (o, _) = launch(&r, &format!("bad{i}"), bad);
        assert_ne!(o.status.code(), Some(0), "session \"{bad}\"");
    }
    assert!(agent_rows(&r).is_empty(), "no tokens issued");
}

#[test]
fn agent_reissue_revokes_previous_token_of_the_session() {
    let r = rig();
    let (_, a) = launch(&r, "a", "3");
    let old = token_in(&a);
    let (_, b) = launch(&r, "b", "3");
    let new = token_in(&b);
    let (_, c) = launch(&r, "c", "4");
    let other = token_in(&c);
    let sock = r.p("run/agentd.sock");
    assert_eq!(http(&sock, "GET", "/v1/sessions/3/messages", &old).0, 401);
    assert_eq!(http(&sock, "GET", "/v1/sessions/3/messages", &new).0, 200);
    assert_eq!(
        http(&sock, "GET", "/v1/sessions/4/messages", &other).0,
        200,
        "token of another session untouched"
    );
    let rows = agent_rows(&r);
    assert_eq!(
        rows.iter().map(|x| x.4).collect::<Vec<_>>(),
        [true, false, false],
        "{rows:?}"
    );
}

#[test]
fn agent_token_cannot_reach_other_sessions() {
    let r = rig();
    let (_, a) = launch(&r, "a", "3");
    let token = token_in(&a);
    let sock = r.p("run/agentd.sock");
    assert_eq!(http(&sock, "GET", "/v1/sessions/3/messages", &token).0, 200);
    let (st, body) = http(&sock, "GET", "/v1/sessions/9/messages", &token);
    assert_eq!(st, 403, "foreign session is closed: {body}");
}

/// `(action, target, actor, result)` of token issuance records in the journal.
fn issue_journal(r: &Rig) -> Vec<(String, String, String, String)> {
    let c = agentd_db::Connection::open(r.p("agentos/sessions.db")).unwrap();
    let mut st = c
        .prepare("SELECT action, target, actor, result FROM actions_log WHERE action = 'token_issue' ORDER BY id")
        .unwrap();
    st.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

#[test]
fn agent_token_issue_is_journaled_without_the_value() {
    let r = rig();
    let (_, a) = launch(&r, "a", "5");
    let token = token_in(&a);
    let rows = issue_journal(&r);
    assert_eq!(
        rows,
        [(
            "token_issue".into(),
            "agent:5".into(),
            "user".into(),
            "ok".into()
        )],
        "{rows:?}"
    );
    let c = agentd_db::Connection::open(r.p("agentos/sessions.db")).unwrap();
    let all: String = c
        .query_row(
            "SELECT group_concat(coalesce(args_json,'') || coalesce(target,'') || coalesce(result,'')) FROM actions_log",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!all.contains(&token), "token value in the journal");
    // A denied request does not appear in the issuance journal.
    let direct = Command::new(AGENTD)
        .args(["agent-launch", "--launch-socket"])
        .arg(r.p("run/launch.sock"))
        .args(["--session", "5", "--", "/bin/true"])
        .output()
        .unwrap();
    assert_eq!(direct.status.code(), Some(3));
    assert_eq!(issue_journal(&r).len(), 1);
}

#[test]
fn agent_token_cannot_decide_or_change_permissions() {
    let r = rig();
    let (_, a) = launch(&r, "a", "2");
    let token = token_in(&a);
    let sock = r.p("run/agentd.sock");
    for (m, p) in [
        ("POST", "/v1/permission-requests/1/decision"),
        ("PUT", "/v1/permissions/global/0/shell"),
        ("GET", "/v1/permissions"),
        ("POST", "/v1/tokens"),
        ("DELETE", "/v1/sessions/2"),
    ] {
        let (st, body) = http(&sock, m, p, &token);
        assert_eq!(st, 403, "{m} {p}: {body}");
    }
}

#[test]
fn agent_token_denied_for_unknown_session() {
    let r = rig();
    let (o, _) = launch(&r, "a", "42");
    assert_eq!(o.status.code(), Some(3));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("denied: no_session"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        agent_rows(&r).is_empty(),
        "no token issued for a nonexistent session"
    );
}
