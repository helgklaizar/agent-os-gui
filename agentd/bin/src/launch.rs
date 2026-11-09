//! GUI token issuance (D29, D41, SP-1.5): service socket `agentd-launch.sock` (`0600`) and the `gui-launch` subcommand.
//! The server checks the uid, the requester's ancestor (cage) and the absence of a live GUI, issues a `kind = gui`, `scope = admin` token,
//! revokes the previous ones and hands the value over as a `memfd` descriptor via `SCM_RIGHTS`. The value reaches neither a file nor `environ`.

use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agentd_api::Hub;
use agentd_auth::{NewToken, TokenStore};
use agentd_core::{Actor, Audit, Decision, NewAction, Scope, TokenKind};
use agentd_sys::proc::{self, ProcInfo};
use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

/// What counts as "our own cage".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPolicy {
    /// Ancestor process name (`comm`); default `cage`.
    pub ancestor: String,
    /// logind session service (`greetd`); `None` means do not check.
    pub service: Option<String>,
    /// Process whose descendants get an agent token (the agent unit under `systemd --user`); default `systemd`.
    pub agent_ancestor: String,
    /// Agent unit name; `{}` is the session number (`agent@{}.service`). The token requester must sit in the cgroup of this unit.
    pub agent_unit: String,
}

/// Default for `LaunchPolicy::agent_unit`.
pub const DEFAULT_AGENT_UNIT: &str = "agent@{}.service";

/// Whether a session exists (for issuing an agent token). A storage error means "no" (deny by default).
pub trait SessionLookup: Send + Sync + 'static {
    fn exists(&self, id: i64) -> agentd_core::PortFuture<'_, bool>;
}

impl<T: agentd_core::TreeStore> SessionLookup for T {
    fn exists(&self, id: i64) -> agentd_core::PortFuture<'_, bool> {
        Box::pin(async move { self.get_session(id).await.is_ok() })
    }
}

/// The agent token requester of session `session` sits in the cgroup of its unit (`unit` with `{}` in place of the number).
pub fn check_agent_cgroup(cgroup: &str, unit: &str, session: i64) -> Result<(), &'static str> {
    let name = unit.replace("{}", &session.to_string());
    let held = cgroup
        .lines()
        .filter_map(|l| l.splitn(3, ':').nth(2))
        .any(|path| path.split('/').any(|seg| seg == name));
    if held { Ok(()) } else { Err("unit") }
}

/// Admission to an agent token: the session unit in the requester's cgroup, then the session's existence.
pub async fn check_agent_request(
    sessions: &dyn SessionLookup,
    cgroup: &str,
    unit: &str,
    session: i64,
) -> Result<(), &'static str> {
    check_agent_cgroup(cgroup, unit, session)?;
    if sessions.exists(session).await {
        Ok(())
    } else {
        Err("no_session")
    }
}

/// Shells that cage inserts between itself and the command (`sh -c`).
const WRAPPERS: [&str; 3] = ["sh", "dash", "bash"];

/// What the service socket client asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    /// `GUI\n`.
    Gui,
    /// `AGENT <session>\n`, the session is a positive integer.
    Agent(i64),
}

/// Request line parsing; anything extra or non-numeric is `None` (`bad_request`).
#[must_use]
pub fn parse_request(raw: &[u8]) -> Option<Request> {
    let line = std::str::from_utf8(raw).ok()?.strip_suffix('\n')?;
    if line == "GUI" {
        return Some(Request::Gui);
    }
    let n = line.strip_prefix("AGENT ")?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse::<i64>().ok().filter(|n| *n > 0).map(Request::Agent)
}

/// Decision about the requester: `Ok` or a short denial code (`parent`, `session`, `internal`).
pub fn check_requester(pid: i32, policy: &LaunchPolicy) -> Result<(), &'static str> {
    if let Some(svc) = &policy.service
        && session_service(pid).as_deref() != Some(svc.as_str())
    {
        return Err("session");
    }
    let chain = proc::ancestors(pid);
    if chain.is_empty() {
        return Err("internal");
    }
    check_chain(&chain, &policy.ancestor)
}

/// The first ancestor that is not a wrapper shell must be the required process.
pub fn check_chain(chain: &[ProcInfo], ancestor: &str) -> Result<(), &'static str> {
    match chain.iter().find(|p| !WRAPPERS.contains(&p.comm.as_str())) {
        Some(p) if p.comm == ancestor => Ok(()),
        _ => Err("parent"),
    }
}

fn session_service(pid: i32) -> Option<String> {
    let cg = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let id = cg
        .split('/')
        .find_map(|p| p.strip_prefix("session-")?.strip_suffix(".scope"))?
        .to_string();
    let out = std::process::Command::new("loginctl")
        .args(["show-session", &id, "-p", "Service", "--value"])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

struct Lease {
    pid: i32,
    start: u64,
}

pub struct LaunchService {
    pub store: Arc<TokenStore>,
    pub hub: Hub,
    pub policy: LaunchPolicy,
    /// Action journal: every issued GUI and agent token (A4-13 item 5); the token value is not recorded there.
    pub audit: Audit,
    /// Sessions: an agent token is issued only for an existing one.
    pub sessions: Arc<dyn SessionLookup>,
    lease: Mutex<Option<Lease>>,
}

impl LaunchService {
    #[must_use]
    pub fn new(
        store: Arc<TokenStore>,
        hub: Hub,
        policy: LaunchPolicy,
        audit: Audit,
        sessions: Arc<dyn SessionLookup>,
    ) -> Arc<LaunchService> {
        Arc::new(LaunchService {
            store,
            hub,
            policy,
            audit,
            sessions,
            lease: Mutex::new(None),
        })
    }

    /// "Token issued" journal entry: kind and target label (`gui`, `agent:<session>`), without the value.
    async fn journal_issue(&self, target: &str) {
        let entry = NewAction {
            actor: Actor::User,
            session_id: None,
            persona_id: None,
            action: "token_issue".to_string(),
            target: Some(target.to_string()),
            args: None,
            decision: Some(Decision::Allow),
            result: Some("ok".to_string()),
        };
        if self.audit.record(entry).await.is_err() {
            tracing::error!(
                scope = "audit",
                event = "record_failed",
                "token issue not recorded in the journal"
            );
        }
    }

    /// New GUI token: previous ones are revoked, their WS are closed with `4401`.
    async fn issue(&self, pid: i32, start: u64) -> Result<String, &'static str> {
        let mut lease = self.lease.lock().await;
        if lease
            .as_ref()
            .is_some_and(|l| proc::is_alive(l.pid, l.start))
        {
            return Err("issued");
        }
        let old = self
            .store
            .revoke_active(TokenKind::Gui)
            .await
            .map_err(|_| "internal")?;
        for id in old {
            self.hub.revoke_token(id);
        }
        let issued = self
            .store
            .issue(NewToken {
                label: "GUI".into(),
                kind: TokenKind::Gui,
                scope: Scope::Admin,
                ttl_days: None,
                allowlist: None,
                created_by: None,
            })
            .await
            .map_err(|_| "internal")?;
        *lease = Some(Lease { pid, start });
        self.journal_issue("gui").await;
        Ok(issued.value.expose().clone())
    }

    /// Session agent token: the requester is a descendant of the `agent_ancestor` process, sits in the cgroup of this session's
    /// unit (`agent_unit`), and the session exists; previous tokens of this session are
    /// revoked, their WS are closed with `4401`. The value goes as a `memfd` descriptor, like the GUI one.
    async fn serve_agent(self: Arc<Self>, mut stream: UnixStream, pid: i32, session: i64) {
        let verdict = if cfg!(target_os = "linux") {
            let ancestor = self.policy.agent_ancestor.clone();
            let cgroup = tokio::task::spawn_blocking(move || {
                let chain = proc::ancestors(pid);
                if chain.is_empty() {
                    return Err("internal");
                }
                check_chain(&chain, &ancestor)?;
                std::fs::read_to_string(format!("/proc/{pid}/cgroup")).map_err(|_| "internal")
            })
            .await
            .unwrap_or(Err("internal"));
            match cgroup {
                Ok(cg) => {
                    check_agent_request(&*self.sessions, &cg, &self.policy.agent_unit, session)
                        .await
                }
                Err(e) => Err(e),
            }
        } else {
            Err("unsupported")
        };
        if let Err(reason) = verdict {
            tracing::warn!(
                scope = "auth",
                event = "agent_launch_denied",
                reason,
                "agent token request rejected"
            );
            return deny(&mut stream, reason).await;
        }
        let issued = async {
            let old = self.store.revoke_agent_tokens(session).await?;
            for id in old {
                self.hub.revoke_token(id);
            }
            self.store.issue_agent(session).await
        }
        .await;
        let Ok(issued) = issued else {
            return deny(&mut stream, "internal").await;
        };
        let token = issued.value.expose().clone();
        self.journal_issue(&format!("agent:{session}")).await;
        let sent = async {
            let std = stream.into_std()?;
            std.set_nonblocking(false)?;
            let fd = agentd_sys::fdpass::memfd_with("agentd-agent-token", token.as_bytes())?;
            agentd_sys::fdpass::send_fd(&std, b"OK\n", fd.as_fd())
        }
        .await;
        match sent {
            Ok(()) => tracing::info!(
                scope = "auth",
                event = "agent_token_issued",
                token_id = issued.id,
                session_id = session,
                "agent token issued"
            ),
            Err(e) => {
                tracing::error!(scope = "auth", event = "agent_token_send_failed", kind = ?e.kind(), "descriptor not sent")
            }
        }
    }

    async fn serve_conn(self: Arc<Self>, mut stream: UnixStream) {
        let uid_ok = stream
            .peer_cred()
            .ok()
            .filter(|c| c.uid() == agentd_sys::current_uid())
            .and_then(|c| c.pid());
        let Some(pid) = uid_ok else {
            tracing::warn!(
                scope = "auth",
                event = "launch_denied",
                reason = "uid",
                "GUI token request from a foreign uid"
            );
            return deny(&mut stream, "uid").await;
        };
        let mut buf = [0u8; 48];
        let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await;
        let request = match read {
            Ok(Ok(n)) => parse_request(&buf[..n]),
            _ => None,
        };
        let Some(request) = request else {
            return deny(&mut stream, "bad_request").await;
        };
        if let Request::Agent(session) = request {
            return self.serve_agent(stream, pid, session).await;
        }
        let verdict = if cfg!(target_os = "linux") {
            let policy = self.policy.clone();
            tokio::task::spawn_blocking(move || {
                check_requester(pid, &policy)
                    .and_then(|()| proc::proc_info(pid).map_err(|_| "internal"))
            })
            .await
            .unwrap_or(Err("internal"))
        } else {
            Err("unsupported")
        };
        let info = match verdict {
            Ok(i) => i,
            Err(reason) => {
                tracing::warn!(
                    scope = "auth",
                    event = "launch_denied",
                    reason,
                    "GUI token request rejected"
                );
                return deny(&mut stream, reason).await;
            }
        };
        let token = match self.issue(pid, info.start).await {
            Ok(t) => t,
            Err(reason) => {
                tracing::warn!(
                    scope = "auth",
                    event = "launch_denied",
                    reason,
                    "GUI token request rejected"
                );
                return deny(&mut stream, reason).await;
            }
        };
        let sent = async {
            let std = stream.into_std()?;
            std.set_nonblocking(false)?;
            let fd = agentd_sys::fdpass::memfd_with("agentd-gui-token", token.as_bytes())?;
            agentd_sys::fdpass::send_fd(&std, b"OK\n", fd.as_fd())
        }
        .await;
        match sent {
            Ok(()) => tracing::info!(
                scope = "auth",
                event = "gui_token_issued",
                "GUI token issued"
            ),
            Err(e) => {
                tracing::error!(scope = "auth", event = "gui_token_send_failed", kind = ?e.kind(), "descriptor not sent")
            }
        }
    }
}

async fn deny(stream: &mut UnixStream, reason: &str) {
    if stream
        .write_all(format!("DENY {reason}\n").as_bytes())
        .await
        .is_err()
    {
        tracing::debug!(scope = "auth", event = "launch_reply_failed", "client gone");
    }
}

/// The socket is created under a temporary name, gets `0600` and is renamed; a live one is not hijacked.
pub fn bind(path: &Path) -> Result<UnixListener> {
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        anyhow::bail!("service socket {} is already taken", path.display());
    }
    let tmp = PathBuf::from(format!("{}.{}.new", path.display(), std::process::id()));
    drop(std::fs::remove_file(&tmp));
    let l = UnixListener::bind(&tmp).with_context(|| format!("socket {}", tmp.display()))?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, path)?;
    Ok(l)
}

pub fn spawn(listener: UnixListener, svc: Arc<LaunchService>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((s, _)) => {
                    tokio::spawn(svc.clone().serve_conn(s));
                }
                Err(e) => {
                    tracing::warn!(scope = "auth", event = "launch_accept_error", kind = ?e.kind(), "accept error");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    })
}

/// `agentd gui-launch -- <command>`: takes a token from `agentd` and `exec`s the command with the inherited descriptor.
/// The descriptor number is in `AGENTD_TOKEN_FD`. Exit code: 3 is a denial, 1 other errors, 127 the command did not start.
pub fn gui_launch(socket: &Path, command: &[String]) -> u8 {
    launch_with(socket, b"GUI\n", "gui-launch", command)
}

/// `agentd agent-launch --session N -- <command>`: a `kind = agent` token for the agent process of session `N`; same protocol,
/// descriptor in `AGENTD_TOKEN_FD`. Exit codes as for `gui-launch`.
pub fn agent_launch(socket: &Path, session: i64, command: &[String]) -> u8 {
    launch_with(
        socket,
        format!("AGENT {session}\n").as_bytes(),
        "agent-launch",
        command,
    )
}

fn launch_with(socket: &Path, request: &[u8], name: &str, command: &[String]) -> u8 {
    use std::io::Write;
    let Some((prog, args)) = command.split_first() else {
        eprintln!("agentd: {name}: no command given after --");
        return 2;
    };
    let mut sock = match std::os::unix::net::UnixStream::connect(socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "agentd: {name}: agentd does not answer on {}: {:?}",
                socket.display(),
                e.kind()
            );
            return 1;
        }
    };
    if sock.write_all(request).is_err() {
        eprintln!("agentd: {name}: request not sent");
        return 1;
    }
    let mut buf = [0u8; 64];
    let (n, fd) = match agentd_sys::fdpass::recv_fd(&sock, &mut buf) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("agentd: {name}: no reply received: {:?}", e.kind());
            return 1;
        }
    };
    let Some(fd) = fd else {
        let reply = String::from_utf8_lossy(&buf[..n]);
        let reason = reply.trim().strip_prefix("DENY ").unwrap_or("unknown");
        eprintln!("agentd: {name}: denied: {reason}");
        return 3;
    };
    if agentd_sys::fdpass::make_inheritable(&fd).is_err() {
        eprintln!("agentd: {name}: descriptor is not inherited");
        return 1;
    }
    drop(sock);
    let raw = std::os::fd::AsRawFd::as_raw_fd(&fd);
    // `fd` is deliberately not closed: the launched command inherits it.
    std::mem::forget(fd);
    let err = std::process::Command::new(prog)
        .args(args)
        .env("AGENTD_TOKEN_FD", raw.to_string())
        .exec();
    eprintln!("agentd: {name}: command not started: {:?}", err.kind());
    127
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(comm: &str) -> ProcInfo {
        ProcInfo {
            pid: 1,
            comm: comm.into(),
            ppid: 0,
            start: 0,
        }
    }

    #[test]
    fn request_forms() {
        assert_eq!(parse_request(b"GUI\n"), Some(Request::Gui));
        assert_eq!(parse_request(b"AGENT 12\n"), Some(Request::Agent(12)));
        for bad in [
            &b"GUI"[..],
            b"GUI \n",
            b"AGENT\n",
            b"AGENT \n",
            b"AGENT 0\n",
            b"AGENT -3\n",
            b"AGENT 1x\n",
            b"AGENT +5\n",
            b"AGENT 99999999999999999999\n",
            b"agent 1\n",
            b"",
            b"\xff\n",
        ] {
            assert_eq!(parse_request(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn chain_rules() {
        assert_eq!(check_chain(&[p("cage")], "cage"), Ok(()));
        assert_eq!(
            check_chain(&[p("sh"), p("dash"), p("cage")], "cage"),
            Ok(())
        );
        assert_eq!(
            check_chain(&[p("sh"), p("agent-shell"), p("cage")], "cage"),
            Err("parent")
        );
        assert_eq!(check_chain(&[p("bash"), p("sshd")], "cage"), Err("parent"));
        assert_eq!(check_chain(&[p("sh")], "cage"), Err("parent"));
        assert_eq!(check_chain(&[], "cage"), Err("parent"));
    }

    struct Known(Vec<i64>);

    impl SessionLookup for Known {
        fn exists(&self, id: i64) -> agentd_core::PortFuture<'_, bool> {
            Box::pin(async move { self.0.contains(&id) })
        }
    }

    const TPL: &str = "agent@{}.service";

    fn cg(unit: &str) -> String {
        format!("0::/user.slice/user-1000.slice/user@1000.service/app.slice/{unit}\n")
    }

    #[test]
    fn cgroup_must_hold_the_unit_of_that_session() {
        assert_eq!(check_agent_cgroup(&cg("agent@7.service"), TPL, 7), Ok(()));
        // Foreign session, foreign unit, overlap by session number prefix, empty cgroup.
        for bad in [
            cg("agent@8.service"),
            cg("agent@70.service"),
            cg("agent@7.service.bak"),
            cg("app-foo.service"),
            "0::/user.slice/user-1000.slice/user@1000.service/init.scope\n".to_string(),
            String::new(),
        ] {
            assert_eq!(check_agent_cgroup(&bad, TPL, 7), Err("unit"), "{bad:?}");
        }
        // A template without `{}` is a fixed unit name.
        assert_eq!(
            check_agent_cgroup(&cg("one.service"), "one.service", 5),
            Ok(())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn token_needs_known_session_and_own_unit() {
        let known = Known(vec![7]);
        assert_eq!(
            check_agent_request(&known, &cg("agent@7.service"), TPL, 7).await,
            Ok(())
        );
        assert_eq!(
            check_agent_request(&known, &cg("agent@9.service"), TPL, 9).await,
            Err("no_session")
        );
        assert_eq!(
            check_agent_request(&known, &cg("app-foo.service"), TPL, 7).await,
            Err("unit")
        );
    }
}
