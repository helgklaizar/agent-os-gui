//! A4-02: weakening findings, session `cfg.perms`, the `settings` delta and the permission change log.

mod common;

use std::time::Duration;

use common::{Env, fields, mode_of, session};
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn weaken_attempt_logged() {
    let mut e = Env::start().await;
    let s = session(&e, None).await;
    let (st, _) = e
        .admin
        .put(
            &format!("/permissions/session/{s}/send"),
            json!({"mode": "allow"}),
        )
        .await;
    assert_eq!(st, 403);
    let found: Vec<(String, String, String, String, String)> = e
        .inspect(|c| {
            c.prepare("SELECT kind, verdict, ref_table, severity, detail FROM observer_findings")?
                .query_map([], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?
                .collect()
        })
        .await;
    assert_eq!(found.len(), 1, "{found:?}");
    let (kind, verdict, table, _sev, detail) = &found[0];
    assert_eq!(
        (kind.as_str(), verdict.as_str(), table.as_str()),
        ("permission_loosen", "blocked", "permissions")
    );
    let d: Value = serde_json::from_str(detail).unwrap();
    assert_eq!(
        (
            d["level"].as_str(),
            d["action"].as_str(),
            d["scope_id"].as_i64()
        ),
        (Some("session"), Some("send"), Some(s))
    );
    assert!(
        d["by"].as_str().is_some_and(|b| !b.is_empty()),
        "initiator in detail: {d}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_cfg_perms_only_tighten() {
    let e = Env::start().await;
    let s = session(&e, None).await;
    let path = format!("/sessions/{s}");
    let (st, v) = e
        .chat
        .patch(&path, json!({"cfg": {"perms": {"send": "allow"}}}))
        .await;
    assert_eq!(
        (st, v["error"]["code"].as_str()),
        (403, Some("permission_denied")),
        "{v}"
    );
    let (st, v) = e
        .chat
        .patch(&path, json!({"cfg": {"perms": {"send": "deny"}}}))
        .await;
    assert_eq!(
        (st, v["cfg"]["perms"].clone()),
        (200, json!({"send": "deny"})),
        "{v}"
    );
    let (_, rows) = e
        .read
        .get(&format!("/permissions?scope=session&scope_id={s}"))
        .await;
    assert_eq!(
        mode_of(&rows, "session", s, "send").as_deref(),
        Some("deny")
    );
    for bad in [
        json!({"teleport": "deny"}),
        json!({"send": "maybe"}),
        json!({"send": null}),
        json!({"send": 1}),
    ] {
        let (st, v) = e.chat.patch(&path, json!({"cfg": {"perms": bad}})).await;
        assert_eq!(
            (st, fields(&v)),
            (422, vec!["cfg.perms".to_string()]),
            "{bad}: {v}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn permissions_api_publishes_settings_delta_and_logs() {
    let mut e = Env::start().await;
    let mut ws = e.ws();
    ws.subscribe("settings");
    let (st, _) = e
        .admin
        .put("/permissions/global/0/shell", json!({"mode": "ask"}))
        .await;
    assert_eq!(st, 200);
    let frames = ws.until(Duration::from_secs(5), |f| f.data["page"] == "permissions");
    assert!(frames.last().unwrap().data["rev"].as_i64().unwrap() > 0);
    let rows: Vec<(String, String, Option<String>)> = e
        .inspect(|c| {
            c.prepare(
                "SELECT actor, action, target FROM actions_log WHERE action = 'permission_put'",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect()
        })
        .await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        (rows[0].0.as_str(), rows[0].2.as_deref()),
        ("user", Some("global/0/shell"))
    );
}
