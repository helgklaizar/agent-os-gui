//! Events of all 11 subtypes: fixtures, the "all subtypes" scenario, D-Bus emulation, agent reply, unknown `sub`.

mod chat_common;
mod events_common;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use events_common::*;
use fake_agentd::{Config, Scenario};
use serde_json::Value;

const ELEVEN: [&str; 11] = [
    "ai/agent",
    "ai/claude",
    "ai/local",
    "ai/watch",
    "msg/mail",
    "msg/sys",
    "msg/tg",
    "msg/wa",
    "task/deleg",
    "task/done",
    "task/sched",
];

fn pairs(v: &Value) -> BTreeSet<String> {
    v["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|e| {
            format!(
                "{}/{}",
                e["kind"].as_str().unwrap_or(""),
                e["sub"].as_str().unwrap_or("")
            )
        })
        .collect()
}

fn scenario_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scenarios")
        .join(name)
}

fn eleven() -> BTreeSet<String> {
    ELEVEN.iter().map(|s| (*s).to_string()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn events_fixtures_cover_all_eleven_subtypes() {
    let r = rig().await;
    let v = r.events("limit=200").await;
    assert_eq!(
        pairs(&v),
        eleven(),
        "the fixtures have each of the 11 subtypes"
    );
    let by_sub = v["counts"]["by_sub"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    assert_eq!(by_sub.len(), 11, "a counter for every subtype: {by_sub:?}");
    let items = v["items"].as_array().cloned().unwrap_or_default();
    assert!(
        items.iter().any(|e| e["session_id"].is_null()),
        "there are system events"
    );
    assert!(
        items.iter().any(|e| e["session_id"].is_i64()),
        "there are session events"
    );
    assert!(
        items.iter().any(|e| e["unread"] == false),
        "there are read ones too"
    );
    let levels: BTreeSet<&str> = items.iter().filter_map(|e| e["level"].as_str()).collect();
    assert!(levels.len() >= 3, "levels differ: {levels:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_fixtures_refer_to_existing_sessions_and_are_not_repeated() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let first = rig_with(Config::temp().no_ticker().manual_clock().db(db.clone())).await;
    let n1 = first.events("limit=200").await["items"]
        .as_array()
        .map_or(0, Vec::len);
    let sessions: Vec<i64> = first
        .get("/v1/sessions")
        .await
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s["id"].as_i64())
        .collect();
    for e in first.events("limit=200").await["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        if let Some(sid) = e["session_id"].as_i64() {
            assert!(
                sessions.contains(&sid),
                "the event refers to session {sid}, which does not exist"
            );
        }
    }
    drop(first);
    let second = rig_with(Config::temp().no_ticker().manual_clock().db(db)).await;
    let n2 = second.events("limit=200").await["items"]
        .as_array()
        .map_or(0, Vec::len);
    assert!(n1 >= 11);
    assert_eq!(n1, n2, "a repeated fixture run does not multiply");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_scenario_all_subtypes_gives_exactly_eleven_pairs() {
    let sc = Scenario::from_file(&scenario_path("all-subtypes.toml")).expect("scenario parses");
    let r = rig_with(
        Config::temp()
            .no_ticker()
            .manual_clock()
            .no_event_fixtures()
            .scenario(sc),
    )
    .await;
    let w = watch(&r.d, &r.admin, "events").await;
    assert!(
        w.until(5, |w| (!w.snapshot().is_null()).then_some(()))
            .await
            .is_some()
    );
    for _ in 0..40 {
        r.d.advance(Duration::from_secs(1));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let got = w
        .until(5, |w| {
            let n = w.all().iter().filter(|f| f["type"] == "event").count();
            (n >= 11).then_some(n)
        })
        .await;
    assert_eq!(got, Some(11), "one frame per event: {:?}", w.all().len());
    let v = r.events("limit=200").await;
    assert_eq!(pairs(&v), eleven());
    assert_eq!(
        v["items"].as_array().map(Vec::len),
        Some(11),
        "exactly 11 events"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_scenario_event_step_rejects_bad_input() {
    for step in [
        r#"event = { kind = "bogus", sub = "agent", text = "x" }"#,
        r#"event = { kind = "ai", sub = "agent" }"#,
        r#"event = { kind = "ai", sub = "agent", text = "x", level = "fatal" }"#,
        r#"event = { kind = "ai", sub = "agent", text = "x" }
action = "drop_ws""#,
    ] {
        let text = format!("name = \"x\"\n[[step]]\nat_ms = 0\n{step}\n");
        assert!(Scenario::from_toml(&text).is_err(), "{step}");
    }
    let ok = "name = \"x\"\n[[step]]\nat_ms = 0\nevent = { kind = \"msg\", sub = \"wa\", text = \"x\" }\n";
    assert!(Scenario::from_toml(ok).is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn events_unknown_sub_is_listed_and_counted() {
    let r = bare().await;
    let id = r.add("msg", "signal", "from a new provider", None);
    let v = r.events("").await;
    assert_eq!(v["items"][0]["id"], id);
    assert_eq!(v["items"][0]["sub"], "signal");
    assert_eq!(v["counts"]["by_sub"]["msg.signal"], 1);
    assert_eq!(r.ids("sub=signal").await, vec![id]);
    let known: Vec<String> = r
        .get("/v1/events/subtypes")
        .await
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s["sub"].as_str().map(str::to_string))
        .collect();
    assert!(
        !known.contains(&"signal".to_string()),
        "the subtype is not in the registry, the event lives anyway"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_dbus_bridge_is_msg_sys() {
    let r = bare().await;
    let w = watch(&r.d, &r.admin, "events").await;
    assert!(
        w.until(5, |w| (!w.snapshot().is_null()).then_some(()))
            .await
            .is_some()
    );
    let id =
        r.d.dbus_notification("Battery low")
            .expect("write")
            .expect("event");
    let e = r.events("").await["items"][0].clone();
    assert_eq!(e["id"], id);
    assert_eq!(
        (e["kind"].clone(), e["sub"].clone()),
        ("msg".into(), "sys".into())
    );
    assert_eq!(e["source"], "agent-sys");
    assert_eq!(e["text"], "Battery low");
    assert_eq!(e["session_id"], Value::Null);
    assert!(
        w.until(5, |w| w.all().into_iter().find(|f| f["type"] == "event"))
            .await
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_agent_reply_creates_event_and_session_unread() {
    let r = bare().await;
    let s = r.session().await;
    let before = r.get(&format!("/v1/sessions/{s}")).await["unread"]
        .as_i64()
        .unwrap_or(-1);
    assert_eq!(r.say(s, "hello", UUID_A).await.status, 201);
    let ev = loop_until(&r, s).await;
    assert_eq!(
        (ev["kind"].clone(), ev["sub"].clone()),
        ("ai".into(), "agent".into())
    );
    assert_eq!(ev["session_id"], s);
    assert_eq!(ev["unread"], true);
    let after = r.get(&format!("/v1/sessions/{s}")).await["unread"]
        .as_i64()
        .unwrap_or(-1);
    assert_eq!(after, before + 1, "the session badge grew by one reply");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_stopped_reply_makes_no_event() {
    let r = bare().await;
    let s = r.session().await;
    r.say(s, "/slow long", UUID_A).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let stop = r
        .call("POST", &format!("/v1/sessions/{s}/stop"), None)
        .await;
    assert!(stop.status < 300, "{}", stop.status);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        r.ids(&format!("session_id={s}")).await.len(),
        0,
        "a stopped reply is not an event"
    );
}

async fn loop_until(r: &Rig, s: i64) -> Value {
    for _ in 0..300 {
        let v = r.events(&format!("session_id={s}")).await;
        if let Some(e) = v["items"].as_array().and_then(|a| a.first()) {
            return e.clone();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("agent reply event did not appear");
}
