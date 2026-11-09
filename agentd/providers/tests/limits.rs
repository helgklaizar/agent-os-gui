//! Limits from headers, reset time.

use agentd_core::{LimitWindow, RateLimit};
use agentd_providers::limits::{
    anthropic_limits, openai_limits, parse_duration_secs, parse_rfc3339,
};
use hyper::HeaderMap;

fn hm(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        h.insert(
            hyper::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
            v.parse().unwrap(),
        );
    }
    h
}

#[test]
fn rfc3339_to_unix() {
    assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_rfc3339("2026-11-03T10:00:00Z"), Some(1_793_700_000));
    assert_eq!(
        parse_rfc3339("2026-11-03T10:00:00.750Z"),
        Some(1_793_700_000)
    );
    assert_eq!(
        parse_rfc3339("2026-11-03T12:00:00+02:00"),
        Some(1_793_700_000)
    );
    for bad in ["", "yesterday", "2026-13-03T10:00:00Z", "2026-11-03"] {
        assert_eq!(parse_rfc3339(bad), None, "{bad}");
    }
}

#[test]
fn duration_forms() {
    for (s, want) in [
        ("1s", 1.0),
        ("6m0s", 360.0),
        ("1h2m3.5s", 3723.5),
        ("250ms", 0.25),
        ("20ms", 0.02),
    ] {
        assert_eq!(parse_duration_secs(s), Some(want), "{s}");
    }
    for bad in ["", "abc", "5", "1x"] {
        assert_eq!(parse_duration_secs(bad), None, "{bad}");
    }
}

#[test]
fn anthropic_headers_applied() {
    let h = hm(&[
        ("anthropic-ratelimit-requests-limit", "50"),
        ("anthropic-ratelimit-requests-remaining", "49"),
        ("anthropic-ratelimit-requests-reset", "2026-11-03T10:00:00Z"),
        ("anthropic-ratelimit-tokens-limit", "40000"),
        ("anthropic-ratelimit-tokens-remaining", "30000"),
        ("anthropic-ratelimit-tokens-reset", "2026-11-03T10:00:30Z"),
    ]);
    let r = anthropic_limits(&h, 1_000);
    assert_eq!(
        r.requests,
        Some(LimitWindow {
            limit: 50,
            remaining: 49,
            reset_at: Some(1_793_700_000)
        })
    );
    assert_eq!(
        r.tokens,
        Some(LimitWindow {
            limit: 40000,
            remaining: 30000,
            reset_at: Some(1_793_700_030)
        })
    );
}

#[test]
fn openai_headers_applied_with_relative_reset() {
    let h = hm(&[
        ("x-ratelimit-limit-requests", "60"),
        ("x-ratelimit-remaining-requests", "59"),
        ("x-ratelimit-reset-requests", "1s"),
        ("x-ratelimit-limit-tokens", "150000"),
        ("x-ratelimit-remaining-tokens", "149000"),
        ("x-ratelimit-reset-tokens", "6m0s"),
    ]);
    let r = openai_limits(&h, 1_000);
    assert_eq!(
        r.requests,
        Some(LimitWindow {
            limit: 60,
            remaining: 59,
            reset_at: Some(1_001)
        })
    );
    assert_eq!(
        r.tokens,
        Some(LimitWindow {
            limit: 150_000,
            remaining: 149_000,
            reset_at: Some(1_360)
        })
    );
}

#[test]
fn no_headers_no_limits_and_garbage_ignored() {
    assert!(anthropic_limits(&HeaderMap::new(), 0).is_empty());
    assert_eq!(openai_limits(&HeaderMap::new(), 0), RateLimit::default());
    let h = hm(&[
        ("x-ratelimit-limit-requests", "many"),
        ("x-ratelimit-remaining-requests", "5"),
    ]);
    assert!(
        openai_limits(&h, 0).is_empty(),
        "no numeric limit means no window"
    );
}
