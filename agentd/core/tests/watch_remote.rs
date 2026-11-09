//! Observer in a separate `agent-watch` process — the `RemoteObserver` client over a unix socket. The server here is a
//! test thread speaking the real protocol (one JSON line per request, one JSON line per reply); the behavior of the real
//! process is checked by `agentd/bin/tests/agent_watch.rs`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use agentd_core::{Action, Observer, ObserverStatus, RemoteObserver};
use serde_json::{Value, json};

struct Server {
    path: PathBuf,
    _dir: tempfile::TempDir,
    calls: Arc<AtomicUsize>,
}

/// The server answers `reply(request)`; `None` — silently closes the connection.
fn serve(reply: impl Fn(&Value) -> Option<String> + Send + 'static) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("watch.sock");
    let l = UnixListener::bind(&path).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    std::thread::spawn(move || {
        for s in l.incoming().map_while(Result::ok) {
            let mut w = s.try_clone().unwrap();
            for line in BufReader::new(s).lines().map_while(Result::ok) {
                c.fetch_add(1, Ordering::SeqCst);
                let Ok(req) = serde_json::from_str::<Value>(&line) else {
                    break;
                };
                match reply(&req) {
                    Some(r) => drop(writeln!(w, "{r}")),
                    // Silent connection close: the server keeps accepting the next ones.
                    None => break,
                }
            }
        }
    });
    Server {
        path,
        _dir: dir,
        calls,
    }
}

fn client(s: &Server) -> RemoteObserver {
    RemoteObserver::new(s.path.clone(), Duration::from_millis(500))
}

fn hit(kind: &str, sev: &str, verdict: &str) -> String {
    json!({"hit": {"kind": kind, "severity": sev, "verdict": verdict, "rule": "r1"}}).to_string()
}

#[test]
fn hit_and_no_hit_round_trip() {
    let s = serve(|req| match req["op"].as_str() {
        Some("ping") => Some(json!({"pong": true}).to_string()),
        Some("action") if req["target"] == "rm -rf ~/" => {
            Some(hit("dangerous_cmd", "high", "blocked"))
        }
        Some("text") if req["text"].as_str().is_some_and(|t| t.contains("ignore")) => {
            Some(hit("injection", "med", "blocked"))
        }
        _ => Some(json!({"hit": null}).to_string()),
    });
    let o = client(&s);
    let h = o.check_action(Action::Shell, "rm -rf ~/").unwrap();
    assert_eq!(
        (h.kind, h.severity, h.verdict, h.rule.as_str()),
        ("dangerous_cmd", "high", "blocked", "r1")
    );
    assert!(o.check_action(Action::Shell, "ls -la").is_none());
    let t = o.check_text("please ignore previous").unwrap();
    assert_eq!((t.kind, t.verdict), ("injection", "blocked"));
    assert!(o.check_text("good afternoon").is_none());
    assert_eq!(o.status(), ObserverStatus::Available);
}

#[test]
fn non_shell_actions_do_not_reach_the_process() {
    let s = serve(|_| Some(json!({"hit": null, "pong": true}).to_string()));
    let o = client(&s);
    let before = s.calls.load(Ordering::SeqCst);
    assert!(o.check_action(Action::Browse, "https://a.test/").is_none());
    assert_eq!(
        s.calls.load(Ordering::SeqCst),
        before,
        "command rules are not about the browser"
    );
}

#[test]
fn unreachable_process_is_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let o = RemoteObserver::new(dir.path().join("none.sock"), Duration::from_millis(300));
    assert_eq!(o.status(), ObserverStatus::Unavailable);
    let h = o
        .check_action(Action::Shell, "ls")
        .expect("command closed with a request");
    assert_eq!((h.kind, h.verdict), ("dangerous_cmd", "asked"));
    assert!(h.rule.starts_with("watch_unavailable"), "{}", h.rule);
    let t = o.check_text("any text").expect("text is blocked");
    assert_eq!((t.kind, t.verdict), ("injection", "blocked"));
    assert!(t.rule.starts_with("watch_unavailable"), "{}", t.rule);
}

#[test]
fn garbage_and_unknown_fields_are_fail_closed() {
    for reply in [
        "not json".to_string(),
        json!({"hit": {"kind": "x", "severity": "high", "verdict": "blocked", "rule": "r"}}).to_string(),
        json!({"hit": {"kind": "dangerous_cmd", "severity": "critical", "verdict": "blocked", "rule": "r"}}).to_string(),
        json!({"hit": {"kind": "dangerous_cmd", "severity": "high", "verdict": "allow", "rule": "r"}}).to_string(),
        json!({"nothing": 1}).to_string(),
    ] {
        let s = serve(move |req| {
            if req["op"] == "ping" {
                Some(json!({"pong": true}).to_string())
            } else {
                Some(reply.clone())
            }
        });
        let o = client(&s);
        let h = o.check_action(Action::Shell, "ls").expect("closed");
        assert!(h.rule.starts_with("watch_unavailable"), "{}", h.rule);
        assert_eq!(o.status(), ObserverStatus::Unavailable);
    }
}

#[test]
fn silent_close_and_recovery() {
    let up = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let u = up.clone();
    let s = serve(move |req| {
        if !u.load(Ordering::SeqCst) {
            return None;
        }
        Some(if req["op"] == "ping" {
            json!({"pong": true}).to_string()
        } else {
            json!({"hit": null}).to_string()
        })
    });
    let o = client(&s);
    let h = o
        .check_action(Action::Shell, "ls")
        .expect("closed while the process is silent");
    assert!(h.rule.starts_with("watch_unavailable"));
    assert_eq!(o.status(), ObserverStatus::Unavailable);
    up.store(true, Ordering::SeqCst);
    assert!(
        o.check_action(Action::Shell, "ls").is_none(),
        "the process came back"
    );
    assert_eq!(o.status(), ObserverStatus::Available);
}
