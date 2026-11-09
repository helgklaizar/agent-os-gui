//! Schedule: cron in the task's time zone, DST shifts, boundaries (unit tests `cron_*`).

use agentd_core::{CronError, cron_next};
use chrono::TimeZone;
use chrono_tz::Asia::Jerusalem;

/// 2026-01-15T00:00:00Z, winter (UTC+2).
const WINTER: i64 = 1_768_435_200;
/// 2026-07-15T00:00:00Z, summer (UTC+3).
const SUMMER: i64 = 1_784_073_600;

fn utc(ts: i64) -> String {
    chrono::Utc
        .timestamp_opt(ts, 0)
        .single()
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Local date and time of each of the `n` nearest runs after `from`.
fn locals(expr: &str, from: i64, n: usize) -> Vec<String> {
    let mut t = from;
    let mut out = Vec::new();
    for _ in 0..n {
        t = cron_next(expr, "Asia/Jerusalem", t).unwrap().unwrap();
        out.push(
            Jerusalem
                .timestamp_opt(t, 0)
                .single()
                .unwrap()
                .format("%Y-%m-%d %H:%M")
                .to_string(),
        );
    }
    out
}

#[test]
fn cron_0800_jerusalem_winter_summer_utc() {
    let w = cron_next("0 8 * * *", "Asia/Jerusalem", WINTER)
        .unwrap()
        .unwrap();
    assert_eq!(
        utc(w),
        "2026-01-15T06:00:00Z",
        "in winter 08:00 = 06:00 UTC"
    );
    let s = cron_next("0 8 * * *", "Asia/Jerusalem", SUMMER)
        .unwrap()
        .unwrap();
    assert_eq!(
        utc(s),
        "2026-07-15T05:00:00Z",
        "in summer 08:00 = 05:00 UTC"
    );
}

#[test]
fn cron_next_is_strictly_after() {
    let at = cron_next("0 8 * * *", "Asia/Jerusalem", WINTER)
        .unwrap()
        .unwrap();
    let next = cron_next("0 8 * * *", "Asia/Jerusalem", at)
        .unwrap()
        .unwrap();
    assert_eq!(
        next - at,
        86_400,
        "exactly a day later, not the same moment"
    );
    let before = cron_next("0 8 * * *", "Asia/Jerusalem", at - 1)
        .unwrap()
        .unwrap();
    assert_eq!(before, at);
}

#[test]
fn cron_dst_boundary_single_run() {
    // Spring 2027: in Israel on 26 March at 02:00 the clocks go to 03:00, 02:30 does not exist that day.
    let spring = locals("30 2 * * *", 1_805_889_600, 5);
    assert_eq!(
        spring,
        [
            "2027-03-25 02:30",
            "2027-03-26 03:00",
            "2027-03-27 02:30",
            "2027-03-28 02:30",
            "2027-03-29 02:30"
        ],
        "a run from the skipped hour goes right after it, once"
    );
    // Autumn 2026: on 25 October at 02:00 the clocks go back to 01:00, 01:30 happens twice — one run.
    let autumn = locals("30 1 * * *", 1_792_756_800, 4);
    assert_eq!(
        autumn,
        [
            "2026-10-24 01:30",
            "2026-10-25 01:30",
            "2026-10-26 01:30",
            "2026-10-27 01:30"
        ]
    );
}

#[test]
fn cron_rejects_bad_expression_and_zone() {
    for bad in ["", "* * * *", "0 0 8 * * *", "61 * * * *", "x y z w v"] {
        assert_eq!(
            cron_next(bad, "Asia/Jerusalem", WINTER),
            Err(CronError::Pattern),
            "{bad:?}"
        );
    }
    assert_eq!(
        cron_next("0 8 * * *", "Mars/Base", WINTER),
        Err(CronError::Zone)
    );
}
