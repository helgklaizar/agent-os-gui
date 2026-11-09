//! A7-11: compatibility. `/health` reports `capabilities` for the enabled features: `if-match` and `ws-seq` are always present,
//! `tool-results` while the browser command bridge is running, `terminal` while the agent runner accepts connections, `observer-model`
//! when the model observer is enabled. The `about` page shows the same list. An incompatible `api.major` on WS is closed
//! with code `4426` (test `ws_unsupported_api_major_4426` in `agentd/api/tests/ws.rs`).

mod common;

use std::time::Duration;

use common::Env;

async fn caps(e: &Env) -> Vec<String> {
    let (st, h) = e.read.get("/health").await;
    assert_eq!(st, 200);
    h["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect()
}

async fn wait_caps(e: &Env, name: &str, present: bool) {
    for _ in 0..60 {
        if caps(e).await.iter().any(|c| c == name) == present {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!(
        "capabilities: expected {name} present={present}, got {:?}",
        caps(e).await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn compat_capabilities_baseline() {
    let e = Env::start().await;
    let c = caps(&e).await;
    for want in ["if-match", "ws-seq", "tool-results"] {
        assert!(c.iter().any(|x| x == want), "{want} in {c:?}");
    }
    for absent in ["terminal", "observer-model"] {
        assert!(
            !c.iter().any(|x| x == absent),
            "{absent} is not enabled: {c:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn compat_capabilities_follow_runner() {
    let e = Env::start().await;
    let project = e.home.join("project");
    std::fs::create_dir_all(&project).unwrap();
    {
        let _runner = e.start_runner(&[&project]);
        wait_caps(&e, "terminal", true).await;
    }
    wait_caps(&e, "terminal", false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn compat_capabilities_watch() {
    let e = Env::start_watch(60).await;
    assert!(caps(&e).await.iter().any(|c| c == "observer-model"));
}

#[tokio::test(flavor = "multi_thread")]
async fn compat_about_page_lists_same_capabilities() {
    let e = Env::start().await;
    let (st, about) = e.read.get("/settings/about").await;
    assert_eq!(st, 200, "{about}");
    let mut page: Vec<String> = about["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    let mut health = caps(&e).await;
    page.sort();
    health.sort();
    assert_eq!(page, health);
}

#[tokio::test(flavor = "multi_thread")]
async fn compat_health_declares_gui_range() {
    let e = Env::start().await;
    let (_, h) = e.read.get("/health").await;
    assert_eq!(h["api"]["major"], 1);
    for k in ["min_client", "min_gui", "max_gui"] {
        assert!(h[k].as_str().is_some_and(|s| !s.is_empty()), "{k}: {h}");
    }
}
