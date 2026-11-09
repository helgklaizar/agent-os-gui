//! `GET /events`: filters, pagination by `id`, `counts` over the whole selection; `GET /events/subtypes`.

mod chat_common;
mod events_common;

use events_common::*;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn events_list_empty_has_counts() {
    let r = bare().await;
    let v = r.events("").await;
    assert_eq!(v["items"], json!([]));
    assert_eq!(unread_total(&v), 0);
    assert_eq!(v["counts"]["by_kind"], json!({}));
    assert_eq!(v["counts"]["by_sub"], json!({}));
}

#[tokio::test(flavor = "multi_thread")]
async fn events_list_newest_first_and_event_shape() {
    let r = bare().await;
    let s = r.session().await;
    let a = r.add("ai", "agent", "first", Some(s));
    let b = r.add("msg", "sys", "second", None);
    let v = r.events("").await;
    let items = v["items"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        items.iter().map(|e| e["id"].clone()).collect::<Vec<_>>(),
        vec![json!(b), json!(a)],
        "newest first"
    );
    let e = &items[1];
    assert_eq!(e["kind"], "ai");
    assert_eq!(e["sub"], "agent");
    assert_eq!(e["level"], "info");
    assert_eq!(e["text"], "first");
    assert_eq!(e["session_id"], s);
    assert_eq!(e["tab"], "chat", "Event.tab defaults to chat");
    assert_eq!(e["unread"], true);
    assert!(e["ts"].as_i64().unwrap_or(0) > 0);
    assert_eq!(items[0]["session_id"], Value::Null);
    assert!(
        e.get("dedup_key").is_none(),
        "dedup_key is not exposed to the client"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_list_filters_kind_sub_session_unread() {
    let r = bare().await;
    let s = r.session().await;
    let a = r.add("ai", "agent", "a", Some(s));
    let b = r.add("ai", "watch", "b", None);
    let c = r.add("task", "done", "c", Some(s));
    let d = r.add("msg", "wa", "d", None);
    assert_eq!(r.ids("kind=ai").await, vec![b, a]);
    assert_eq!(r.ids("kind=ai&sub=watch").await, vec![b]);
    assert_eq!(r.ids("sub=done").await, vec![c]);
    assert_eq!(r.ids(&format!("session_id={s}")).await, vec![c, a]);
    r.read_all(json!({"ids": [a, d]})).await;
    assert_eq!(r.ids("unread=true").await, vec![c, b]);
    assert_eq!(r.ids("unread=false").await, vec![d, a]);
    assert_eq!(r.ids("kind=msg&sub=tg").await, Vec::<i64>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn events_list_pagination_by_id() {
    let r = bare().await;
    let ids: Vec<i64> = (0..5)
        .map(|i| r.add("ai", "agent", &format!("e{i}"), None))
        .collect();
    assert_eq!(r.ids("limit=2").await, vec![ids[4], ids[3]]);
    assert_eq!(
        r.ids(&format!("limit=2&before={}", ids[3])).await,
        vec![ids[2], ids[1]]
    );
    assert_eq!(
        r.ids(&format!("limit=2&before={}", ids[1])).await,
        vec![ids[0]],
        "the last page is shorter"
    );
    assert_eq!(
        r.ids(&format!("before={}", ids[0])).await,
        Vec::<i64>::new()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_response_counts_cover_whole_selection_not_page() {
    let r = bare().await;
    let s = r.session().await;
    let read = r.add("ai", "agent", "read", Some(s));
    for i in 0..3 {
        r.add("ai", "agent", &format!("a{i}"), Some(s));
    }
    r.add("ai", "watch", "w", None);
    r.add("task", "sched", "t", None);
    r.add("msg", "wa", "m", None);
    r.read_all(json!({"ids": [read]})).await;
    let v = r.events("limit=1").await;
    assert_eq!(
        v["items"].as_array().map(Vec::len),
        Some(1),
        "a page of one"
    );
    assert_eq!(unread_total(&v), 6, "unread: 3 + 1 + 1 + 1");
    assert_eq!(
        v["counts"]["by_kind"],
        json!({"ai": 4, "task": 1, "msg": 1})
    );
    assert_eq!(
        v["counts"]["by_sub"],
        json!({"ai.agent": 3, "ai.watch": 1, "task.sched": 1, "msg.wa": 1})
    );
    let v = r.events("kind=ai&limit=1").await;
    assert_eq!(unread_total(&v), 4, "counters for the kind=ai selection");
    assert_eq!(v["counts"]["by_kind"], json!({"ai": 4}));
}

#[tokio::test(flavor = "multi_thread")]
async fn events_list_rejects_bad_params() {
    let r = bare().await;
    for q in [
        "before=abc",
        "limit=0",
        "limit=201",
        "limit=x",
        "unread=maybe",
        "kind=bogus",
        "session_id=x",
    ] {
        let resp = r.call("GET", &format!("/v1/events?{q}"), None).await;
        assert_eq!(
            resp.status,
            400,
            "{q}: {}",
            String::from_utf8_lossy(&resp.body)
        );
        assert_eq!(code(&resp), "bad_request", "{q}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn events_requires_token_and_read_scope() {
    let r = bare().await;
    let none = send_plain(&r, "GET", "/v1/events").await;
    assert_eq!(none, 401, "without a token");
    let reader = r.token(fake_agentd::Scope::Read, fake_agentd::Kind::External);
    let ok = r.call_as(&reader, "GET", "/v1/events", None).await;
    assert_eq!(ok.status, 200, "read can read the feed");
    let sub = r.call_as(&reader, "GET", "/v1/events/subtypes", None).await;
    assert_eq!(sub.status, 200);
}

async fn send_plain(r: &Rig, method: &str, path: &str) -> u16 {
    send(r.d.socket_path(), Req::new(method, path)).await.status
}

#[tokio::test(flavor = "multi_thread")]
async fn events_subtypes_default_registry_has_eleven() {
    let r = bare().await;
    let v = r.get("/v1/events/subtypes").await;
    let items = v.as_array().cloned().unwrap_or_default();
    let mut pairs: Vec<String> = items
        .iter()
        .map(|s| {
            format!(
                "{}/{}",
                s["kind"].as_str().unwrap_or(""),
                s["sub"].as_str().unwrap_or("")
            )
        })
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        [
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
            "task/sched"
        ]
    );
    for s in &items {
        assert!(s["label"].as_str().is_some_and(|l| !l.is_empty()), "{s}");
        assert!(s["icon"].as_str().is_some_and(|l| !l.is_empty()), "{s}");
        assert_eq!(s["enabled"], true);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn events_subtypes_registry_is_data() {
    let r = bare().await;
    r.d.register_subtype("msg", "signal", "Signal", Some("signal-symbolic"));
    let v = r.get("/v1/events/subtypes").await;
    let items = v.as_array().cloned().unwrap_or_default();
    assert_eq!(
        items.len(),
        12,
        "a subtype is added without changing client code"
    );
    assert!(
        items
            .iter()
            .any(|s| s["sub"] == "signal" && s["label"] == "Signal")
    );
    r.d.register_subtype("msg", "signal", "Signal", None);
    let v = r.get("/v1/events/subtypes").await;
    let items = v.as_array().cloned().unwrap_or_default();
    assert_eq!(
        items.len(),
        12,
        "re-registration replaces, does not duplicate"
    );
    assert!(
        items
            .iter()
            .any(|s| s["label"] == "Signal" && s["icon"].is_null())
    );
}
