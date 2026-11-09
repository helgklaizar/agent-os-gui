//! Tunnel processes (A6-06): `ssh -N` and `wg-quick` as child processes with an empty environment (only `PATH`), status
//! `up|down|error`, restart with a growing delay (1, 2, 4 … 60 s), `msg.sys` events on status change, a journal of start and
//! stop (`actor = user`). The core does not launch the `tailscale`, `cloudflare`, `ngrok` types (`external`).

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agentd_core::{
    Actor, Audit, Decision, EventLevel, Events, NewAction, NewEvent, Tunnel, TunnelStatuses,
    TunnelStore,
};
use tokio::process::{Child, Command};

const PATH: &str = "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";
/// An `ssh` process that has lived this long is considered up.
const UP_AFTER: Duration = Duration::from_millis(1500);
const MAX_PAUSE: u64 = 60;
const TICK: Duration = Duration::from_millis(500);

struct Proc {
    child: Option<Child>,
    started: Instant,
    fails: u32,
    next_try: Instant,
    reported: &'static str,
    kind: String,
    addr: String,
}

pub(crate) struct Runner {
    pub store: Arc<dyn TunnelStore>,
    pub statuses: TunnelStatuses,
    pub events: Events,
    pub audit: Audit,
}

/// Command by type: in a `test-support` build it can be replaced by environment variables (never in a release one).
fn program(kind: &str) -> String {
    if test_build() {
        let var = if kind == "ssh" {
            "AGENTD_TUNNEL_SSH"
        } else {
            "AGENTD_TUNNEL_WG"
        };
        if let Some(p) = std::env::var_os(var) {
            return p.to_string_lossy().into_owned();
        }
    }
    if kind == "ssh" { "ssh" } else { "wg-quick" }.to_string()
}

fn command(kind: &str, args: &[&str]) -> Command {
    let mut c = Command::new(program(kind));
    c.args(args)
        .env_clear()
        .env("PATH", PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if test_build()
        && let Some(p) = std::env::var_os("TUNNEL_FAKE_LOG")
    {
        c.env("TUNNEL_FAKE_LOG", p);
    }
    c
}

/// Core test build: controllable clocks exist only with `test-support` (in release `manual_clock` is `None`).
fn test_build() -> bool {
    agentd_core::manual_clock(0).is_some()
}

/// Interface name for `wg-quick`: short, without a path and without a leading "-".
fn iface_ok(s: &str) -> bool {
    (1..=15).contains(&s.len())
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_=+.-".contains(c))
}

impl Runner {
    async fn note(&self, id: i64, status: &'static str, level: EventLevel) {
        self.statuses.set(id, status);
        let ev = NewEvent::new("msg", "sys", level, format!("Tunnel {id}: {status}"));
        self.events.emit(ev).await;
    }

    async fn journal(&self, id: i64, action: &str, result: &str) {
        let entry = NewAction {
            actor: Actor::User,
            session_id: None,
            persona_id: None,
            action: action.to_string(),
            target: Some(format!("tunnel:{id}")),
            args: None,
            decision: Some(Decision::Allow),
            result: Some(result.to_string()),
        };
        if self.audit.record(entry).await.is_err() {
            tracing::error!(
                scope = "audit",
                event = "record_failed",
                "journal write failed"
            );
        }
    }

    async fn start(&self, t: &Tunnel, procs: &mut HashMap<i64, Proc>) {
        let prev = procs.remove(&t.id);
        let fails = prev.as_ref().map_or(0, |p| p.fails);
        let mut p = Proc {
            child: None,
            started: Instant::now(),
            fails,
            next_try: Instant::now(),
            reported: prev.map_or("down", |p| p.reported),
            kind: t.kind.clone(),
            addr: t.addr.clone(),
        };
        let spawned = if t.kind == "ssh" {
            if t.addr.starts_with('-') {
                None
            } else {
                command(
                    "ssh",
                    &[
                        "-N",
                        "-o",
                        "ExitOnForwardFailure=yes",
                        "-o",
                        "BatchMode=yes",
                        "--",
                        &t.addr,
                    ],
                )
                .spawn()
                .ok()
            }
        } else if iface_ok(&t.addr) {
            command("wg", &["up", &t.addr]).spawn().ok()
        } else {
            None
        };
        match spawned {
            Some(child) => {
                p.child = Some(child);
                self.journal(t.id, "tunnel_start", "ok").await;
            }
            None => {
                p.fails += 1;
                p.next_try = Instant::now() + Duration::from_secs(backoff(p.fails));
                self.journal(t.id, "tunnel_start", "error").await;
                if p.reported != "error" {
                    p.reported = "error";
                    self.note(t.id, "error", EventLevel::Warn).await;
                }
            }
        }
        procs.insert(t.id, p);
    }

    async fn stop(&self, id: i64, p: Proc) {
        if let Some(mut c) = p.child {
            drop(c.kill().await);
        }
        if p.kind == "wireguard" && iface_ok(&p.addr) {
            // `wg-quick down` is a short command; its outcome is visible in the journal.
            let ok = command("wg", &["down", &p.addr])
                .status()
                .await
                .is_ok_and(|s| s.success());
            self.journal(id, "tunnel_stop", if ok { "ok" } else { "error" })
                .await;
        } else {
            self.journal(id, "tunnel_stop", "ok").await;
        }
        self.statuses.set(id, "down");
        if p.reported != "down" {
            self.note(id, "down", EventLevel::Info).await;
        }
    }

    async fn watch(&self, id: i64, p: &mut Proc) {
        let Some(child) = p.child.as_mut() else {
            return;
        };
        let exited = child.try_wait().ok().flatten();
        match (exited, p.kind.as_str()) {
            (Some(st), "wireguard") => {
                p.child = None;
                let (status, level) = if st.success() {
                    ("up", EventLevel::Info)
                } else {
                    ("error", EventLevel::Warn)
                };
                if !st.success() {
                    p.fails += 1;
                    p.next_try = Instant::now() + Duration::from_secs(backoff(p.fails));
                }
                if p.reported != status {
                    p.reported = status;
                    self.note(id, status, level).await;
                }
            }
            (Some(_), _) => {
                p.child = None;
                p.fails += 1;
                p.next_try = Instant::now() + Duration::from_secs(backoff(p.fails));
                if p.reported != "error" {
                    p.reported = "error";
                    self.note(id, "error", EventLevel::Warn).await;
                }
            }
            (None, "ssh") if p.reported != "up" && p.started.elapsed() >= UP_AFTER => {
                p.reported = "up";
                p.fails = 0;
                self.note(id, "up", EventLevel::Info).await;
            }
            _ => {}
        }
    }

    async fn tick(&self, procs: &mut HashMap<i64, Proc>) {
        let Ok(list) = self.store.list().await else {
            return;
        };
        let live: Vec<i64> = list.iter().map(|t| t.id).collect();
        let gone: Vec<i64> = procs
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        for id in gone {
            if let Some(p) = procs.remove(&id) {
                self.stop(id, p).await;
            }
            self.statuses.remove(id);
        }
        for t in list {
            let managed = t.kind == "ssh" || t.kind == "wireguard";
            if !managed {
                self.statuses.set(t.id, "external");
                continue;
            }
            match (t.enabled, procs.contains_key(&t.id)) {
                (false, true) => {
                    if let Some(p) = procs.remove(&t.id) {
                        self.stop(t.id, p).await;
                    }
                }
                (false, false) => self.statuses.set(t.id, "down"),
                (true, false) => self.start(&t, procs).await,
                (true, true) => {
                    if let Some(p) = procs.get_mut(&t.id) {
                        if p.child.is_none()
                            && (p.kind == "ssh" || p.reported == "error")
                            && Instant::now() >= p.next_try
                        {
                            self.start(&t, procs).await;
                        } else {
                            self.watch(t.id, p).await;
                        }
                    }
                }
            }
        }
    }
}

fn backoff(fails: u32) -> u64 {
    (1u64 << fails.saturating_sub(1).min(6)).min(MAX_PAUSE)
}

pub(crate) fn spawn(runner: Runner) {
    tokio::spawn(async move {
        let mut procs: HashMap<i64, Proc> = HashMap::new();
        loop {
            runner.tick(&mut procs).await;
            tokio::time::sleep(TICK).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_is_capped() {
        let v: Vec<u64> = (1..=9).map(backoff).collect();
        assert_eq!(v, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
    }

    #[test]
    fn interface_names() {
        assert!(iface_ok("wg0"));
        for bad in ["", "-x", "a/b", "this-name-is-too-long", "a b"] {
            assert!(!iface_ok(bad), "{bad}");
        }
    }
}
