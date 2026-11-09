//! A6-11: JSON config exports (`config.json`, `connections.json`, `personas.json`, `modules.json`, `tunnels.json`):
//! the core regenerates them atomically on change (`0600`), without secrets, and never reads them back.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use common::Env;
use serde_json::{Value, json};

const SECRET: &str = "sk-ant-TEST-value-not-for-files";

fn path(e: &Env, name: &str) -> PathBuf {
    e.home.join(".agentos").join(name)
}

async fn wait_json(e: &Env, name: &str, ok: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..200 {
        if let Ok(t) = fs::read_to_string(path(e, name))
            && let Ok(v) = serde_json::from_str::<Value>(&t)
            && ok(&v)
        {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{name} did not appear or was not updated");
}

#[tokio::test(flavor = "multi_thread")]
async fn json_export_regenerates_on_change_without_secrets() {
    let e = Env::start().await;
    let (st, v) = e
        .admin
        .put("/secrets/provider.anthropic", json!({"value": SECRET}))
        .await;
    assert!(st == 200 || st == 201 || st == 204, "{st} {v}");
    let p = e
        .admin
        .make("/personas", json!({"name": "Asya", "color": "#abc"}))
        .await;
    assert_eq!(
        e.admin
            .post(
                "/tunnels",
                json!({"type": "ssh", "addr": "host.example.test"})
            )
            .await
            .0,
        201
    );
    let personas = wait_json(&e, "personas.json", |v| {
        v.as_array().is_some_and(|a| a.len() == 1)
    })
    .await;
    assert_eq!(personas[0]["name"], "Asya");
    assert_eq!(personas[0]["id"], p["id"]);
    let tunnels = wait_json(&e, "tunnels.json", |v| {
        v.as_array().is_some_and(|a| a.len() == 1)
    })
    .await;
    assert_eq!(tunnels[0]["addr"], "host.example.test");
    for n in ["config.json", "connections.json", "modules.json"] {
        wait_json(&e, n, |_| true).await;
    }
    for n in [
        "config.json",
        "connections.json",
        "personas.json",
        "modules.json",
        "tunnels.json",
    ] {
        let md = fs::metadata(path(&e, n)).unwrap();
        assert_eq!(md.permissions().mode() & 0o777, 0o600, "{n}");
        assert!(
            !fs::read_to_string(path(&e, n)).unwrap().contains(SECRET),
            "{n}: secret in the export"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn json_export_is_never_read_back() {
    let e = Env::start().await;
    e.admin
        .make("/personas", json!({"name": "First", "color": "#abc"}))
        .await;
    wait_json(&e, "personas.json", |v| {
        v.as_array().is_some_and(|a| a.len() == 1)
    })
    .await;
    // Editing the file by hand does not affect core state.
    fs::write(
        path(&e, "personas.json"),
        r#"[{"id": 99, "name": "Forgery"}]"#,
    )
    .unwrap();
    let (_, list) = e.read.get("/personas").await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["name"], "First");
    // The next change rewrites the file from DB data, the forgery disappears.
    e.admin
        .make("/personas", json!({"name": "Second", "color": "#abc"}))
        .await;
    let v = wait_json(&e, "personas.json", |v| {
        v.as_array().is_some_and(|a| a.len() == 2)
    })
    .await;
    assert!(
        v.as_array().unwrap().iter().all(|p| p["name"] != "Forgery"),
        "{v}"
    );
}
