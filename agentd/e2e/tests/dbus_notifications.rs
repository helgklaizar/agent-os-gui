//! A7-05: interception of OS notifications (`org.freedesktop.Notifications`) on the user's session bus and conversion into events
//! `msg.sys` with `source = agent-sys`. The tests start a real `dbus-daemon --session` and send calls with `gdbus` (Linux only; macOS has no such bus).
//! Notification text is untrusted: stored as data; the rate is limited to 20 events per minute per application, the rest
//! is folded into one "N notifications" event (the test uses a 3 s window instead of a minute).
#![cfg(target_os = "linux")]

mod common;

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use common::Env;
use serde_json::Value;

struct Bus {
    child: Child,
    addr: String,
}

impl Bus {
    fn start() -> Bus {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        Bus {
            child,
            addr: line.trim().to_string(),
        }
    }

    /// A notification from application `app` (there is no recipient on the bus: only the monitor sees the message). Sent with `gdbus`;
    /// strings are passed as GVariant text in single quotes with escaping.
    fn notify(&self, app: &str, summary: &str, body: &str, urgency: u8) {
        let q = |s: &str| format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"));
        let out = Command::new("gdbus")
            .env("DBUS_SESSION_BUS_ADDRESS", &self.addr)
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.Notifications",
                "--object-path",
                "/org/freedesktop/Notifications",
                "--method",
                "org.freedesktop.Notifications.Notify",
            ])
            .arg(q(app))
            .arg("uint32 0")
            .arg("''")
            .arg(q(summary))
            .arg(q(body))
            .arg("@as []")
            .arg(format!("{{'urgency': <byte {urgency}>}}"))
            .arg("0")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        // No recipient: we expect exactly "service not found", not an argument parsing error.
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("ServiceUnknown") || err.contains("not provided by any .service"),
            "gdbus: {}",
            err.chars().take(300).collect::<String>()
        );
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        drop(self.child.kill());
        drop(self.child.wait());
    }
}

async fn start(bus: &Bus) -> Env {
    Env::start_with_env(vec![
        ("DBUS_SESSION_BUS_ADDRESS".into(), bus.addr.clone()),
        ("AGENTD_NOTIFY_WINDOW_SECS".into(), "3".into()),
    ])
    .await
}

async fn sys_events(e: &Env, app: &str) -> Vec<Value> {
    let (st, v) = e.read.get("/events?kind=msg&sub=sys&limit=200").await;
    assert_eq!(st, 200, "{v}");
    let mut items: Vec<Value> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| {
            i["text"]
                .as_str()
                .is_some_and(|t| t.starts_with(&format!("{app}:")))
        })
        .cloned()
        .collect();
    items.reverse();
    items
}

async fn wait_events(e: &Env, app: &str, n: usize, secs: u64) -> Vec<Value> {
    let mut last = Vec::new();
    for _ in 0..secs * 4 {
        last = sys_events(e, app).await;
        if last.len() >= n {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("expected {n} events {app}, got {}: {last:?}", last.len());
}

#[tokio::test(flavor = "multi_thread")]
async fn dbus_notification_to_event() {
    let bus = Bus::start();
    let e = start(&bus).await;
    // The bus subscription is set up asynchronously: keep sending until the event appears.
    let mut got = Vec::new();
    for _ in 0..20 {
        bus.notify("mail", "New mail", "from Anna", 1);
        got = sys_events(&e, "mail").await;
        if !got.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let ev = got.first().expect("notification event");
    assert_eq!(ev["source"], "agent-sys", "{ev}");
    assert_eq!(ev["kind"], "msg");
    assert_eq!(ev["sub"], "sys");
    assert!(ev["text"].as_str().unwrap().contains("New mail"), "{ev}");
    assert_eq!(ev["level"], "info", "normal urgency");
    bus.notify("mail", "Urgent", "server down", 2);
    let all = wait_events(&e, "mail", 2, 10).await;
    assert_eq!(all.last().unwrap()["level"], "warn", "critical urgency");
}

#[tokio::test(flavor = "multi_thread")]
async fn dbus_rate_limited() {
    let bus = Bus::start();
    let e = start(&bus).await;
    // Wait for the subscription with one probe notification from another application.
    for _ in 0..20 {
        bus.notify("probe", "ready", "", 1);
        if !sys_events(&e, "probe").await.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    for i in 0..30 {
        bus.notify("spam", &format!("message {i}"), "", 1);
    }
    // 20 separate events and one summary "10 notifications" after the window ends.
    let all = wait_events(&e, "spam", 21, 15).await;
    assert_eq!(all.len(), 21, "{all:?}");
    let last = all.last().unwrap()["text"].as_str().unwrap();
    assert!(last.contains("10 notifications"), "{last}");
    assert!(
        all[..20]
            .iter()
            .all(|i| !i["text"].as_str().unwrap().contains("notifications")),
        "the first 20 are separate"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dbus_evil_text_stored_verbatim() {
    let bus = Bus::start();
    let e = start(&bus).await;
    let evil = "<script>alert(1)</script>**bold** [x](javascript:alert(1)) '; DROP TABLE events;--";
    // 100k ASCII characters: one exec argument cannot exceed 128 KiB.
    let big = "x".repeat(100_000);
    let mut got = Vec::new();
    for _ in 0..20 {
        bus.notify("evil", evil, &big, 1);
        got = sys_events(&e, "evil").await;
        if !got.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let ev = got.first().expect("event");
    let text = ev["text"].as_str().unwrap();
    assert!(
        text.contains(evil),
        "strings are stored as data, unmodified"
    );
    assert!(
        text.chars().count() <= 1000,
        "length is limited by the event field"
    );
    // The DB is intact: the feed keeps responding.
    assert_eq!(e.read.get("/events?kind=msg&sub=sys").await.0, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn dbus_without_session_bus_does_not_break_core() {
    let e = Env::start_with_env(vec![(
        "DBUS_SESSION_BUS_ADDRESS".into(),
        "unix:path=/nonexistent/agentd-test-bus".into(),
    )])
    .await;
    assert_eq!(e.read.get("/health").await.1["status"], "ok");
}
