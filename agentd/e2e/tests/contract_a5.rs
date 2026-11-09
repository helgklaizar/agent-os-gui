//! `contract_a5_routes`: A5 route responses against the `openapi-draft.yaml` schemas.

mod common;
mod openapi;

use common::Env;
use openapi::{check, declared, schemas};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn contract_a5_routes() {
    let all = schemas();
    for name in [
        "Event",
        "EventCounts",
        "EventSubtype",
        "ConnectFlow",
        "Indicator",
        "IndicatorsPage",
    ] {
        assert!(!all[name].props.is_empty(), "schema {name} was not parsed");
    }
    let e = Env::start_with(|c| {
        c.execute(
            "INSERT INTO events(kind, sub, level, text, unread, ts) VALUES ('msg','sys','warn','t',1,5)",
            [],
        )?;
        Ok(())
    })
    .await;
    let mut seen: Vec<(&str, &str, u16)> = Vec::new();
    let mut rec = |m: &'static str, p: &'static str, st: u16| seen.push((m, p, st));

    let (st, v) = e.read.get("/events").await;
    rec("get", "/events", st);
    assert_eq!(st, 200);
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .for_each(|i| check("Event", i, &all));
    check("EventCounts", &v["counts"], &all);
    let (st, v) = e.read.get("/events/subtypes").await;
    rec("get", "/events/subtypes", st);
    v.as_array()
        .unwrap()
        .iter()
        .for_each(|i| check("EventSubtype", i, &all));
    let (st, v) = e.chat.post("/events/read", json!({"kind": "msg"})).await;
    rec("post", "/events/read", st);
    assert_eq!((st, &v["marked"]), (200, &json!(1)));
    let (st, _) = e.chat.post("/events/1/read", json!({})).await;
    rec("post", "/events/{id}/read", st);
    assert_eq!(st, 204);
    let (st, _) = e.chat.post("/events/999/read", json!({})).await;
    rec("post", "/events/{id}/read", st);
    assert_eq!(st, 404);
    let (st, _) = e.read.get("/events?kind=zzz").await;
    rec("get", "/events", st);
    assert_eq!(st, 422);

    let (st, page) = e.read.get("/settings/indicators").await;
    rec("get", "/settings/indicators", st);
    check("IndicatorsPage", &page, &all);
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .for_each(|i| check("Indicator", i, &all));
    let (st, _) = e
        .admin
        .patch("/settings/indicators", json!({"items": []}))
        .await;
    rec("patch", "/settings/indicators", st);
    assert_eq!(st, 428);

    e.admin.post("/_test/channel", json!({"mode": "qr"})).await;
    let a = e
        .admin
        .make(
            "/accounts",
            json!({"type": "scripted-channel", "label": "c"}),
        )
        .await["id"]
        .as_i64()
        .unwrap();
    let (st, flow) = e
        .admin
        .post(&format!("/accounts/{a}/connect"), json!({}))
        .await;
    rec("post", "/accounts/{id}/connect", st);
    assert_eq!(st, 202);
    check("ConnectFlow", &flow, &all);
    let (st, _) = e.admin.post("/accounts/999/connect", json!({})).await;
    rec("post", "/accounts/{id}/connect", st);
    assert_eq!(st, 404);
    let (st, _) = e.admin.delete(&format!("/accounts/{a}/connect")).await;
    rec("delete", "/accounts/{id}/connect", st);
    assert_eq!(st, 204);

    assert!(seen.len() >= 11, "{} calls", seen.len());
    for (method, path, st) in &seen {
        let ok = declared(path, method);
        assert!(!ok.is_empty(), "{method} {path}: operation not in the spec");
        assert!(
            ok.contains(st) || matches!(st, 403 | 404 | 409 | 422 | 428),
            "{method} {path} -> {st}, declared {ok:?}"
        );
    }
}
