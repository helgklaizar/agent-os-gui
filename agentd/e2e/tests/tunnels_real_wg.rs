//! A6-06 on a real `wg-quick`: the core brings a real WireGuard interface up and down. `wg-quick`
//! needs root while the core runs as a user, so the wrapper (`AGENTD_TUNNEL_WG`, `test-support` build only) calls
//! `sudo -n wg-quick` and substitutes the config path in a temp dir (the core validates the interface name itself).
//! Linux only; needs `wg`, `wg-quick`, `ip` and passwordless `sudo -n` (the `dev-vm` VM). The interface is always brought down at the end.
#![cfg(target_os = "linux")]

mod common;

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::Env;
use serde_json::json;

/// Each test uses its own interface name: tests run in parallel.
const IFACE: &str = "wgc100t";
const IFACE_BAD: &str = "wgc100e";

fn run(cmd: &str, args: &[&str]) -> (bool, String) {
    let o = Command::new(cmd).args(args).output().unwrap_or_else(|e| {
        panic!("{cmd}: {e}");
    });
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
    )
}

fn iface_exists(name: &str) -> bool {
    Command::new("ip")
        .args(["link", "show", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Brings the interface down when the test exits, including on panic.
struct Cleanup(String, &'static str);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if iface_exists(self.1) {
            drop(
                Command::new("sudo")
                    .args(["-n", "wg-quick", "down", &self.0])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status(),
            );
        }
    }
}

fn write_conf(dir: &Path, iface: &str) -> String {
    let (ok, key) = run("wg", &["genkey"]);
    assert!(ok, "wg genkey");
    let path = dir.join(format!("{iface}.conf"));
    std::fs::write(
        &path,
        format!(
            "[Interface]\nPrivateKey = {}\nAddress = 10.99.99.1/32\nListenPort = 51999\n",
            key.trim()
        ),
    )
    .unwrap();
    Command::new("chmod")
        .arg("600")
        .arg(&path)
        .status()
        .unwrap();
    path.to_string_lossy().into_owned()
}

fn wrapper(dir: &Path, conf: &str) -> String {
    let p = dir.join("wg-real.sh");
    // The core calls `wg-quick up|down <interface>`; the interface is replaced by the config path.
    std::fs::write(
        &p,
        format!("#!/bin/sh\nexec sudo -n /usr/bin/wg-quick \"$1\" {conf}\n"),
    )
    .unwrap();
    Command::new("chmod").arg("755").arg(&p).status().unwrap();
    p.to_string_lossy().into_owned()
}

async fn status_of(e: &Env, id: i64) -> String {
    let (_, list) = e.admin.get("/tunnels").await;
    list.as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn wait_status(e: &Env, id: i64, want: &str) {
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        if status_of(e, id).await == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!(
        "status {want} was not reached, now {}",
        status_of(e, id).await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn real_wg_quick_brings_interface_up_and_down() {
    assert!(
        run("sudo", &["-n", "true"]).0,
        "passwordless sudo -n required (dev-vm VM)"
    );
    assert!(!iface_exists(IFACE), "interface {IFACE} already exists");
    let dir = tempfile::tempdir().unwrap();
    let conf = write_conf(dir.path(), IFACE);
    let _cleanup = Cleanup(conf.clone(), IFACE);
    let e = Env::start_with_env(vec![(
        "AGENTD_TUNNEL_WG".into(),
        wrapper(dir.path(), &conf),
    )])
    .await;
    let id = e
        .admin
        .make(
            "/tunnels",
            json!({"type": "wireguard", "addr": IFACE, "enabled": true}),
        )
        .await["id"]
        .as_i64()
        .unwrap();
    wait_status(&e, id, "up").await;
    assert!(iface_exists(IFACE), "the real interface is up");
    let (_, wg) = run("sudo", &["-n", "wg", "show", IFACE]);
    assert!(wg.contains("listening port: 51999"), "{wg}");
    assert_eq!(
        e.admin
            .patch(&format!("/tunnels/{id}"), json!({"enabled": false}))
            .await
            .0,
        200
    );
    wait_status(&e, id, "down").await;
    assert!(!iface_exists(IFACE), "interface is down");
}

#[tokio::test(flavor = "multi_thread")]
async fn real_wg_quick_failure_is_reported_as_error() {
    assert!(
        run("sudo", &["-n", "true"]).0,
        "passwordless sudo -n required"
    );
    let dir = tempfile::tempdir().unwrap();
    // No config: `wg-quick` exits with an error, the interface does not appear.
    let missing = dir.path().join(format!("{IFACE_BAD}.conf"));
    let _cleanup = Cleanup(missing.to_string_lossy().into_owned(), IFACE_BAD);
    let e = Env::start_with_env(vec![(
        "AGENTD_TUNNEL_WG".into(),
        wrapper(dir.path(), &missing.to_string_lossy()),
    )])
    .await;
    let id = e
        .admin
        .make(
            "/tunnels",
            json!({"type": "wireguard", "addr": IFACE_BAD, "enabled": true}),
        )
        .await["id"]
        .as_i64()
        .unwrap();
    wait_status(&e, id, "error").await;
    assert!(!iface_exists(IFACE_BAD));
}
