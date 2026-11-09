//! W-100: `GET /v1/actions` is the action log in pages with filters (api.md section 5.4); token values and arguments are not returned.

mod common;

use common::Env;
use serde_json::{Value, json};

fn ids(v: &Value) -> Vec<i64> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn actions_pages_filters_and_shape() {
    let e = Env::start().await;
    let mut issued = Vec::new();
    for n in 0..3 {
        let (st, t) = e
            .admin
            .post(
                "/tokens",
                json!({"name": format!("t{n}"), "scope": "chat", "ttl_days": 5}),
            )
            .await;
        assert_eq!(st, 201, "{t}");
        issued.push(t["token"].as_str().unwrap().to_string());
    }
    // Read with a `read` token: the log is visible without admin rights.
    let (st, all) = e.read.get("/actions").await;
    assert_eq!(st, 200, "{all}");
    let rows = all["items"].as_array().unwrap();
    assert!(rows.len() >= 3, "{all}");
    let first = &rows[0];
    assert_eq!(first["actor"], "user");
    assert_eq!(first["action"], "token_issue");
    assert!(
        first["target"].as_str().unwrap().starts_with("token:"),
        "{first}"
    );
    assert!(first["ts"].as_i64().unwrap() > 0);
    assert!(
        first.get("args_json").is_none() && first.get("undo_json").is_none(),
        "{first}"
    );
    // Newest first; no token values in the response.
    let all_ids = ids(&all);
    assert!(all_ids.windows(2).all(|w| w[0] > w[1]), "{all_ids:?}");
    let dump = all.to_string();
    for v in &issued {
        assert!(!dump.contains(v.as_str()), "token value in the log");
    }
    // Pages: limit=2, the cursor leads to older entries without repeats.
    let (_, p1) = e.read.get("/actions?limit=2").await;
    assert_eq!(ids(&p1).len(), 2, "{p1}");
    let cur = p1["next_cursor"].as_str().expect("cursor").to_string();
    let (_, p2) = e.read.get(&format!("/actions?limit=2&cursor={cur}")).await;
    assert!(
        ids(&p2).iter().all(|i| *i < *ids(&p1).last().unwrap()),
        "{p1} {p2}"
    );
    assert_eq!(ids(&p2)[0], all_ids[2], "{p2}");
    // Exactly one full page: no cursor, and no extra empty page either.
    let (_, exact) = e
        .read
        .get(&format!("/actions?limit={}", all_ids.len()))
        .await;
    assert_eq!(ids(&exact), all_ids);
    assert!(exact["next_cursor"].is_null(), "{exact}");
    // Filters.
    let (_, none) = e.read.get("/actions?actor=agent").await;
    assert!(ids(&none).is_empty(), "{none}");
    assert!(none["next_cursor"].is_null());
    let (_, users) = e.read.get("/actions?actor=user").await;
    assert_eq!(ids(&users), all_ids);
    let (_, future) = e.read.get("/actions?from=9999999999").await;
    assert!(ids(&future).is_empty());
    let (_, past) = e.read.get("/actions?to=1").await;
    assert!(ids(&past).is_empty());
    // Parameter validation.
    for bad in [
        "actor=bot",
        "limit=0",
        "limit=500",
        "cursor=x",
        "session_id=a",
        "from=-",
    ] {
        let (st, body) = e.read.get(&format!("/actions?{bad}")).await;
        assert_eq!(st, 422, "{bad}: {body}");
    }
}
