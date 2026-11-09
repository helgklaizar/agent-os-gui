//! Session settings in `fake-agentd`: `cfg` (model, notifications, context, memory, autonomous mode) survives
//! a restart; clearing and export; loosening a session permission gives `403 permission_denied` and a `permission_loosen` finding.

mod chat_common;

use chat_common::*;
use fake_agentd::Config;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn session_settings_persist_across_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let first = rig_with(Config::temp().no_ticker().manual_clock().db(db.clone())).await;
    let s = first.session().await;
    let patch = first
        .call(
            "PATCH",
            &format!("/v1/sessions/{s}"),
            Some(json!({"cfg": {"notif": "important", "ctx": "none", "mem": true, "auto": true, "model": 2}, "path": "/work/x"})),
        )
        .await;
    assert_eq!(
        patch.status,
        200,
        "{}",
        String::from_utf8_lossy(&patch.body)
    );
    drop(first);
    let second = rig_with(Config::temp().no_ticker().manual_clock().db(db)).await;
    let got = second.get(&format!("/v1/sessions/{s}")).await;
    assert_eq!(got["cfg"]["notif"], "important");
    assert_eq!(got["cfg"]["ctx"], "none");
    assert_eq!(got["cfg"]["mem"], true);
    assert_eq!(got["cfg"]["auto"], true);
    assert_eq!(got["cfg"]["model"], 2);
    assert_eq!(got["path"], "/work/x");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_cfg_rejects_bad_values() {
    let r = rig().await;
    let s = r.session().await;
    for bad in [
        json!({"cfg": {"notif": "loud"}}),
        json!({"cfg": {"model": "gpt"}}),
        json!({"cfg": {"ctx": "all"}}),
    ] {
        let resp = r
            .call("PATCH", &format!("/v1/sessions/{s}"), Some(bad.clone()))
            .await;
        assert_eq!(resp.status, 422, "{bad}");
    }
    let ok = r
        .call(
            "PATCH",
            &format!("/v1/sessions/{s}"),
            Some(json!({"cfg": {"model": null, "notif": "off"}})),
        )
        .await;
    assert_eq!(ok.status, 200, "model: null - \"same as the persona\"");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_clear_and_export() {
    let r = rig().await;
    let s = r.session().await;
    r.say(s, "hello", UUID_A).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!r.messages(s).await.is_empty());
    let export = r
        .call("POST", &format!("/v1/sessions/{s}/export"), None)
        .await;
    assert_eq!(
        export.status,
        200,
        "{}",
        String::from_utf8_lossy(&export.body)
    );
    let v = export.json();
    assert!(
        v["path"]
            .as_str()
            .is_some_and(|p| p.starts_with("~/.agentos/export/")),
        "{v}"
    );
    assert!(v["size"].as_i64().unwrap_or(0) > 0);
    let clear = r
        .call("POST", &format!("/v1/sessions/{s}/clear"), None)
        .await;
    assert_eq!(clear.status, 204);
    assert!(r.messages(s).await.is_empty(), "history cleared");
    let ghost = r.call("POST", "/v1/sessions/9999/clear", None).await;
    assert_eq!(ghost.status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn session_perm_loosen_is_403_with_finding() {
    let r = rig().await;
    let s = r.session().await;
    assert_eq!(r.put_rule("global", 0, "shell", "deny").await.status, 200);
    let loosen = r.put_rule("session", s, "shell", "allow").await;
    assert_eq!(loosen.status, 403);
    assert_eq!(code(&loosen), "permission_denied");
    let findings =
        r.log_rows("SELECT kind FROM observer_findings WHERE kind = 'permission_loosen'");
    assert_eq!(findings.len(), 1, "finding written");
    let tighten = r.put_rule("session", s, "send", "deny").await;
    assert_eq!(tighten.status, 200, "tightening is allowed");
}
