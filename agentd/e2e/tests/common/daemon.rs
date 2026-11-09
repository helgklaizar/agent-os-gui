//! Starts the real `agentd` binary on a temporary DB.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::bin::agentd_bin;

/// Core readiness timeout: under load (load in the tens) startup takes tens of seconds; polled every 25 ms.
const READY_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) struct Daemon(Option<Child>);

impl Daemon {
    /// `log` is the file the core's stderr goes to (appended); `rust_log` is the core log level.
    pub(super) fn start(
        db: &Path,
        sock: &Path,
        home: &Path,
        log: &Path,
        rust_log: &str,
        test_clock: bool,
        extra_env: &[(String, String)],
    ) -> Daemon {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        let mut cmd = Command::new(agentd_bin());
        cmd.args(["serve", "--db"])
            .arg(db)
            .arg("--socket")
            .arg(sock)
            .env_remove("XDG_RUNTIME_DIR")
            .env("HOME", home)
            .env("RUST_LOG", rust_log)
            .stdout(Stdio::null())
            .stderr(log);
        // Tunnel command override (honoured only by the `test-support` build) and the override command's log.
        cmd.env("AGENTD_TUNNEL_SSH", env!("CARGO_BIN_EXE_tunnel-fake"))
            .env("AGENTD_TUNNEL_WG", env!("CARGO_BIN_EXE_tunnel-fake"))
            .env("TUNNEL_FAKE_LOG", home.join("tunnel.log"))
            .env("XDG_SESSION_ID", "c1")
            .env("AGENTD_RUNNER_SOCKET", sock.with_file_name("runner.sock"))
            .env("AGENTD_WEATHER_URLFILE", home.join("weather.url"))
            .env("AGENTD_WEATHER_EVERY_SECS", "1")
            .env("AGENTD_SYSFS_ROOT", home.join("sysroot"))
            .env("AGENTD_SYS_GATE_FILE", home.join("sys-indicators.on"))
            .env("AGENTD_MODULE_ALLOW_FILE", home.join("module-allow.txt"));
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        if test_clock {
            // Controllable core clock (`POST /v1/_test/advance`): `test-support` build only.
            cmd.env("AGENTD_TEST_CLOCK", "1");
        }
        let daemon = Daemon(Some(cmd.spawn().unwrap()));
        let end = Instant::now() + READY_TIMEOUT;
        while Instant::now() < end {
            if let Ok(mut s) = std::os::unix::net::UnixStream::connect(sock) {
                use std::io::{Read, Write};
                s.write_all(
                    b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .ok();
                let mut out = String::new();
                s.read_to_string(&mut out).ok();
                if out.contains("\"status\":\"ok\"") || out.contains("\"status\":\"degraded\"") {
                    return daemon;
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("agentd was not ready within {} s", READY_TIMEOUT.as_secs());
    }

    /// Core crash: `SIGKILL` without stopping tasks.
    pub(super) fn crash(&mut self) {
        if let Some(mut c) = self.0.take() {
            drop(c.kill());
            drop(c.wait());
        }
    }

    pub(super) fn stop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let st = Command::new("kill")
                .args(["-TERM", &c.id().to_string()])
                .status()
                .unwrap();
            assert!(st.success());
            assert!(c.wait().unwrap().success(), "clean shutdown");
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            drop(c.kill());
            drop(c.wait());
        }
    }
}
