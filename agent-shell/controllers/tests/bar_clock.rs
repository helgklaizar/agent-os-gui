//! S2-03, S2-10: clock `HH:MM:SS`, date, system zone and `Asia/Jerusalem` fallback, TZif parsing.

mod bar_common;

use bar_common::*;
use controllers::Controller;
use controllers::clock_fmt::{format_date, format_time};
use controllers::controllers::bar::{BarController, BarMsg};
use controllers::timezone::{FALLBACK_ZONE, Zone, ZoneSource, resolve};

const MIDNIGHT_EVE: i64 = 1_793_662_200; // 2026-11-02 23:30:00 UTC

#[test]
fn clock_format() {
    assert_eq!(format_time(NOW, 7200), "16:05:09");
    assert_eq!(format_date(NOW, 7200), "Mon, Nov 2");
    assert_eq!(format_time(NOW, 0), "14:05:09");
    // the date also changes after local midnight
    assert_eq!(
        (
            format_time(MIDNIGHT_EVE, 7200),
            format_date(MIDNIGHT_EVE, 7200)
        ),
        ("01:30:00".to_string(), "Tue, Nov 3".to_string())
    );
    assert_eq!(format_date(MIDNIGHT_EVE, 0), "Mon, Nov 2");
    // negative offset and the epoch start
    assert_eq!(
        (format_time(0, -3600), format_date(0, -3600)),
        ("23:00:00".to_string(), "Wed, Dec 31".to_string())
    );
}

#[test]
fn clock_months_and_weekdays_cover_all() {
    let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    for (i, want) in days.iter().enumerate() {
        // 2026-11-01 is a Sunday
        let unix = NOW - 86_400 + (i as i64) * 86_400;
        assert!(
            format_date(unix, 0).starts_with(want),
            "{i}: {}",
            format_date(unix, 0)
        );
    }
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    for (i, want) in months.iter().enumerate() {
        let unix = 1_767_225_600 + (i as i64) * 31 * 86_400 + 5 * 86_400; // from 2026-01-01
        let date = format_date(unix, 0);
        assert!(date.ends_with(want) || date.contains(want), "{i}: {date}");
    }
}

#[test]
fn bar_controller_state_has_time_and_date_deterministic() {
    let mut c = BarController::new(Zone::fixed(7200));
    send(&mut c, BarMsg::Tick(NOW));
    let v = c.view();
    assert_eq!(
        (v.time.as_str(), v.date.as_str()),
        ("16:05:09", "Mon, Nov 2")
    );
    send(&mut c, BarMsg::Tick(NOW + 1));
    assert_eq!(c.view().time, "16:05:10", "seconds tick");
    let mut other = BarController::new(Zone::fixed(7200));
    send(&mut other, BarMsg::Tick(NOW));
    assert_eq!(other.view().time, "16:05:09", "same clock - same value");
}

fn zone_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("zoneinfo directory");
    let root = dir.path();
    std::fs::create_dir_all(root.join("Europe")).expect("Europe");
    std::fs::create_dir_all(root.join("Asia")).expect("Asia");
    std::fs::write(root.join("Europe/Test"), tzif(&[], &[(18_000, false)])).expect("Test file");
    // Jerusalem: winter +2 h, summer +3 h (transitions 2026-03-27 and 2026-10-25)
    let jer = tzif(
        &[(1_774_569_600, 1), (1_792_886_400, 0)],
        &[(7200, false), (10_800, true)],
    );
    std::fs::write(root.join("Asia/Jerusalem"), jer).expect("Jerusalem file");
    std::fs::write(root.join("Europe/Broken"), b"not tzif at all").expect("broken file");
    dir
}

#[test]
fn clock_timezone_system_then_default() {
    let dir = zone_dir();
    let (z, src) = resolve(Some("Europe/Test"), dir.path());
    assert_eq!(
        (src, z.offset_at(NOW)),
        (ZoneSource::System, 18_000),
        "system one is available - use it"
    );
    for missing in [
        None,
        Some("Nowhere/Missing"),
        Some("Europe/Broken"),
        Some(""),
        Some("../Europe/Test"),
        Some("/etc/passwd"),
        Some("Europe/../Europe/Test"),
    ] {
        let (z, src) = resolve(missing, dir.path());
        assert_eq!(src, ZoneSource::Fallback, "{missing:?}");
        assert_eq!(
            z.offset_at(NOW),
            7200,
            "{missing:?}: Asia/Jerusalem, winter"
        );
    }
    assert_eq!(FALLBACK_ZONE, "Asia/Jerusalem");
    let empty = tempfile::tempdir().expect("empty directory");
    let (z, src) = resolve(Some("Europe/Test"), empty.path());
    assert_eq!(
        (src, z.offset_at(NOW)),
        (ZoneSource::Utc, 0),
        "neither a system nor a fallback file - UTC"
    );
}

#[test]
fn tzif_transitions_switch_offset() {
    let dir = zone_dir();
    let (z, _) = resolve(Some("Asia/Jerusalem"), dir.path());
    assert_eq!(z.offset_at(1_774_569_599), 7200);
    assert_eq!(
        z.offset_at(1_774_569_600),
        10_800,
        "already summer time at the transition moment"
    );
    assert_eq!(z.offset_at(1_782_907_200), 10_800, "summer");
    assert_eq!(z.offset_at(1_792_886_399), 10_800);
    assert_eq!(z.offset_at(1_792_886_400), 7200);
    assert_eq!(
        z.offset_at(0),
        7200,
        "before the first transition - standard time"
    );
}

#[test]
fn tzif_rejects_garbage_and_truncated() {
    assert!(Zone::parse_tzif(b"").is_err());
    assert!(Zone::parse_tzif(b"TZif").is_err());
    let good = tzif(&[(100, 0)], &[(3600, false)]);
    assert!(Zone::parse_tzif(&good).is_ok());
    for cut in [10, 30, 44, good.len() - 3] {
        assert!(
            Zone::parse_tzif(&good[..cut]).is_err(),
            "truncated at {cut}"
        );
    }
    let mut bad_index = tzif(&[(100, 5)], &[(3600, false)]);
    bad_index.push(0);
    assert!(
        Zone::parse_tzif(&bad_index).is_err(),
        "type index outside the table"
    );
}

#[test]
fn real_zoneinfo_jerusalem_matches_known_offsets() {
    let (z, src) = resolve(
        Some("Asia/Jerusalem"),
        std::path::Path::new("/usr/share/zoneinfo"),
    );
    assert_eq!(
        src,
        ZoneSource::System,
        "the dev machine must have /usr/share/zoneinfo"
    );
    assert_eq!(z.offset_at(NOW), 7200, "2 Nov 2026: winter time");
    assert_eq!(
        z.offset_at(1_782_907_200),
        10_800,
        "1 Jul 2026: summer time"
    );
}

#[test]
fn clock_timezone_rejects_absolute_name_outside_zoneinfo() {
    // a valid TZif lies outside the zoneinfo directory; an absolute-path name must not lead there (`dir.join(abs)` = abs)
    let outside = tempfile::Builder::new()
        .prefix("zo")
        .tempdir()
        .expect("directory outside zoneinfo");
    std::fs::write(outside.path().join("zz"), tzif(&[], &[(18_000, false)])).expect("file");
    let inside = zone_dir();
    let abs = outside.path().join("zz");
    let name = abs.to_str().expect("path is UTF-8").to_string();
    let (z, src) = resolve(Some(&name), inside.path());
    assert_eq!(
        src,
        ZoneSource::Fallback,
        "absolute path {name} is not a zone"
    );
    assert_eq!(z.offset_at(NOW), 7200);
}
