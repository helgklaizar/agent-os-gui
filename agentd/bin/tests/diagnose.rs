//! A7-10: `agentd diagnose` is a report without PII: versions, environment, `ptrace_scope`, DB version, `GET /health`; paths, user
//! names and tokens do not get into the report (an unsafe string is replaced with `<dropped: unsafe>`).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn agentd(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_agentd"));
    c.args(args)
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("AGENTD_DB")
        .env_remove("AGENTD_SOCKET");
    c
}

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        drop(self.0.kill());
        drop(self.0.wait());
    }
}

fn health_ok(sock: &std::path::Path) -> bool {
    let Ok(mut s) = UnixStream::connect(sock) else {
        return false;
    };
    drop(s.write_all(b"GET /v1/health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"));
    let mut out = String::new();
    drop(s.read_to_string(&mut out));
    out.contains("\"status\":\"ok\"")
}

#[test]
fn diagnose_reports_live_core_without_personal_data() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sock) = (
        dir.path().join("secretdir/sessions.db"),
        dir.path().join("agentd.sock"),
    );
    let d = Daemon(
        agentd(&[
            "serve",
            "--db",
            db.to_str().unwrap(),
            "--socket",
            sock.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(15);
    while !health_ok(&sock) {
        assert!(Instant::now() < end, "core not ready");
        std::thread::sleep(Duration::from_millis(25));
    }
    let o = agentd(&[
        "diagnose",
        "--db",
        db.to_str().unwrap(),
        "--socket",
        sock.to_str().unwrap(),
    ])
    .output()
    .unwrap();
    drop(d);
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(o.status.success(), "{text}");
    for want in [
        "agentd ",
        "os: ",
        "ptrace_scope: ",
        "disk_encryption: ",
        "health: ok",
        "db: user_version 10 of 10",
    ] {
        assert!(text.contains(want), "no \"{want}\" in:\n{text}");
    }
    let dir_s = dir.path().to_string_lossy().to_string();
    for bad in [dir_s.as_str(), "secretdir", "agt_"] {
        assert!(!text.contains(bad), "PII \"{bad}\" in the report:\n{text}");
    }
    assert!(text.len() < 256 * 1024);
}

#[test]
fn diagnose_without_core_says_unavailable_and_still_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let o = agentd(&[
        "diagnose",
        "--db",
        dir.path().join("none.db").to_str().unwrap(),
        "--socket",
        dir.path().join("none.sock").to_str().unwrap(),
    ])
    .output()
    .unwrap();
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(o.status.success(), "{text}");
    assert!(text.contains("health: unavailable"), "{text}");
    assert!(text.contains("db: absent"), "{text}");
}
