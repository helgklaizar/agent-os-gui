//! `shell` executor: the agent command goes to the agent runner (the `ShellRunner` port), session subscribers get the frames
//! `terminal.command`, `terminal.output`, `terminal.status` (`docs/reference/api.md` 4.4). The GUI does not execute the command and does not
//! return the result. Runner unavailable or not allowing — the command is not executed (fail-closed), status `denied`.
//! If the reply is interrupted (`stop`), the executor future is dropped, the runner connection closes, the runner kills the process
//! group, and the guard publishes `killed`.

use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};

use crate::shell_token::{AgentToken, AgentTokens, RevokeGuard};
use crate::{ActionCall, ActionExecutor, ExecError, Notifier, PortFuture, TreeStore};

/// Accumulated command output limit, bytes (`docs/reference/api.md` 4.4).
pub const MAX_TERMINAL_OUTPUT: usize = 256 * 1024;
/// Default command timeout and upper bound for `timeout_secs` from a model call, seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
pub const MAX_TIMEOUT_SECS: u64 = 600;
/// How much output goes into the tool result for the model (it sees nothing else).
const RESULT_OUTPUT_BYTES: usize = 64 * 1024;

/// What we ask of the runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    pub cwd: String,
    pub command: String,
    pub timeout_secs: u64,
    pub max_output: usize,
    /// Token for the command descriptor; `None` — the command runs without a token.
    pub token: Option<AgentToken>,
}

/// Runner reply events before the outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    /// The runner accepted the request and started the command.
    Started,
    Output {
        stream: &'static str,
        data: String,
    },
}

/// Command outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDone {
    pub exit_code: i32,
    pub duration_ms: u64,
    pub truncated: bool,
    pub timeout: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerError {
    /// The runner does not answer (socket closed, unit stopped): the command was not executed.
    Unavailable,
    /// The runner refused by policy (`peer`, `dir`, `forbidden`, `bad_request`).
    Denied(String),
    /// The runner dropped the connection mid-command.
    Lost,
}

/// Agent runner port. The implementation is a socket client in the binary; in tests, a real runner behind a substitute socket.
pub trait ShellRunner: Send + Sync + 'static {
    fn run<'a>(
        &'a self,
        spec: RunSpec,
        sink: &'a (dyn Fn(RunEvent) + Send + Sync),
    ) -> PortFuture<'a, Result<RunDone, RunnerError>>;
}

/// Publishes `killed` if the executor did not reach an outcome (the future was dropped by stopping the reply).
struct KillGuard {
    notifier: Arc<dyn Notifier>,
    topic: String,
    request_id: String,
    started: Instant,
    armed: bool,
}

impl Drop for KillGuard {
    fn drop(&mut self) {
        if self.armed {
            let ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
            self.notifier.publish_event(
                &self.topic,
                json!({"kind": "terminal.status", "request_id": self.request_id,
                       "status": "killed", "duration_ms": ms}),
            );
        }
    }
}

pub struct ShellExecutor {
    runner: Arc<dyn ShellRunner>,
    tree: Arc<dyn TreeStore>,
    notifier: Arc<dyn Notifier>,
    tokens: Option<Arc<dyn AgentTokens>>,
}

impl ShellExecutor {
    #[must_use]
    pub fn new(
        runner: Arc<dyn ShellRunner>,
        tree: Arc<dyn TreeStore>,
        notifier: Arc<dyn Notifier>,
    ) -> ShellExecutor {
        ShellExecutor {
            runner,
            tree,
            notifier,
            tokens: None,
        }
    }

    /// Commands get a `kind = agent` token of their session through a descriptor.
    #[must_use]
    pub fn with_tokens(mut self, tokens: Arc<dyn AgentTokens>) -> ShellExecutor {
        self.tokens = Some(tokens);
        self
    }

    /// Session project directory (`path`); none — `None`.
    async fn project_of(&self, session_id: i64) -> Option<String> {
        let s = self.tree.get_session(session_id).await.ok()?;
        s.get("path")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
    }

    fn status(&self, topic: &str, rid: &str, status: &str, extra: Value) {
        let mut v = json!({"kind": "terminal.status", "request_id": rid, "status": status});
        if let (Value::Object(m), Value::Object(e)) = (&mut v, extra) {
            m.extend(e);
        }
        self.notifier.publish_event(topic, v);
    }
}

fn clip(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

impl ActionExecutor for ShellExecutor {
    fn execute<'a>(&'a self, call: &'a ActionCall) -> PortFuture<'a, Result<Value, ExecError>> {
        Box::pin(async move {
            let topic = format!("session.{}", call.session_id);
            let rid = format!("term-{}", uuid::Uuid::now_v7().simple());
            let cwd = self.project_of(call.session_id).await;
            self.notifier.publish_event(
                &topic,
                json!({"kind": "terminal.command", "command": call.target,
                       "cwd": cwd.clone().unwrap_or_default(), "request_id": rid}),
            );
            let Some(cwd) = cwd else {
                self.status(&topic, &rid, "denied", json!({}));
                // The session has no project directory: nowhere to execute, the action is closed (same code as a missing executor).
                return Err(ExecError {
                    code: "executor_unavailable",
                });
            };
            let timeout_secs = call
                .args
                .get("timeout_secs")
                .and_then(Value::as_u64)
                .map_or(DEFAULT_TIMEOUT_SECS, |t| t.clamp(1, MAX_TIMEOUT_SECS));
            let token = match &self.tokens {
                Some(t) => match t.issue(call.session_id).await {
                    Some(tok) => Some(tok),
                    None => {
                        // No token issued — the command is not executed (fail-closed).
                        self.status(&topic, &rid, "denied", json!({}));
                        return Err(ExecError {
                            code: "executor_unavailable",
                        });
                    }
                },
                None => None,
            };
            let revoke = RevokeGuard {
                tokens: self.tokens.clone(),
                id: token.as_ref().map(|t| t.id),
            };
            let spec = RunSpec {
                cwd,
                command: call.target.clone(),
                timeout_secs,
                max_output: MAX_TERMINAL_OUTPUT,
                token,
            };
            let mut guard = KillGuard {
                notifier: self.notifier.clone(),
                topic: topic.clone(),
                request_id: rid.clone(),
                started: Instant::now(),
                armed: false,
            };
            let seq = std::sync::atomic::AtomicU64::new(0);
            let collected = std::sync::Mutex::new(String::new());
            let sink = |ev: RunEvent| match ev {
                RunEvent::Started => {
                    self.status(&topic, &rid, "started", json!({}));
                }
                RunEvent::Output { stream, data } => {
                    let n = seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    if let Ok(mut c) = collected.lock()
                        && c.len() < RESULT_OUTPUT_BYTES
                    {
                        c.push_str(&data);
                    }
                    self.notifier.publish_event(
                        &topic,
                        json!({"kind": "terminal.output", "request_id": rid, "seq": n,
                               "stream": stream, "data": data}),
                    );
                }
            };
            guard.armed = true;
            let res = self.runner.run(spec, &sink).await;
            guard.armed = false;
            // The token lives exactly one command: revoked before the final frame.
            revoke.revoke_now().await;
            match res {
                Ok(done) if done.timeout => {
                    self.status(
                        &topic,
                        &rid,
                        "timeout",
                        json!({"duration_ms": done.duration_ms, "truncated": done.truncated}),
                    );
                    Err(ExecError { code: "timeout" })
                }
                Ok(done) => {
                    self.status(
                        &topic,
                        &rid,
                        "exited",
                        json!({"exit_code": done.exit_code, "duration_ms": done.duration_ms,
                               "truncated": done.truncated}),
                    );
                    let out = collected.lock().map(|c| c.clone()).unwrap_or_default();
                    Ok(
                        json!({"exit_code": done.exit_code, "truncated": done.truncated,
                              "output": clip(out, RESULT_OUTPUT_BYTES)}),
                    )
                }
                Err(RunnerError::Unavailable) => {
                    self.status(&topic, &rid, "denied", json!({}));
                    Err(ExecError {
                        code: "executor_unavailable",
                    })
                }
                Err(RunnerError::Denied(_)) => {
                    self.status(&topic, &rid, "denied", json!({}));
                    Err(ExecError {
                        code: "runner_denied",
                    })
                }
                Err(RunnerError::Lost) => {
                    self.status(&topic, &rid, "killed", json!({}));
                    Err(ExecError {
                        code: "runner_lost",
                    })
                }
            }
        })
    }
}
