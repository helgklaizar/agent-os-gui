//! A6-06: tunnel processes: `ssh` and `wg-quick` run as child processes with a clean environment, status `up|down|error`,
//! restart with increasing backoff, events and a log. Commands are replaced by `tunnel-fake` (`test-support` build only).

mod common;

use std::time::{Duration, Instant};

use common::Env;
use serde_json::{Value, json};

fn log(e: &Env) -> Vec<String> {
    std::fs::read_to_string(e.home.join("tunnel.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
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
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end {
        if status_of(e, id).await == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!(
        "status {want} was not reached, now {}",
        status_of(e, id).await
    );
}

async fn create(e: &Env, t: &str, addr: &str, enabled: bool) -> i64 {
    e.admin
        .make(
            "/tunnels",
            json!({"type": t, "addr": addr, "enabled": enabled}),
        )
        .await["id"]
        .as_i64()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn tunnel_proc_ssh_starts_clean_and_stops() {
    let mut e = Env::start().await;
    let id = create(&e, "ssh", "host.example.test", false).await;
    assert_eq!(status_of(&e, id).await, "down");
    assert!(log(&e).is_empty(), "a disabled tunnel is not started");
    assert_eq!(
        e.admin
            .patch(&format!("/tunnels/{id}"), json!({"enabled": true}))
            .await
            .0,
        200
    );
    wait_status(&e, id, "up").await;
    let lines = log(&e);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("start -N -o ExitOnForwardFailure=yes -o BatchMode=yes"),
        "{}",
        lines[0]
    );
    assert!(lines[0].contains("host.example.test"));
    let env_names = lines[0].split("| env ").nth(1).unwrap();
    assert!(
        !env_names.contains("HOME")
            && !env_names.contains("AGENTD")
            && !env_names.contains("TOKEN"),
        "process environment: {env_names}"
    );
    assert!(env_names.contains("PATH"));
    assert_eq!(
        e.admin
            .patch(&format!("/tunnels/{id}"), json!({"enabled": false}))
            .await
            .0,
        200
    );
    wait_status(&e, id, "down").await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(log(&e).len(), 1, "no restarts after disabling");
    let acts: Vec<String> = e
        .inspect(|c| {
            c.prepare("SELECT action || ':' || actor FROM actions_log WHERE action LIKE 'tunnel_%' ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await;
    assert_eq!(acts, ["tunnel_start:user", "tunnel_stop:user"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn tunnel_proc_restarts_with_growing_pause_and_reports_error() {
    let e = Env::start().await;
    let id = create(&e, "ssh", "crash.example.test", true).await;
    wait_status(&e, id, "error").await;
    let end = Instant::now() + Duration::from_secs(12);
    while log(&e).len() < 3 && Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(log(&e).len() >= 3, "restarts are happening: {:?}", log(&e));
    let (_, ev) = e.read.get("/events?kind=msg&sub=sys").await;
    assert!(
        ev["items"].as_array().unwrap().iter().any(|x| x["text"]
            .as_str()
            .unwrap()
            .contains("Tunnel")
            && x["level"] == "warn"),
        "status change is an event: {ev}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tunnel_proc_wireguard_up_and_down_and_external_types_untouched() {
    let e = Env::start().await;
    let wg = create(&e, "wireguard", "wg0", true).await;
    wait_status(&e, wg, "up").await;
    assert!(
        log(&e).iter().any(|l| l.starts_with("start up wg0 |")),
        "{:?}",
        log(&e)
    );
    e.admin
        .patch(&format!("/tunnels/{wg}"), json!({"enabled": false}))
        .await;
    wait_status(&e, wg, "down").await;
    let end = Instant::now() + Duration::from_secs(5);
    while !log(&e).iter().any(|l| l.starts_with("start down wg0 |")) && Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        log(&e).iter().any(|l| l.starts_with("start down wg0 |")),
        "{:?}",
        log(&e)
    );
    let before = log(&e).len();
    let ts = create(&e, "tailscale", "tailnet.example.test", true).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(status_of(&e, ts).await, "external");
    assert_eq!(log(&e).len(), before, "the core does not start tailscale");
    let v: Value = e.admin.get("/tunnels").await.1;
    assert_eq!(v.as_array().unwrap().len(), 2);
}
