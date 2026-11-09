//! A1-01…A1-04: the whole `agentd` binary: start, stop, second instance, DB newer than code, `migrate --check`.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Limit for waiting on readiness and stop. Startup hits fsync when opening the DB, and on a VM with a busy FS journal
/// a single fsync costs seconds (S2-12): a short limit gave a false "socket did not appear", and a truly hung process
/// is still caught by this limit.
const PROCESS_LIMIT: Duration = Duration::from_secs(120);

struct Paths {
    _dir: tempfile::TempDir,
    db: PathBuf,
    sock: PathBuf,
}

fn paths() -> Paths {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agentos/sessions.db");
    let sock = dir.path().join("agentd.sock");
    Paths {
        _dir: dir,
        db,
        sock,
    }
}

fn agentd(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_agentd"));
    c.args(args)
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("AGENTD_DB")
        .env_remove("AGENTD_SOCKET")
        .env_remove("AGENTD_TCP");
    c
}

/// A child `agentd` that will not outlive the test, even if the test failed an assertion.
struct Daemon(Child);

impl std::ops::Deref for Daemon {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.0
    }
}

impl std::ops::DerefMut for Daemon {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // An already finished process cannot be killed: this is not a test error.
        if self.0.try_wait().ok().flatten().is_none() {
            drop(self.0.kill());
            drop(self.0.wait());
        }
    }
}

fn serve(p: &Paths) -> Daemon {
    Daemon(
        agentd(&[
            "serve",
            "--db",
            p.db.to_str().unwrap(),
            "--socket",
            p.sock.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("agentd starts"),
    )
}

fn wait_for_socket(sock: &Path, child: &mut Child) {
    let end = Instant::now() + PROCESS_LIMIT;
    while Instant::now() < end {
        if UnixStream::connect(sock).is_ok() {
            return;
        }
        if let Some(st) = child.try_wait().unwrap() {
            let mut err = String::new();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut err)
                .unwrap();
            panic!("agentd exited before ready: {st}: {err}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("socket did not appear within {} s", PROCESS_LIMIT.as_secs());
}

fn get_health(sock: &Path) -> String {
    let mut s = UnixStream::connect(sock).unwrap();
    s.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

fn stop(child: &mut Child) -> std::process::ExitStatus {
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let end = Instant::now() + PROCESS_LIMIT;
    while Instant::now() < end {
        if let Some(st) = child.try_wait().unwrap() {
            return st;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    child.kill().unwrap();
    panic!(
        "agentd did not stop on SIGTERM within {} s",
        PROCESS_LIMIT.as_secs()
    );
}

/// Run with a time limit: a hung `agentd` is killed, the test goes red instead of hanging.
fn run_limited(mut cmd: Command) -> Output {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("agentd starts");
    let end = Instant::now() + PROCESS_LIMIT;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > end {
            child.kill().unwrap();
            let out = child.wait_with_output().unwrap();
            panic!(
                "agentd did not finish within {} s (refusal expected): {}",
                PROCESS_LIMIT.as_secs(),
                text(&out)
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    child.wait_with_output().unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn serve_creates_db_answers_health_and_stops_on_sigterm() {
    let p = paths();
    let mut child = serve(&p);
    wait_for_socket(&p.sock, &mut child);
    let resp = get_health(&p.sock);
    assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    assert!(
        resp.to_ascii_lowercase().contains("x-agentd-version"),
        "{resp}"
    );
    assert!(p.db.exists(), "DB created by migration");
    assert!(stop(&mut child).success(), "graceful stop: code 0");
}

#[test]
fn second_instance_refuses_to_start() {
    let p = paths();
    let mut first = serve(&p);
    wait_for_socket(&p.sock, &mut first);
    let other_sock = p.sock.with_file_name("other.sock");
    let out = run_limited(agentd(&[
        "serve",
        "--db",
        p.db.to_str().unwrap(),
        "--socket",
        other_sock.to_str().unwrap(),
    ]));
    assert!(
        !out.status.success(),
        "second instance on the same DB does not start: {}",
        text(&out)
    );
    assert!(
        text(&out).contains("already open"),
        "clear error: {}",
        text(&out)
    );
    assert!(!other_sock.exists(), "second one's socket not created");
    assert!(
        get_health(&p.sock).starts_with("HTTP/1.1 200"),
        "first is alive"
    );
    assert!(stop(&mut first).success());
}

#[test]
fn db_newer_than_code_refuses() {
    let p = paths();
    std::fs::create_dir_all(p.db.parent().unwrap()).unwrap();
    {
        let c = agentd_db_conn(&p.db);
        c.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL); PRAGMA user_version = 99;").unwrap();
    }
    let out = run_limited(agentd(&[
        "serve",
        "--db",
        p.db.to_str().unwrap(),
        "--socket",
        p.sock.to_str().unwrap(),
    ]));
    assert!(!out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("newer than the code"), "{}", text(&out));
    assert!(!p.sock.exists(), "socket is not opened before migrations");
}

#[test]
fn exit_code_db_newer_than_code() {
    // DB newer than code: a separate exit code 3 (like `migrate --check`), the reason in stderr/journal.
    let p = paths();
    std::fs::create_dir_all(p.db.parent().unwrap()).unwrap();
    {
        let c = agentd_db_conn(&p.db);
        c.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL); PRAGMA user_version = 99;").unwrap();
    }
    let out = run_limited(agentd(&[
        "serve",
        "--db",
        p.db.to_str().unwrap(),
        "--socket",
        p.sock.to_str().unwrap(),
    ]));
    assert_eq!(out.status.code(), Some(3), "{}", text(&out));
    assert!(text(&out).contains("newer than the code"), "{}", text(&out));
}

#[test]
fn other_startup_failures_keep_exit_code_1() {
    let p = paths();
    let out = run_limited(agentd(&[
        "serve",
        "--db",
        "/proc/forbidden/sessions.db",
        "--socket",
        p.sock.to_str().unwrap(),
    ]));
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
}

fn agentd_db_conn(path: &Path) -> agentd_db::Connection {
    agentd_db::Connection::open(path).unwrap()
}

#[test]
fn migrate_check_reports_pending_then_up_to_date() {
    let p = paths();
    let db = p.db.to_str().unwrap();
    let before = agentd(&["migrate", "--check", "--db", db])
        .output()
        .unwrap();
    assert_eq!(
        before.status.code(),
        Some(1),
        "new DB: there is something to apply: {}",
        text(&before)
    );
    assert!(!p.db.exists(), "--check creates and changes nothing");
    let applied = agentd(&["migrate", "--db", db]).output().unwrap();
    assert!(applied.status.success(), "{}", text(&applied));
    let after = agentd(&["migrate", "--check", "--db", db])
        .output()
        .unwrap();
    assert_eq!(after.status.code(), Some(0), "{}", text(&after));
    assert!(text(&after).contains("user_version 10"), "{}", text(&after));
}

#[test]
fn missing_runtime_dir_is_an_error_not_a_fallback() {
    let p = paths();
    let out = run_limited(agentd(&["serve", "--db", p.db.to_str().unwrap()]));
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    assert!(text(&out).contains("XDG_RUNTIME_DIR"), "{}", text(&out));
}

#[test]
fn unknown_and_unimplemented_subcommands_fail_loudly() {
    assert_eq!(agentd(&[]).output().unwrap().status.code(), Some(2));
    assert_eq!(agentd(&["bogus"]).output().unwrap().status.code(), Some(2));
    // gui-launch without the service socket path and without XDG_RUNTIME_DIR is a usage error, not a silent substitution.
    let gui = agentd(&["gui-launch", "--", "true"]).output().unwrap();
    assert_eq!(gui.status.code(), Some(2));
    assert!(text(&gui).contains("XDG_RUNTIME_DIR"), "{}", text(&gui));
}
