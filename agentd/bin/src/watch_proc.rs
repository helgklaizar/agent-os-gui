//! `agentd watch` is the observer as a separate process (A7-03): command rules and injection markers with no access to
//! secrets, the DB and the network. In production it is the `agent-watch@.service` unit under uid `agentos-watch` via socket activation `Accept=yes`:
//! the accepted connection is standard input. Protocol: one JSON line per request, one JSON line per reply (see
//! `agentd-core::RemoteObserver`). Any extra field, unknown operation or too long line yields `{"error":…}`.
//! The peer must be our own uid only (`--allow-uid`, by default the process's own uid).

use std::os::fd::AsFd;
use std::path::PathBuf;
use std::time::Duration;

use agentd_core::{Action, Hit, Observer, RuleObserver};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// The longest request line: the core splits text into chunks up to 256 KiB, with headroom for JSON escaping.
const MAX_LINE: u64 = 1024 * 1024;
/// A connection with no requests for longer than this is closed.
const IDLE: Duration = Duration::from_secs(60);

fn hit_json(h: Option<Hit>) -> Value {
    match h {
        None => json!({"hit": null}),
        Some(h) => {
            json!({"hit": {"kind": h.kind, "severity": h.severity, "verdict": h.verdict, "rule": h.rule}})
        }
    }
}

fn bad(why: &str) -> Value {
    json!({"error": why})
}

/// Reply to one request; a pure function, checked by unit tests without a socket.
fn handle(rules: &RuleObserver, req: &Value) -> Value {
    let Some(o) = req.as_object() else {
        return bad("bad_request");
    };
    let allowed: &[&str] = match o.get("op").and_then(Value::as_str) {
        Some("ping") => &["op"],
        Some("action") => &["op", "action", "target"],
        Some("text") => &["op", "text"],
        _ => return bad("bad_op"),
    };
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return bad("bad_request");
    }
    match o["op"].as_str() {
        Some("ping") => json!({"pong": true}),
        Some("action") => {
            let (Some(action), Some(target)) = (
                o.get("action")
                    .and_then(Value::as_str)
                    .and_then(Action::parse),
                o.get("target").and_then(Value::as_str),
            ) else {
                return bad("bad_request");
            };
            hit_json(rules.check_action(action, target))
        }
        _ => {
            let Some(text) = o.get("text").and_then(Value::as_str) else {
                return bad("bad_request");
            };
            hit_json(rules.check_text(text))
        }
    }
}

pub async fn run(allow_uid: Option<u32>, rules_dir: Option<PathBuf>) -> Result<()> {
    let rules = RuleObserver::load(rules_dir.as_deref())
        .map_err(|e| anyhow::anyhow!("observer rules: {e:?}"))?;
    let fd = std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .context("standard input")?;
    let std_stream = std::os::unix::net::UnixStream::from(fd);
    std_stream.set_nonblocking(true)?;
    let stream = UnixStream::from_std(std_stream).context("observer connection")?;
    let want = allow_uid.unwrap_or_else(agentd_sys::current_uid);
    if stream.peer_cred().ok().map(|c| c.uid()) != Some(want) {
        tracing::warn!(
            scope = "watch",
            event = "denied",
            reason = "peer",
            "foreign uid"
        );
        return Ok(());
    }
    let (rd, mut wr) = stream.into_split();
    let mut reader = BufReader::new(rd);
    loop {
        let mut line = Vec::new();
        let read = tokio::time::timeout(
            IDLE,
            (&mut reader)
                .take(MAX_LINE + 1)
                .read_until(b'\n', &mut line),
        )
        .await;
        let Ok(Ok(n)) = read else { break };
        if n == 0 {
            break;
        }
        let too_long = line.last() != Some(&b'\n');
        let reply = if too_long {
            bad("too_long")
        } else {
            match serde_json::from_slice::<Value>(&line) {
                Ok(req) => handle(&rules, &req),
                Err(_) => bad("bad_json"),
            }
        };
        let mut out = reply.to_string();
        out.push('\n');
        if wr.write_all(out.as_bytes()).await.is_err() || too_long {
            break;
        }
    }
    drop(wr.shutdown().await);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> RuleObserver {
        RuleObserver::load(None).unwrap()
    }

    #[test]
    fn ping_action_text_and_rejections() {
        let r = rules();
        assert_eq!(handle(&r, &json!({"op": "ping"})), json!({"pong": true}));
        let hit = handle(
            &r,
            &json!({"op": "action", "action": "shell", "target": "rm -rf ~/"}),
        );
        assert_eq!(hit["hit"]["kind"], "dangerous_cmd", "{hit}");
        assert_eq!(
            handle(
                &r,
                &json!({"op": "action", "action": "shell", "target": "ls"})
            ),
            json!({"hit": null})
        );
        for bad_req in [
            json!([1]),
            json!({"op": "x"}),
            json!({"op": "ping", "extra": 1}),
            json!({"op": "action", "action": "shell"}),
            json!({"op": "action", "action": "nope", "target": "ls"}),
            json!({"op": "text"}),
            json!({"op": "text", "text": 5}),
        ] {
            assert!(handle(&r, &bad_req).get("error").is_some(), "{bad_req}");
        }
    }
}
