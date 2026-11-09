//! A7-10: Healthchecks heartbeat (D43). The address is `AGENTD_HEARTBEAT_URL` (disabled by default), the period is
//! `AGENTD_HEARTBEAT_SECS` (default 300). An enabled `agentd serve` pings the address with `GET`; a disabled one does not ping;
//! `agentd diagnose` shows the state ("disabled"/"enabled") but not the address itself (it contains the secret check identifier).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            drop(self.0.kill());
            drop(self.0.wait());
        }
    }
}

fn agentd(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_agentd"));
    c.args(args)
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("AGENTD_DB")
        .env_remove("AGENTD_SOCKET")
        .env_remove("AGENTD_TCP")
        .env_remove("AGENTD_HEARTBEAT_URL")
        .env_remove("AGENTD_HEARTBEAT_SECS");
    c
}

const SECRET_ID: &str = "hc-uuid-5f1c9e7a";

#[test]
fn heartbeat_pings_when_url_set() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agentos/sessions.db");
    let sock = dir.path().join("agentd.sock");
    let _d = Daemon(
        agentd(&[
            "serve",
            "--db",
            db.to_str().unwrap(),
            "--socket",
            sock.to_str().unwrap(),
        ])
        .env(
            "AGENTD_HEARTBEAT_URL",
            format!("http://127.0.0.1:{port}/ping/{SECRET_ID}"),
        )
        .env("AGENTD_HEARTBEAT_SECS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(90);
    let mut first_lines: Vec<String> = Vec::new();
    l.set_nonblocking(true).unwrap();
    while first_lines.len() < 2 && Instant::now() < end {
        let Ok((mut s, _)) = l.accept() else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        s.set_nonblocking(false).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut buf = [0u8; 2048];
        let n = s.read(&mut buf).unwrap_or(0);
        drop(s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nOK"));
        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
        first_lines.push(text.lines().next().unwrap_or("").to_string());
    }
    assert_eq!(
        first_lines.len(),
        2,
        "expected two pings, got {first_lines:?}"
    );
    for l in &first_lines {
        assert_eq!(l, &format!("GET /ping/{SECRET_ID} HTTP/1.1"));
    }
}

fn diagnose(url: Option<&str>) -> String {
    let dir = tempfile::tempdir().unwrap();
    let mut c = agentd(&[
        "diagnose",
        "--db",
        dir.path().join("none.db").to_str().unwrap(),
        "--socket",
        dir.path().join("none.sock").to_str().unwrap(),
    ]);
    if let Some(u) = url {
        c.env("AGENTD_HEARTBEAT_URL", u);
    }
    let o = c.output().unwrap();
    assert!(o.status.success());
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn heartbeat_disabled_without_url() {
    let text = diagnose(None);
    assert!(text.contains("heartbeat: disabled"), "{text}");
}

#[test]
fn heartbeat_enabled_in_report_without_the_address() {
    let text = diagnose(Some(&format!("https://hc.example.test/ping/{SECRET_ID}")));
    assert!(text.contains("heartbeat: enabled"), "{text}");
    assert!(
        !text.contains(SECRET_ID) && !text.contains("hc.example.test"),
        "{text}"
    );
}

#[test]
fn heartbeat_rejects_non_http_address() {
    let text = diagnose(Some("file:///etc/passwd"));
    assert!(text.contains("heartbeat: disabled"), "{text}");
    assert!(!text.contains("passwd"), "{text}");
}
