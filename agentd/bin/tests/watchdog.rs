//! A7-10: `sd_notify`: a running `agentd serve` sends `READY=1` and `WATCHDOG=1` to `NOTIFY_SOCKET` more often than `WatchdogSec`
//! (`WATCHDOG_USEC`); the core's user unit declares `Type=notify` and `WatchdogSec`.

use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const WATCHDOG_USEC: u64 = 4_000_000;
const LIMIT: Duration = Duration::from_secs(60);

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            drop(self.0.kill());
            drop(self.0.wait());
        }
    }
}

fn serve(dir: &Path, notify: &Path) -> Daemon {
    let mut c = Command::new(env!("CARGO_BIN_EXE_agentd"));
    c.arg("serve")
        .arg("--db")
        .arg(dir.join("agentos/sessions.db"))
        .arg("--socket")
        .arg(dir.join("agentd.sock"))
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("AGENTD_DB")
        .env_remove("AGENTD_SOCKET")
        .env_remove("AGENTD_TCP")
        .env_remove("NOTIFY_SOCKET")
        .env_remove("WATCHDOG_USEC")
        .env_remove("WATCHDOG_PID")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c.env("NOTIFY_SOCKET", notify)
        .env("WATCHDOG_USEC", WATCHDOG_USEC.to_string());
    Daemon(c.spawn().expect("agentd starts"))
}

#[test]
fn watchdog_unit_declares_notify_and_watchdog_sec() {
    let unit = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../packaging/deb/usr/lib/systemd/user/agentd.service"),
    )
    .unwrap();
    let lines: Vec<&str> = unit.lines().map(str::trim).collect();
    for need in ["Type=notify", "WatchdogSec=30", "NotifyAccess=main"] {
        assert!(lines.contains(&need), "core unit lacks {need}");
    }
}

#[test]
fn watchdog_notify_sent() {
    let dir = tempfile::tempdir().unwrap();
    let notify = dir.path().join("notify");
    let sock = UnixDatagram::bind(&notify).unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let _d = serve(dir.path(), &notify);
    let start = Instant::now();
    let mut ready = false;
    let mut beats: Vec<Instant> = Vec::new();
    let mut buf = [0u8; 256];
    while start.elapsed() < LIMIT && beats.len() < 3 {
        let Ok(n) = sock.recv(&mut buf) else { continue };
        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
        if text.contains("READY=1") {
            ready = true;
        }
        if text.contains("WATCHDOG=1") {
            beats.push(Instant::now());
        }
    }
    assert!(ready, "READY=1 did not arrive");
    assert_eq!(
        beats.len(),
        3,
        "WATCHDOG=1 arrived fewer than three times within {LIMIT:?}"
    );
    for w in beats.windows(2) {
        let gap = w[1] - w[0];
        assert!(
            gap < Duration::from_micros(WATCHDOG_USEC),
            "interval between WATCHDOG=1 {gap:?} not less than WatchdogSec"
        );
    }
}
