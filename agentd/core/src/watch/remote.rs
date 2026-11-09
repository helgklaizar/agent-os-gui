//! Observer client in a separate `agent-watch` process: the rules run in a process under its own uid without access
//! to secrets and the network; the core talks to it over a unix socket. Protocol: one JSON line per request, one JSON line per reply
//! (`{"op":"ping"}` → `{"pong":true}`; `{"op":"action","action":"shell","target":…}` and `{"op":"text","text":…}` →
//! `{"hit":null}` or `{"hit":{kind,severity,verdict,rule}}`). Any failure — timeout, disconnect, foreign reply, unknown
//! value in a finding — marks the observer `Unavailable` and closes the action (fail-closed): the command goes to the
//! user as a request, foreign text is blocked.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Action, Hit, Observer, ObserverStatus};

/// Longest process reply line.
const MAX_REPLY: u64 = 64 * 1024;
/// Longer text is cut into overlapping pieces: a marker on a piece boundary is not lost.
const CHUNK: usize = 256 * 1024;
const OVERLAP: usize = 1024;

pub struct RemoteObserver {
    socket: PathBuf,
    timeout: Duration,
    healthy: AtomicBool,
}

fn kind_of(s: &str) -> Option<&'static str> {
    match s {
        "dangerous_cmd" => Some("dangerous_cmd"),
        "injection" => Some("injection"),
        _ => None,
    }
}

fn severity_of(s: &str) -> Option<&'static str> {
    match s {
        "low" => Some("low"),
        "med" => Some("med"),
        "high" => Some("high"),
        _ => None,
    }
}

fn verdict_of(s: &str) -> Option<&'static str> {
    match s {
        "blocked" => Some("blocked"),
        "asked" => Some("asked"),
        _ => None,
    }
}

/// A finding from the process reply; `Err` — the reply violates the protocol.
fn parse_hit(reply: &Value) -> Result<Option<Hit>, ()> {
    let hit = reply.get("hit").ok_or(())?;
    if hit.is_null() {
        return Ok(None);
    }
    let field = |k: &str| hit.get(k).and_then(Value::as_str).ok_or(());
    Ok(Some(Hit {
        kind: kind_of(field("kind")?).ok_or(())?,
        severity: severity_of(field("severity")?).ok_or(())?,
        verdict: verdict_of(field("verdict")?).ok_or(())?,
        rule: field("rule")?.chars().take(200).collect(),
    }))
}

/// Text pieces for checking: whole if short.
fn chunks(text: &str) -> Vec<&str> {
    if text.len() <= CHUNK {
        return vec![text];
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + CHUNK).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        out.push(&text[start..end]);
        if end == text.len() {
            break;
        }
        let mut next = end.saturating_sub(OVERLAP).max(start + 1);
        while !text.is_char_boundary(next) {
            next += 1;
        }
        start = next;
    }
    out
}

impl RemoteObserver {
    /// Connects to the process socket and verifies it with a ping: until the first reply the observer is unavailable.
    #[must_use]
    pub fn new(socket: PathBuf, timeout: Duration) -> RemoteObserver {
        let me = RemoteObserver {
            socket,
            timeout,
            healthy: AtomicBool::new(false),
        };
        me.ping();
        me
    }

    /// Availability heartbeat every `every`: the process crashes and comes back without the core's involvement.
    pub fn spawn_heartbeat(self: &Arc<Self>, every: Duration) {
        let me = self.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(every);
                me.ping();
            }
        });
    }

    fn ping(&self) {
        let ok = self
            .call(&json!({"op": "ping"}))
            .is_ok_and(|r| r.get("pong") == Some(&Value::Bool(true)));
        self.set_health(ok);
    }

    fn set_health(&self, ok: bool) {
        if self.healthy.swap(ok, Ordering::SeqCst) != ok {
            tracing::warn!(
                scope = "observer",
                event = if ok {
                    "watch_proc_up"
                } else {
                    "watch_proc_down"
                },
                "observer process availability changed"
            );
        }
    }

    /// One request and one reply on a new connection; `Err` — a failure code without process text.
    fn call(&self, req: &Value) -> Result<Value, &'static str> {
        let s = UnixStream::connect(&self.socket).map_err(|_| "io")?;
        s.set_read_timeout(Some(self.timeout)).map_err(|_| "io")?;
        s.set_write_timeout(Some(self.timeout)).map_err(|_| "io")?;
        let mut w = s.try_clone().map_err(|_| "io")?;
        writeln!(w, "{req}").map_err(|_| "io")?;
        let mut line = String::new();
        BufReader::new(s.take(MAX_REPLY))
            .read_line(&mut line)
            .map_err(|_| "io")?;
        if line.is_empty() {
            return Err("io");
        }
        let v: Value = serde_json::from_str(&line).map_err(|_| "protocol")?;
        if v.get("error").is_some() {
            return Err("protocol");
        }
        Ok(v)
    }

    /// Finding request; a failure is `Err` and unavailability.
    fn hit(&self, req: &Value) -> Result<Option<Hit>, &'static str> {
        let r = self
            .call(req)
            .and_then(|r| parse_hit(&r).map_err(|()| "protocol"));
        self.set_health(r.is_ok());
        r
    }
}

impl Observer for RemoteObserver {
    fn status(&self) -> ObserverStatus {
        if self.healthy.load(Ordering::SeqCst) {
            ObserverStatus::Available
        } else {
            ObserverStatus::Unavailable
        }
    }

    fn check_action(&self, action: Action, target: &str) -> Option<Hit> {
        if action != Action::Shell {
            return None;
        }
        match self.hit(&json!({"op": "action", "action": "shell", "target": target})) {
            Ok(h) => h,
            Err(code) => Some(Hit {
                kind: "dangerous_cmd",
                severity: "med",
                verdict: "asked",
                rule: format!("watch_unavailable:{code}"),
            }),
        }
    }

    fn check_text(&self, text: &str) -> Option<Hit> {
        for part in chunks(text) {
            match self.hit(&json!({"op": "text", "text": part})) {
                Ok(None) => {}
                Ok(h) => return h,
                Err(code) => {
                    return Some(Hit {
                        kind: "injection",
                        severity: "high",
                        verdict: "blocked",
                        rule: format!("watch_unavailable:{code}"),
                    });
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_is_chunked_with_overlap_on_char_boundaries() {
        let text = "é".repeat(CHUNK); // 2 bytes per char: boundaries are not on bytes
        let parts = chunks(&text);
        assert!(parts.len() >= 2);
        assert!(parts.iter().all(|p| p.len() <= CHUNK));
        let marker = format!("{}MARKER{}", "é".repeat(CHUNK / 2 - 3), "é".repeat(10));
        assert!(chunks(&marker).iter().any(|p| p.contains("MARKER")));
        assert_eq!(chunks("short"), vec!["short"]);
    }
}
