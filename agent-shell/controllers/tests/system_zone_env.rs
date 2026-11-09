//! S2-mutants: machine time zone name from `TZ`. The test runs its own binary with `TZ` in the child's environment (no `unsafe`).

use controllers::timezone::{ZoneSource, system_zone};

const ZONE_FILE: &str = "/usr/share/zoneinfo/Asia/Tokyo";

#[test]
fn child_reads_tz() {
    if std::env::var_os("S2_TZ_CHILD").is_none() {
        return;
    }
    let (zone, src) = system_zone();
    assert_eq!(src, ZoneSource::System);
    assert_eq!(zone.offset_at(1_793_628_309), 32_400);
}

#[test]
fn system_zone_follows_tz_env() {
    if !std::path::Path::new(ZONE_FILE).is_file() {
        eprintln!("no {ZONE_FILE}: skipped");
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("test path"))
        .args(["--exact", "child_reads_tz", "--nocapture"])
        .env("S2_TZ_CHILD", "1")
        .env("TZ", ":Asia/Tokyo")
        .output()
        .expect("child launch");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("1 passed"));
}
