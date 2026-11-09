//! Bar clock and date: `HH:MM:SS` and "Mon, Nov 2" (order and names come from the current language resources).

use crate::i18n::{tr, tr_args};

const DAY: i64 = 86_400;

/// Days since 1970-01-01 to month (1–12) and day (Howard Hinnant's algorithm; the clock does not need the year).
fn civil_from_days(days: i64) -> (u32, u32) {
    let z = days + 719_468;
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (month, day)
}

/// `HH:MM:SS` for the `unix` moment in the zone with offset `offset_secs`.
pub fn format_time(unix: i64, offset_secs: i32) -> String {
    let secs = (unix + i64::from(offset_secs)).rem_euclid(DAY);
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

fn weekday(index: i64) -> String {
    match index {
        0 => tr("weekday-sun"),
        1 => tr("weekday-mon"),
        2 => tr("weekday-tue"),
        3 => tr("weekday-wed"),
        4 => tr("weekday-thu"),
        5 => tr("weekday-fri"),
        _ => tr("weekday-sat"),
    }
}

fn month(number: u32) -> String {
    match number {
        1 => tr("month-jan"),
        2 => tr("month-feb"),
        3 => tr("month-mar"),
        4 => tr("month-apr"),
        5 => tr("month-may"),
        6 => tr("month-jun"),
        7 => tr("month-jul"),
        8 => tr("month-aug"),
        9 => tr("month-sep"),
        10 => tr("month-oct"),
        11 => tr("month-nov"),
        _ => tr("month-dec"),
    }
}

/// "Mon, Nov 2"
pub fn format_date(unix: i64, offset_secs: i32) -> String {
    let days = (unix + i64::from(offset_secs)).div_euclid(DAY);
    let (m, d) = civil_from_days(days);
    // 1970-01-01 is a Thursday
    tr_args(
        "date-format",
        &[
            ("weekday", &weekday((days + 4).rem_euclid(7))),
            ("day", &d.to_string()),
            ("month", &month(m)),
        ],
    )
}
