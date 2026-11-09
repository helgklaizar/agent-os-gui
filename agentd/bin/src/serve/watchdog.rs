//! `sd_notify` with no external dependencies (A7-10, ADR-0006): `READY=1` after start, `WATCHDOG=1` at twice the rate of `WatchdogSec`
//! (`WATCHDOG_USEC`), `STOPPING=1` on stop. Works only under systemd: with no `NOTIFY_SOCKET` it does nothing.
//! The `WATCHDOG=1` signal is sent by a task on the same executor that serves requests: a hung executor stops sending.

use std::path::PathBuf;
use std::time::Duration;

use tokio::net::UnixDatagram;

#[derive(Clone)]
pub struct Notifier {
    path: PathBuf,
    period: Option<Duration>,
}

/// Unit environment parsing. Abstract addresses (`@…`) are not supported; `WATCHDOG_PID`, if set, must be our pid.
fn parse(
    notify_socket: Option<&str>,
    watchdog_usec: Option<&str>,
    watchdog_pid: Option<&str>,
    pid: u32,
) -> Option<Notifier> {
    let path = notify_socket.filter(|p| p.starts_with('/'))?;
    let pid_ok = watchdog_pid.is_none_or(|p| p.parse::<u32>().is_ok_and(|p| p == pid));
    let period = watchdog_usec
        .and_then(|u| u.parse::<u64>().ok())
        .filter(|u| *u > 0 && pid_ok)
        .map(|u| Duration::from_micros(u / 2));
    Some(Notifier {
        path: PathBuf::from(path),
        period,
    })
}

impl Notifier {
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let env = |k: &str| std::env::var(k).ok();
        parse(
            env("NOTIFY_SOCKET").as_deref(),
            env("WATCHDOG_USEC").as_deref(),
            env("WATCHDOG_PID").as_deref(),
            std::process::id(),
        )
    }

    async fn send(&self, sock: &UnixDatagram, msg: &str) {
        if let Err(e) = sock.send_to(msg.as_bytes(), &self.path).await {
            tracing::warn!(scope = "api", event = "sd_notify_failed", error = %e, "sd_notify not sent");
        }
    }

    /// Sends `READY=1` and starts the periodic `WATCHDOG=1`.
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let Ok(sock) = UnixDatagram::unbound() else {
                return;
            };
            self.send(&sock, "READY=1").await;
            let Some(period) = self.period else {
                return;
            };
            loop {
                tokio::time::sleep(period).await;
                self.send(&sock, "WATCHDOG=1").await;
            }
        })
    }

    /// `STOPPING=1` before stopping (a blocking send of one datagram).
    pub fn stopping(&self) {
        if let Ok(s) = std::os::unix::net::UnixDatagram::unbound() {
            drop(s.send_to(b"STOPPING=1", &self.path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parse_requires_filesystem_socket_and_matching_pid() {
        assert!(parse(None, Some("1000"), None, 7).is_none());
        assert!(parse(Some("@abstract"), Some("1000"), None, 7).is_none());
        let n = parse(Some("/run/n"), Some("4000000"), None, 7).unwrap();
        assert_eq!(n.period, Some(std::time::Duration::from_secs(2)));
        let other = parse(Some("/run/n"), Some("4000000"), Some("8"), 7).unwrap();
        assert_eq!(other.period, None, "foreign WATCHDOG_PID: no watchdog");
        let mine = parse(Some("/run/n"), Some("4000000"), Some("7"), 7).unwrap();
        assert!(mine.period.is_some());
        assert_eq!(parse(Some("/run/n"), None, None, 7).unwrap().period, None);
        assert_eq!(
            parse(Some("/run/n"), Some("0"), None, 7).unwrap().period,
            None
        );
    }
}
