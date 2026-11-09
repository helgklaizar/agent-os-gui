//! Logic performance. This measures the controllers without GTK and without rendering: the time spent on
//! updating the model and building the view. These tests do not replace measuring frame rendering (16.7 ms at p95) and GUI process CPU over 5 minutes on the target
//! hardware (an assumption until measured on hardware).
//! The thresholds come from the sprint spec; the numbers are printed (`--nocapture`).

use std::time::{Duration, Instant};

use controllers::bar_cluster::{BarCluster, FrameKind};
use controllers::controllers::chat::{ChatController, ChatMsg};
use controllers::controllers::notifications::{NotifMsg, NotificationsController};
use controllers::controllers::session_tree::{SessionTreeController, TreeMsg};
use controllers::notif_types::{EventCounts, EventItem};
use controllers::timezone::Zone;
use controllers::tree_types::{FolderItem, SessionItem};
use controllers::{Controller, DialogKind};
use serde_json::{Value, json};

const SID: i64 = 1;
const FRAME_P95: Duration = Duration::from_micros(16_700);
const FRAME_P99: Duration = Duration::from_millis(33);
const FIRST_SCREEN: Duration = Duration::from_millis(300);
const INPUT_LATENCY: Duration = Duration::from_millis(100);
const POLL_BUDGET: Duration = Duration::from_secs(3);

/// The limit for the current profile. The spec thresholds apply to an optimised build: the strict check is `cargo test --release`;
/// in a debug build the code is an order of magnitude slower, so the limit is ×20 there as a smoke check.
fn limit(d: Duration) -> Duration {
    if cfg!(debug_assertions) { d * 20 } else { d }
}

fn percentile(samples: &mut [Duration], p: f64) -> Duration {
    samples.sort_unstable();
    let idx = ((samples.len() as f64) * p).ceil() as usize;
    samples[idx.saturating_sub(1).min(samples.len() - 1)]
}

fn message(i: i64) -> Value {
    json!({ "id": i, "session_id": SID, "client_id": null, "role": if i % 2 == 0 { "agent" } else { "user" },
            "author": null, "body_md": format!("Message {i} **bold** `code` and [link](https://example.test/{i})"),
            "status": "done", "meta": null, "ts": 1_793_000_000 + i })
}

fn chat_with(count: i64) -> ChatController {
    let mut c = ChatController::new();
    let now = Instant::now();
    c.update(ChatMsg::Select(Some(SID)), now);
    let messages: Vec<Value> = (1..=count).map(message).collect();
    c.update(
        ChatMsg::Frame {
            session_id: SID,
            snapshot: true,
            data: json!({ "session": { "id": SID, "name": "x" }, "messages": messages, "open_requests": [], "tasks": [] }),
        },
        now,
    );
    c
}

#[test]
fn perf_chat_5000_messages_first_screen() {
    let started = Instant::now();
    let c = chat_with(5000);
    let view = c.view();
    let took = started.elapsed();
    println!("perf chat_5000: snapshot and view {took:?}");
    assert!(!view.items.is_empty());
    assert!(
        took <= limit(FIRST_SCREEN),
        "first chat screen: {took:?}, limit {FIRST_SCREEN:?}"
    );
}

#[test]
fn perf_input_latency_while_streaming() {
    let mut c = chat_with(1000);
    let now = Instant::now();
    c.update(
        ChatMsg::Frame {
            session_id: SID,
            snapshot: false,
            data: json!({ "kind": "message.created", "message": { "id": 9000, "session_id": SID, "client_id": null,
                "role": "agent", "author": null, "body_md": "", "status": "streaming", "meta": null, "ts": 1_793_100_000 } }),
        },
        now,
    );
    let (mut offset, mut worst) = (0usize, Duration::ZERO);
    let mut draft = String::new();
    for i in 0..500 {
        let text = "word ";
        c.update(
            ChatMsg::Frame {
                session_id: SID,
                snapshot: false,
                data: json!({ "kind": "message.delta", "message_id": 9000, "offset": offset, "text": text }),
            },
            Instant::now(),
        );
        offset += text.len();
        // a key press between two deltas: latency = model update plus view build
        draft.push('x');
        let started = Instant::now();
        c.update(ChatMsg::DraftChanged(draft.clone()), Instant::now());
        let _ = c.view();
        worst = worst.max(started.elapsed());
        assert!(i < 500);
    }
    println!("perf stream_input: worst input latency {worst:?} over 500 deltas");
    assert!(
        worst <= limit(INPUT_LATENCY),
        "input latency during streaming {worst:?}, limit {INPUT_LATENCY:?}"
    );
}

fn events(count: i64) -> Vec<EventItem> {
    (1..=count)
        .map(|id| EventItem {
            id,
            kind: ["ai", "task", "msg"][(id % 3) as usize].to_string(),
            sub: ["agent", "sched", "wa"][(id % 3) as usize].to_string(),
            level: "info".to_string(),
            text: format!("Event {id}"),
            session_id: Some(id % 50 + 1),
            tab: "chat".to_string(),
            source: None,
            unread: id % 4 == 0,
            ts: 1_793_000_000 + id,
        })
        .collect()
}

#[test]
fn perf_events_1000_update_and_view_within_frame_budget() {
    let mut c = NotificationsController::new();
    let now = Instant::now();
    let items = events(1000);
    c.update(
        NotifMsg::Snapshot {
            items,
            counts: EventCounts::default(),
        },
        now,
    );
    let mut samples = Vec::new();
    for id in 1001..1201 {
        let item = events(1).remove(0);
        let started = Instant::now();
        c.update(NotifMsg::Created(EventItem { id, ..item }), Instant::now());
        let view = c.view();
        samples.push(started.elapsed());
        assert!(view.cards.len() >= 100);
    }
    let (p95, p99) = (
        percentile(&mut samples, 0.95),
        percentile(&mut samples, 0.99),
    );
    println!("perf events_1000: p95 {p95:?}, p99 {p99:?}");
    assert!(p95 <= limit(FRAME_P95), "p95 {p95:?} > {FRAME_P95:?}");
    assert!(p99 <= limit(FRAME_P99), "p99 {p99:?} > {FRAME_P99:?}");
}

#[test]
fn perf_sessions_500_update_and_view_within_frame_budget() {
    let mut c = SessionTreeController::new();
    let now = Instant::now();
    let folders: Vec<FolderItem> = (1..=20)
        .map(|id| FolderItem {
            id,
            parent_id: None,
            name: format!("Folder {id}"),
            persona_id: None,
            open: true,
        })
        .collect();
    let sessions: Vec<SessionItem> = (1..=500)
        .map(|id| SessionItem {
            id,
            folder_id: Some(id % 20 + 1),
            name: format!("Session {id}"),
            ..SessionItem::default()
        })
        .collect();
    c.update(TreeMsg::Snapshot { folders, sessions }, now);
    let mut samples = Vec::new();
    for id in 1..=200 {
        let started = Instant::now();
        c.update(TreeMsg::Opened(id), Instant::now());
        let view = c.view();
        samples.push(started.elapsed());
        assert!(!view.rows.is_empty());
    }
    let (p95, p99) = (
        percentile(&mut samples, 0.95),
        percentile(&mut samples, 0.99),
    );
    println!("perf sessions_500: p95 {p95:?}, p99 {p99:?}");
    assert!(p95 <= limit(FRAME_P95), "p95 {p95:?} > {FRAME_P95:?}");
    assert!(p99 <= limit(FRAME_P99), "p99 {p99:?} > {FRAME_P99:?}");
}

#[test]
fn perf_polling_logic_five_minutes_under_one_percent_cpu() {
    // 5 minutes of polling: a clock tick and a bar.sys frame every second, a quota delta every 10 seconds. 1% of 300 s is 3 s of work.
    let mut c = BarCluster::new(Zone::fixed(7200));
    let quota = |used: f64| {
        json!({ "account_id": 1, "slug": "cl1", "provider": "anthropic", "label": "Claude", "kind": "api_key",
                "window_h": 5.0, "used": used, "rate": 1.0, "reset_at": 1_793_700_000, "eta_at": null, "load": null,
                "usage": null, "limits": null, "updated_at": 1_793_628_309 })
    };
    let started = Instant::now();
    for second in 0..300i64 {
        let unix = 1_793_628_309 + second;
        let sys = json!({ "cpu": second % 100, "gpu": 20, "ram": 66, "disk": 38, "temp": 40 });
        c.on_frame("bar.sys", FrameKind::Delta, &sys, unix, Instant::now());
        if second % 10 == 0 {
            c.on_frame(
                "bar.quota",
                FrameKind::Delta,
                &quota(0.1 + (second as f64) / 1000.0),
                unix,
                Instant::now(),
            );
        }
        c.tick(unix, Instant::now());
        let _ = c.view();
    }
    let spent = started.elapsed();
    println!("perf polling_5min: bar logic over 300 ticks {spent:?} (limit 1% of 300 s = 3 s)");
    assert!(spent <= limit(POLL_BUDGET), "{spent:?}");
}

#[test]
fn perf_percentile_helper_is_correct() {
    let mut v: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
    assert_eq!(percentile(&mut v, 0.95), Duration::from_millis(95));
    assert_eq!(percentile(&mut v, 0.99), Duration::from_millis(99));
    let mut one = vec![Duration::from_millis(7)];
    assert_eq!(percentile(&mut one, 0.99), Duration::from_millis(7));
    let _ = DialogKind::About;
}
