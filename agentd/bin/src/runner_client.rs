//! Agent runner client (A7-02): socket `/run/agent-runner.sock` (the path can be set via `AGENTD_RUNNER_SOCKET`). One connection is
//! one command: a request line, then `{started}`, `{stream, seq, data}` lines and a final `{exit, …}` or `{denied}`.
//! Closing the connection before the final line makes the runner kill the command's process group.

use std::path::PathBuf;

use agentd_core::{PortFuture, RunDone, RunEvent, RunSpec, RunnerError, ShellRunner};
use serde_json::{Value, json};
use std::os::fd::AsFd;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use tokio::io::Interest;
use tokio::net::UnixStream;

pub const DEFAULT_SOCKET: &str = "/run/agent-runner.sock";

pub struct RunnerClient {
    socket: PathBuf,
}

impl RunnerClient {
    #[must_use]
    pub fn new(socket: PathBuf) -> RunnerClient {
        RunnerClient { socket }
    }

    #[must_use]
    pub fn socket(&self) -> &std::path::Path {
        &self.socket
    }

    /// Path from `AGENTD_RUNNER_SOCKET`, otherwise the system one.
    #[must_use]
    pub fn from_env() -> RunnerClient {
        RunnerClient::new(
            std::env::var_os("AGENTD_RUNNER_SOCKET")
                .map_or_else(|| DEFAULT_SOCKET.into(), Into::into),
        )
    }
}

impl ShellRunner for RunnerClient {
    fn run<'a>(
        &'a self,
        spec: RunSpec,
        sink: &'a (dyn Fn(RunEvent) + Send + Sync),
    ) -> PortFuture<'a, Result<RunDone, RunnerError>> {
        Box::pin(async move {
            let stream = UnixStream::connect(&self.socket)
                .await
                .map_err(|_| RunnerError::Unavailable)?;
            let mut req = json!({"cwd": spec.cwd, "command": spec.command,
                             "timeout_secs": spec.timeout_secs, "max_output": spec.max_output});
            let mut line = String::new();
            // The agent token goes to the runner as a descriptor along with the request line and does not appear in the request text.
            let token_fd = match &spec.token {
                Some(t) => Some(
                    agentd_sys::fdpass::secret_fd("agentd-agent-token", t.expose().as_bytes())
                        .map_err(|_| RunnerError::Unavailable)?,
                ),
                None => None,
            };
            if token_fd.is_some() {
                req["token_fd"] = json!(true);
            }
            line.push_str(&req.to_string());
            line.push('\n');
            if let Some(fd) = &token_fd {
                stream
                    .async_io(Interest::WRITABLE, || {
                        agentd_sys::fdpass::send_fd(&stream, line.as_bytes(), fd.as_fd())
                    })
                    .await
                    .map_err(|_| RunnerError::Unavailable)?;
            }
            drop(token_fd);
            let (rd, mut wr) = stream.into_split();
            if spec.token.is_none() {
                wr.write_all(line.as_bytes())
                    .await
                    .map_err(|_| RunnerError::Unavailable)?;
            }
            let mut lines = BufReader::new(rd).lines();
            let mut seen_any = false;
            loop {
                let next = lines.next_line().await;
                let Ok(Some(l)) = next else {
                    return Err(if seen_any {
                        RunnerError::Lost
                    } else {
                        RunnerError::Unavailable
                    });
                };
                seen_any = true;
                let Ok(v) = serde_json::from_str::<Value>(&l) else {
                    continue;
                };
                if let Some(why) = v.get("denied").and_then(Value::as_str) {
                    return Err(RunnerError::Denied(why.to_string()));
                }
                if v.get("started").is_some() {
                    sink(RunEvent::Started);
                } else if let Some(stream) = v.get("stream").and_then(Value::as_str) {
                    let data = v
                        .get("data")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    sink(RunEvent::Output {
                        stream: if stream == "stderr" {
                            "stderr"
                        } else {
                            "stdout"
                        },
                        data,
                    });
                } else if let Some(code) = v.get("exit").and_then(Value::as_i64) {
                    let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
                    return Ok(RunDone {
                        exit_code: i32::try_from(code).unwrap_or(-1),
                        duration_ms: v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0),
                        truncated: flag("truncated"),
                        timeout: flag("timeout"),
                    });
                }
            }
        })
    }
}
