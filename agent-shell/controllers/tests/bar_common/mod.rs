//! Shared bar test helpers: quotas, frames, bus traversal and TZif building.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use controllers::bar_types::{Limit, Limits, QuotaItem, QuotaKind, Usage};
use controllers::controllers::quota::{QuotaController, QuotaListView, QuotaMsg};
use controllers::{Controller, Effect, Fallback};

pub const NOW: i64 = 1_793_628_309; // 2026-11-02 14:05:09 UTC

pub fn t0() -> Instant {
    Instant::now()
}

pub fn later(base: Instant, ms: u64) -> Instant {
    base + Duration::from_millis(ms)
}

/// API key quota: 5 h window, the remainder is set by the `used` share.
pub fn api_key(slug: &str, label: &str, used: f64, rate: f64, reset_in_min: i64) -> QuotaItem {
    QuotaItem {
        account_id: slug.bytes().map(i64::from).sum(),
        slug: slug.to_string(),
        label: label.to_string(),
        provider: "anthropic".to_string(),
        kind: QuotaKind::ApiKey,
        window_h: Some(5.0),
        used: Some(used),
        rate: Some(rate),
        reset_at: Some(NOW + reset_in_min * 60),
        load: None,
        usage: Some(Usage {
            tokens_in: 120_000,
            tokens_out: 0,
            cost: 1.84,
            currency: "USD".to_string(),
        }),
        limits: Some(Limits {
            requests_per_min: Some(Limit {
                limit: 4000,
                remaining: 1500,
                reset_at: None,
            }),
            tokens_per_min: None,
        }),
    }
}

/// Remainder in percent (`remaining_pct` from 0 to 100) instead of the `used` share.
pub fn with_remaining(remaining_pct: f64, rate: f64) -> QuotaItem {
    api_key(
        "cl1",
        "Claude · key-1",
        1.0 - remaining_pct / 100.0,
        rate,
        97,
    )
}

pub fn local(slug: &str, load: f64) -> QuotaItem {
    QuotaItem {
        account_id: 77,
        slug: slug.to_string(),
        label: "Ollama · local".to_string(),
        provider: "ollama".to_string(),
        kind: QuotaKind::Local,
        window_h: Some(0.0),
        used: None,
        rate: None,
        reset_at: None,
        load: Some(load),
        usage: None,
        limits: None,
    }
}

pub fn unknown_limit(slug: &str) -> QuotaItem {
    QuotaItem {
        account_id: 5,
        slug: slug.to_string(),
        label: "Claude · key-3".to_string(),
        provider: "anthropic".to_string(),
        kind: QuotaKind::ApiKey,
        window_h: None,
        used: None,
        rate: None,
        reset_at: None,
        load: None,
        usage: Some(Usage {
            tokens_in: 900,
            tokens_out: 0,
            cost: 0.03,
            currency: "USD".to_string(),
        }),
        limits: None,
    }
}

/// Send a message and return the effects.
pub fn send<C: Controller>(c: &mut C, msg: C::Msg) -> Vec<Effect> {
    c.update(msg, Instant::now())
}

/// Minimal TZif v2: transitions `(time, type index)` and types `(offset, dst)`.
pub fn tzif(transitions: &[(i64, u8)], types: &[(i32, bool)]) -> Vec<u8> {
    let mut out = Vec::new();
    let header = |out: &mut Vec<u8>, timecnt: u32, typecnt: u32, charcnt: u32| {
        out.extend_from_slice(b"TZif2");
        out.extend_from_slice(&[0u8; 15]);
        for n in [0u32, 0, 0, timecnt, typecnt, charcnt] {
            out.extend_from_slice(&n.to_be_bytes());
        }
    };
    header(&mut out, 0, 1, 1); // the v1 block is empty: one type and one name byte
    out.extend_from_slice(&[0u8; 6]);
    out.push(0);
    header(&mut out, transitions.len() as u32, types.len() as u32, 1);
    for (t, _) in transitions {
        out.extend_from_slice(&t.to_be_bytes());
    }
    for (_, idx) in transitions {
        out.push(*idx);
    }
    for (off, dst) in types {
        out.extend_from_slice(&off.to_be_bytes());
        out.push(u8::from(*dst));
        out.push(0);
    }
    out.push(0);
    out.extend_from_slice(b"\n\n");
    out
}

pub fn view_of(
    items: Vec<controllers::bar_types::QuotaItem>,
    fallback: Option<Fallback>,
) -> QuotaListView {
    let mut c = QuotaController::new();
    send(&mut c, QuotaMsg::Clock(NOW));
    send(&mut c, QuotaMsg::Fallback(fallback));
    send(&mut c, QuotaMsg::Snapshot(Some(items)));
    c.view()
}

pub fn tooltip(item: controllers::bar_types::QuotaItem, fallback: Option<Fallback>) -> String {
    view_of(vec![item], fallback).items.remove(0).tooltip
}
