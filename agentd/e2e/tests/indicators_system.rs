//! A7-07: indicators for network (`net`), battery (`battery`), VPN (`vpn`) and backup age (`backup`). Sources read sysfs
//! (in the test build the root is set by `AGENTD_SYSFS_ROOT`) and the `~/.agentos/export` directory. No sensor means `nodata`, not `ok`;
//! a state transition produces exactly one event. Thresholds: battery <= 30 % / <= 15 %, backup
//! > 26 h / > 50 h, no link is red.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::Env;
use serde_json::{Value, json};

fn root(e: &Env) -> PathBuf {
    e.home.join("sysroot")
}

fn put(base: &Path, rel: &str, text: &str) {
    let p = base.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

async fn enable(e: &Env, ids: &[&str]) {
    let (_, page) = e.read.get("/settings/indicators").await;
    let rev = page["rev"].as_i64().unwrap_or(0);
    let items: Vec<Value> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| json!({"id": id, "enabled": true, "position": i}))
        .collect();
    let (st, v) = e
        .admin
        .patch_if_match("/settings/indicators", json!({"items": items}), rev)
        .await;
    assert_eq!(st, 200, "{v}");
}

async fn state(e: &Env, id: &str) -> Option<String> {
    let mut ws = e.ws();
    let snap = ws.subscribe("bar.indicators");
    snap["items"]
        .as_array()?
        .iter()
        .find(|i| i["id"] == id)
        .and_then(|i| i["state"].as_str().map(str::to_string))
}

/// Waits for the state for up to 30 s (indicators are polled every 5 s).
async fn wait_state(e: &Env, id: &str, want: &str) {
    let mut last = None;
    for _ in 0..60 {
        last = state(e, id).await;
        if last.as_deref() == Some(want) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("{id}: expected {want}, last state {last:?}");
}

async fn events_of(e: &Env, id: &str) -> Vec<String> {
    let (st, v) = e.read.get("/events?kind=msg&sub=sys").await;
    assert_eq!(st, 200, "{v}");
    let prefix = format!("{id}:");
    let mut out: Vec<String> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["text"].as_str().is_some_and(|t| t.starts_with(&prefix)))
        .map(|i| i["level"].as_str().unwrap().to_string())
        .collect();
    out.reverse();
    out
}

fn age(path: &Path, secs: u64) {
    let t = SystemTime::now() - Duration::from_secs(secs);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_source_nodata_when_missing() {
    let e = Env::start().await;
    enable(&e, &["net", "battery", "vpn", "backup"]).await;
    for id in ["net", "battery", "vpn", "backup"] {
        wait_state(&e, id, "nodata").await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_source_net_link_one_event_per_transition() {
    let e = Env::start().await;
    let r = root(&e);
    put(&r, "sys/class/net/lo/operstate", "unknown\n");
    put(&r, "sys/class/net/eth0/operstate", "up\n");
    enable(&e, &["net"]).await;
    wait_state(&e, "net", "ok").await;
    put(&r, "sys/class/net/eth0/operstate", "down\n");
    wait_state(&e, "net", "bad").await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    // Transitions: no data -> normal (`ok`), normal -> no link (`error`); repeated event frames add nothing.
    assert_eq!(
        events_of(&e, "net").await,
        ["ok", "error"],
        "exactly one event per transition"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_source_battery_thresholds() {
    let e = Env::start().await;
    let r = root(&e);
    put(&r, "sys/class/power_supply/BAT0/capacity", "80\n");
    put(&r, "sys/class/power_supply/BAT0/status", "Discharging\n");
    enable(&e, &["battery"]).await;
    wait_state(&e, "battery", "ok").await;
    put(&r, "sys/class/power_supply/BAT0/capacity", "30\n");
    wait_state(&e, "battery", "warn").await;
    put(&r, "sys/class/power_supply/BAT0/capacity", "15\n");
    wait_state(&e, "battery", "bad").await;
    // Charging: do not alarm, even at low charge.
    put(&r, "sys/class/power_supply/BAT0/status", "Charging\n");
    wait_state(&e, "battery", "ok").await;
    assert_eq!(events_of(&e, "battery").await, ["warn", "error", "ok"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_source_vpn_interface() {
    let e = Env::start().await;
    let r = root(&e);
    put(&r, "sys/class/net/eth0/operstate", "up\n");
    enable(&e, &["vpn"]).await;
    wait_state(&e, "vpn", "nodata").await;
    // WireGuard reports `unknown` when the interface is up.
    put(&r, "sys/class/net/wg0/operstate", "unknown\n");
    wait_state(&e, "vpn", "ok").await;
    put(&r, "sys/class/net/wg0/operstate", "down\n");
    wait_state(&e, "vpn", "warn").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn indicator_source_backup_age() {
    let e = Env::start().await;
    let dir = e.home.join(".agentos/export");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("backup-1.tar.age");
    std::fs::write(&file, b"x").unwrap();
    age(&file, 3 * 24 * 3600);
    enable(&e, &["backup"]).await;
    wait_state(&e, "backup", "bad").await;
    age(&file, 30 * 3600);
    wait_state(&e, "backup", "warn").await;
    age(&file, 3600);
    wait_state(&e, "backup", "ok").await;
    // The first observation (`bad`) is also an event; then yellow and back to normal.
    assert_eq!(events_of(&e, "backup").await, ["error", "warn", "ok"]);
}

fn now_secs() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

async fn set_restore_test(e: &Env, at: Value) {
    let (_, page) = e.read.get("/settings/backup").await;
    let rev = page["rev"].as_i64().unwrap_or(0);
    let (st, v) = e
        .admin
        .patch_if_match("/settings/backup", json!({"last_restore_test_at": at}), rev)
        .await;
    assert_eq!(st, 200, "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_test_red_after_35_days() {
    // Fresh archive + last restore test mark (`backup.last_restore_test_at`): older than 35 days is red.
    let e = Env::start().await;
    let dir = e.home.join(".agentos/export");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("backup-1.tar.age");
    std::fs::write(&file, b"x").unwrap();
    age(&file, 3600);
    enable(&e, &["backup"]).await;
    wait_state(&e, "backup", "ok").await;
    set_restore_test(&e, json!(now_secs() - 36 * 24 * 3600)).await;
    wait_state(&e, "backup", "bad").await;
    set_restore_test(&e, json!(now_secs() - 34 * 24 * 3600)).await;
    wait_state(&e, "backup", "ok").await;
    // Not a number: validation rejects it, the mark does not change.
    let (_, page) = e.read.get("/settings/backup").await;
    let (st, _) = e
        .admin
        .patch_if_match(
            "/settings/backup",
            json!({"last_restore_test_at": "yesterday"}),
            page["rev"].as_i64().unwrap(),
        )
        .await;
    assert_eq!(st, 422);
}
