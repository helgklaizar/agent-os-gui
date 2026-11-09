//! S2 mutants: tests killing surviving mutants.

mod bar_common;

use std::cell::Cell;
use std::rc::Rc;

use bar_common::*;
use controllers::bar_cluster::{BarCluster, FrameKind};
use controllers::clock_fmt::format_date;
use controllers::level::Level;
use controllers::timezone::{Zone, ZoneSource, resolve};
use controllers::{BusEvent, ConnState, EventBus, Fallback};
use serde_json::json;

/// Brute-force calendar: month and day for a day count since 1970-01-01.
fn naive_date(mut days: i64) -> (usize, i64) {
    let mut year = 1970;
    loop {
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let len = if leap { 366 } else { 365 };
        if days < len {
            let lens = [
                31,
                if leap { 29 } else { 28 },
                31,
                30,
                31,
                30,
                31,
                31,
                30,
                31,
                30,
                31,
            ];
            let mut m = 0;
            while days >= lens[m] {
                days -= lens[m];
                m += 1;
            }
            return (m, days + 1);
        }
        days -= len;
        year += 1;
    }
}

#[test]
fn clock_date_matches_naive_calendar_for_130_years() {
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    for days in 0..47_500_i64 {
        let (m, d) = naive_date(days);
        let got = format_date(days * 86_400 + 43_200, 0);
        let want = format!("{} {d}", months[m]);
        assert!(got.ends_with(&want), "day {days}: {got} instead of …{want}");
    }
}

#[test]
fn level_worst_picks_higher_rank_either_side() {
    let order = [
        Level::NoData,
        Level::Neutral,
        Level::Ok,
        Level::Warn,
        Level::Bad,
    ];
    for (i, a) in order.iter().enumerate() {
        for (j, b) in order.iter().enumerate() {
            let want = order[i.max(j)];
            assert_eq!(a.worst(*b), want, "{a:?}.worst({b:?})");
        }
    }
}

#[test]
fn bus_subscription_ids_are_distinct() {
    let bus = EventBus::new();
    let hits = Rc::new(Cell::new(0));
    let h1 = Rc::clone(&hits);
    let first = bus.subscribe(move |_| h1.set(h1.get() + 1));
    let h2 = Rc::clone(&hits);
    let second = bus.subscribe(move |_| h2.set(h2.get() + 10));
    let h3 = Rc::clone(&hits);
    let third = bus.subscribe(move |_| h3.set(h3.get() + 100));
    assert!(first != second && second != third && first != third);
    bus.unsubscribe(second);
    bus.publish(BusEvent::LayoutChanged);
    assert_eq!(hits.get(), 101, "only the second one is unsubscribed");
}

#[test]
fn cluster_connecting_changes_nothing_in_metrics_and_quota() {
    let mut c = BarCluster::new(Zone::fixed(0));
    let sys = json!({ "cpu": 14, "gpu": 20, "ram": 66, "disk": 38, "temp": 38 });
    c.on_frame("bar.sys", FrameKind::Snapshot, &sys, NOW, t0());
    c.on_connection(ConnState::Connected, t0());
    let before = c.view();
    let fx = c.on_connection(ConnState::Connecting, t0());
    let after = c.view();
    assert_eq!(
        before.metrics, after.metrics,
        "metrics are untouched on Connecting"
    );
    assert_eq!(
        before.quotas, after.quotas,
        "quotas are untouched on Connecting"
    );
    assert!(fx.is_empty(), "{fx:?}");
}

/// TZif v2 with a non-empty v1 block: one leap second in the v1 data.
fn tzif_with_v1_leap(offset: i32) -> Vec<u8> {
    let mut out = Vec::new();
    let header = |out: &mut Vec<u8>, leap: u32, time: u32, typ: u32, chars: u32| {
        out.extend_from_slice(b"TZif2");
        out.extend_from_slice(&[0u8; 15]);
        for n in [0u32, 0, leap, time, typ, chars] {
            out.extend_from_slice(&n.to_be_bytes());
        }
    };
    header(&mut out, 1, 0, 1, 1);
    out.extend_from_slice(&[0u8; 6]);
    out.push(0);
    out.extend_from_slice(&[0u8; 8]); // v1 leap record
    header(&mut out, 0, 0, 1, 1);
    out.extend_from_slice(&offset.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.push(0);
    out
}

#[test]
fn tzif_skips_v1_leap_records() {
    let z = Zone::parse_tzif(&tzif_with_v1_leap(5400)).expect("parse");
    assert_eq!(z.offset_at(NOW), 5400);
}

#[test]
fn zone_file_size_limit_is_inclusive() {
    let dir = tempfile::tempdir().expect("directory");
    let root = dir.path();
    std::fs::create_dir_all(root.join("Europe")).expect("Europe");
    let base = tzif(&[], &[(18_000, false)]);
    let limit = 1usize << 20;
    for (name, size, ok) in [
        ("Exact", limit, true),
        ("Over", limit + 1, false),
        ("Small", base.len(), true),
    ] {
        let mut bytes = base.clone();
        bytes.resize(size, 0);
        std::fs::write(root.join("Europe").join(name), bytes).expect("file");
        let (z, src) = resolve(Some(&format!("Europe/{name}")), root);
        let want = if ok {
            ZoneSource::System
        } else {
            ZoneSource::Utc
        };
        assert_eq!(src, want, "{name}");
        assert_eq!(z.offset_at(NOW), if ok { 18_000 } else { 0 }, "{name}");
    }
}

#[test]
fn zone_directory_is_not_a_zone() {
    let dir = tempfile::tempdir().expect("directory");
    std::fs::create_dir_all(dir.path().join("Europe/Dir")).expect("zone directory");
    let (_, src) = resolve(Some("Europe/Dir"), dir.path());
    assert_eq!(src, ZoneSource::Utc);
}

#[test]
fn fallback_parse_known_and_unknown() {
    assert_eq!(Fallback::parse("local"), Some(Fallback::Local));
    assert_eq!(Fallback::parse("other"), Some(Fallback::Other));
    assert_eq!(Fallback::parse("wait"), Some(Fallback::Wait));
    assert_eq!(Fallback::parse("later"), None);
    assert_eq!(Fallback::parse(""), None);
}

fn quota_tip(item: controllers::bar_types::QuotaItem) -> String {
    use controllers::Controller;
    use controllers::controllers::quota::{QuotaController, QuotaMsg};
    let mut c = QuotaController::new();
    send(&mut c, QuotaMsg::Clock(NOW));
    send(&mut c, QuotaMsg::Snapshot(Some(vec![item])));
    c.view().items.remove(0).tooltip
}

#[test]
fn quota_reset_line_absent_when_reset_is_now() {
    let tip = quota_tip(api_key("cl1", "Claude · key-1", 0.5, 0.5, 0));
    assert!(!tip.contains("Resets in"), "{tip}");
    let soon = quota_tip(api_key("cl1", "Claude · key-1", 0.5, 0.5, 1));
    assert!(soon.contains("Resets in"), "{soon}");
}

#[test]
fn quota_usage_sums_tokens_in_and_out() {
    let mut item = api_key("cl1", "Claude · key-1", 0.5, 0.5, 60);
    if let Some(u) = item.usage.as_mut() {
        u.tokens_in = 100_000;
        u.tokens_out = 20_000;
    }
    let tip = quota_tip(item);
    assert!(tip.contains("Usage: 120000 tokens"), "{tip}");
}
