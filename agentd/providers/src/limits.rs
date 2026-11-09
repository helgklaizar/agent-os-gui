//! Limits from provider response headers (rate-limit): Anthropic and OpenAI-compatible.

use agentd_core::{LimitWindow, RateLimit};
use hyper::HeaderMap;

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// RFC 3339 (`2026-11-03T10:00:00Z`, fractional part and offset allowed) -> unix seconds.
#[must_use]
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    if d.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    let (time, offset) = if let Some(t) = rest.strip_suffix('Z') {
        (t, 0)
    } else {
        let at = rest.rfind(['+', '-'])?;
        let (t, o) = rest.split_at(at);
        let (h, mi) = o[1..].split_once(':')?;
        let secs = h.parse::<i64>().ok()? * 3600 + mi.parse::<i64>().ok()? * 60;
        (t, if o.starts_with('-') { -secs } else { secs })
    };
    let time = time.split('.').next()?;
    let mut t = time.split(':');
    let (h, mi, sec): (i64, i64, i64) = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
    );
    if t.next().is_some() || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    Some(days_from_civil(y, m, day) * 86_400 + h * 3600 + mi * 60 + sec - offset)
}

/// Duration such as `1s`, `6m0s`, `1h2m3.5s`, `250ms` -> seconds.
#[must_use]
pub fn parse_duration_secs(s: &str) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    let mut total = 0.0;
    let mut rest = s;
    while !rest.is_empty() {
        let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
        let num: f64 = rest[..end].parse().ok()?;
        rest = &rest[end..];
        let (mult, len) = if rest.starts_with("ms") {
            (0.001, 2)
        } else if rest.starts_with('h') {
            (3600.0, 1)
        } else if rest.starts_with('m') {
            (60.0, 1)
        } else if rest.starts_with('s') {
            (1.0, 1)
        } else {
            return None;
        };
        total += num * mult;
        rest = &rest[len..];
    }
    Some(total)
}

fn text<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name)?.to_str().ok()
}

fn number(h: &HeaderMap, name: &str) -> Option<u64> {
    text(h, name)?.trim().parse().ok()
}

/// Limit window: needs the limit and the remainder; reset comes from `reset`.
fn window(
    h: &HeaderMap,
    limit: &str,
    remaining: &str,
    reset: impl Fn(&str) -> Option<i64>,
    reset_name: &str,
) -> Option<LimitWindow> {
    Some(LimitWindow {
        limit: number(h, limit)?,
        remaining: number(h, remaining)?,
        reset_at: text(h, reset_name).and_then(reset),
    })
}

/// `anthropic-ratelimit-*` headers: reset is RFC 3339.
#[must_use]
pub fn anthropic_limits(h: &HeaderMap, _now: i64) -> RateLimit {
    let w = |kind: &str| {
        window(
            h,
            &format!("anthropic-ratelimit-{kind}-limit"),
            &format!("anthropic-ratelimit-{kind}-remaining"),
            parse_rfc3339,
            &format!("anthropic-ratelimit-{kind}-reset"),
        )
    };
    RateLimit {
        requests: w("requests"),
        tokens: w("tokens"),
    }
}

/// `x-ratelimit-*` headers: reset is a duration from the moment of the response.
#[must_use]
pub fn openai_limits(h: &HeaderMap, now: i64) -> RateLimit {
    let w = |kind: &str| {
        window(
            h,
            &format!("x-ratelimit-limit-{kind}"),
            &format!("x-ratelimit-remaining-{kind}"),
            |v| {
                // Round up: reset is never earlier than the server says.
                parse_duration_secs(v).map(|d| now + d.ceil() as i64)
            },
            &format!("x-ratelimit-reset-{kind}"),
        )
    };
    RateLimit {
        requests: w("requests"),
        tokens: w("tokens"),
    }
}
