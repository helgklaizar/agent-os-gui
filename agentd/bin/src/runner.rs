//! Agent runner (A7-01): `agentd runner` is started via a systemd socket (`Accept=yes`), the accepted connection is standard
//! input. One JSON request line `{cwd, command, timeout_secs?, max_output?}`; any other field is a denial. Before running:
//! the peer uid (`SO_PEERCRED`), the directory after symlink resolution is inside the allowed ones and outside the service ones (`.ssh`, `.agentos`,
//! `.gnupg`). The command runs as `sh -c` in its own process group with a cleared environment; timeout and output limit kill
//! the whole group. The reply is `{stream, seq, data}` lines and a final `{exit, duration_ms, truncated, timeout}` or `{denied}`.

use std::os::fd::{AsFd, OwnedFd};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::Command;

use crate::runner_req::read_request;

const FORBIDDEN: [&str; 3] = [".ssh", ".agentos", ".gnupg"];
const DEFAULT_TIMEOUT: u64 = 120;
const MAX_TIMEOUT: u64 = 3600;
const DEFAULT_OUTPUT: usize = 1_000_000;
const ALLOWED_KEYS: [&str; 5] = ["cwd", "command", "timeout_secs", "max_output", "token_fd"];

#[derive(Debug)]
struct Request {
    cwd: PathBuf,
    command: String,
    timeout: Duration,
    max_output: usize,
    /// The request line arrived together with an agent token descriptor (`SCM_RIGHTS`).
    token_fd: bool,
}

fn parse_request(line: &str) -> Option<Request> {
    let v: Value = serde_json::from_str(line).ok()?;
    let o = v.as_object()?;
    if o.keys().any(|k| !ALLOWED_KEYS.contains(&k.as_str())) {
        return None;
    }
    let command = o.get("command")?.as_str()?.to_string();
    if command.trim().is_empty() || command.len() > 16 * 1024 {
        return None;
    }
    let cwd = PathBuf::from(o.get("cwd")?.as_str()?);
    let timeout = match o.get("timeout_secs") {
        None => DEFAULT_TIMEOUT,
        Some(t) => t.as_u64().filter(|t| (1..=MAX_TIMEOUT).contains(t))?,
    };
    let max_output = match o.get("max_output") {
        None => DEFAULT_OUTPUT,
        Some(m) => usize::try_from(m.as_u64()?)
            .ok()
            .filter(|m| (1..=DEFAULT_OUTPUT).contains(m))?,
    };
    let token_fd = match o.get("token_fd") {
        None => false,
        Some(Value::Bool(true)) => true,
        Some(_) => return None,
    };
    Some(Request {
        token_fd,
        cwd,
        command,
        timeout: Duration::from_secs(timeout),
        max_output,
    })
}

/// Working directory: absolute, exists, after symlink resolution lies inside an allowed one and does not enter service ones.
fn check_dir(cwd: &Path, allowed: &[PathBuf]) -> Result<PathBuf, &'static str> {
    if !cwd.is_absolute() {
        return Err("dir");
    }
    let real = std::fs::canonicalize(cwd).map_err(|_| "dir")?;
    let Some(rel) = allowed
        .iter()
        .find_map(|a| real.strip_prefix(a).ok().map(Path::to_path_buf))
    else {
        return Err("dir");
    };
    let hits_forbidden = rel
        .components()
        .any(|c| matches!(c, Component::Normal(n) if FORBIDDEN.iter().any(|f| n == *f)));
    if hits_forbidden {
        return Err("forbidden");
    }
    Ok(real)
}

async fn send(w: &mut (impl AsyncWriteExt + Unpin), v: &Value) -> bool {
    let mut line = v.to_string();
    line.push('\n');
    w.write_all(line.as_bytes()).await.is_ok()
}

pub async fn run(allow_uid: u32, dirs: Vec<PathBuf>) -> Result<()> {
    let fd = std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .context("standard input")?;
    let std_stream = std::os::unix::net::UnixStream::from(fd);
    std_stream.set_nonblocking(true)?;
    let stream = UnixStream::from_std(std_stream).context("runner connection")?;
    let allowed: Vec<PathBuf> = dirs
        .iter()
        .filter_map(|d| std::fs::canonicalize(d).ok())
        .collect();
    let peer_ok = stream.peer_cred().ok().map(|c| c.uid()) == Some(allow_uid);
    let first = if peer_ok {
        read_request(&stream).await
    } else {
        None
    };
    let (rd, mut wr) = stream.into_split();
    let result = session(rd, &mut wr, peer_ok, first, &allowed).await;
    // End of reply is closing the write side: the client sees EOF even if someone else holds the descriptor.
    drop(wr.shutdown().await);
    result
}

async fn session(
    rd: tokio::net::unix::OwnedReadHalf,
    wr: &mut tokio::net::unix::OwnedWriteHalf,
    peer_ok: bool,
    first: Option<(String, Option<OwnedFd>)>,
    allowed: &[PathBuf],
) -> Result<()> {
    if !peer_ok {
        tracing::warn!(
            scope = "runner",
            event = "denied",
            reason = "peer",
            "foreign uid"
        );
        send(wr, &json!({"denied": "peer"})).await;
        return Ok(());
    }
    let Some((req, token)) = first.and_then(|(line, fd)| parse_request(&line).map(|r| (r, fd)))
    else {
        send(wr, &json!({"denied": "bad_request"})).await;
        return Ok(());
    };
    // The flag and the descriptor go only together: an extra descriptor or an empty promise is a denial before launch.
    if req.token_fd != token.is_some() {
        send(wr, &json!({"denied": "bad_request"})).await;
        return Ok(());
    }
    let cwd = match check_dir(&req.cwd, allowed) {
        Ok(c) => c,
        Err(why) => {
            tracing::warn!(
                scope = "runner",
                event = "denied",
                reason = why,
                "directory not allowed"
            );
            send(wr, &json!({"denied": why})).await;
            return Ok(());
        }
    };
    send(wr, &json!({"started": true})).await;
    execute(&req, &cwd, wr, rd, token).await
}

enum Stop {
    Eof,
    Timeout,
    Full,
    /// The client closed the connection (the core stopped the reply): the command is killed.
    Closed,
}

/// Finishes when the peer closed the connection (a read returned end or an error).
async fn closed(mut reader: impl AsyncReadExt + Unpin) {
    let mut sink = [0u8; 256];
    while reader.read(&mut sink).await.is_ok_and(|n| n > 0) {}
}

/// Command wrapper with a token: the descriptor from standard input moves to 9, the command's input is `/dev/null`.
const TOKEN_WRAP: &str =
    r#"exec 9<&0 0</dev/null; AGENTD_TOKEN_FD=9; export AGENTD_TOKEN_FD; exec /bin/sh -c "$1""#;

async fn execute(
    req: &Request,
    cwd: &Path,
    wr: &mut (impl AsyncWriteExt + Unpin),
    reader: impl AsyncReadExt + Unpin,
    token: Option<OwnedFd>,
) -> Result<()> {
    let started = Instant::now();
    // The token comes on the wrapper's standard input (`TOKEN_WRAP`): it moves it to descriptor 9 and passes the number in
    // `AGENTD_TOKEN_FD`. A fixed number is needed by `dash`: in `<&N` it understands one digit, while the descriptor
    // received by the runner has a number above 9. The value does not reach the environment or arguments.
    let mut cmd = Command::new("/bin/sh");
    match token {
        Some(fd) => {
            cmd.arg("-c").arg(TOKEN_WRAP).arg("sh").arg(&req.command);
            cmd.stdin(Stdio::from(fd));
        }
        None => {
            cmd.arg("-c").arg(&req.command).stdin(Stdio::null());
        }
    }
    cmd.current_dir(cwd)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("HOME", cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let spawned = cmd.spawn();
    // The runner's descriptor went into `Stdio` and is closed at launch; the token remains only with the command.
    drop(cmd);
    let mut child = spawned.context("command launch")?;
    let pgid = child.id().and_then(|p| i32::try_from(p).ok());
    let (mut out, mut err) = (
        child.stdout.take().context("stdout")?,
        child.stderr.take().context("stderr")?,
    );
    let (mut b1, mut b2) = (vec![0u8; 4096], vec![0u8; 4096]);
    let (mut out_open, mut err_open) = (true, true);
    let (mut seq, mut total, mut truncated) = (0u64, 0usize, false);
    let deadline = tokio::time::Instant::now() + req.timeout;
    let mut stop = Stop::Eof;
    let gone = closed(reader);
    tokio::pin!(gone);
    while out_open || err_open {
        let (name, n, buf) = tokio::select! {
            r = out.read(&mut b1), if out_open => ("stdout", r.unwrap_or(0), &b1),
            r = err.read(&mut b2), if err_open => ("stderr", r.unwrap_or(0), &b2),
            () = tokio::time::sleep_until(deadline) => { stop = Stop::Timeout; break; }
            () = &mut gone => { stop = Stop::Closed; break; }
        };
        if n == 0 {
            if name == "stdout" {
                out_open = false
            } else {
                err_open = false
            }
            continue;
        }
        let take = n.min(req.max_output - total);
        total += take;
        seq += 1;
        let data = String::from_utf8_lossy(&buf[..take]).into_owned();
        if take > 0 && !send(wr, &json!({"stream": name, "seq": seq, "data": data})).await {
            stop = Stop::Full;
            truncated = true;
            break;
        }
        if take < n || total >= req.max_output {
            // Limit reached: the rest of the output is discarded, the group is killed.
            truncated = true;
            stop = Stop::Full;
            break;
        }
    }
    if matches!(stop, Stop::Timeout | Stop::Full | Stop::Closed)
        && let Some(p) = pgid
    {
        agentd_sys::kill_group(p);
    }
    let status = child.wait().await.ok();
    let code = status.and_then(|s| s.code()).unwrap_or(-1);
    send(
        wr,
        &json!({"exit": code, "duration_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "truncated": truncated, "timeout": matches!(stop, Stop::Timeout)}),
    )
    .await;
    Ok(())
}
