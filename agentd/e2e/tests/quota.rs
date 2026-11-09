//! A3-10: actual-usage quotas and the `bar.quota` topic on the real `agentd` binary with the `scripted` provider.

mod common;

use std::time::Duration;

use common::Env;
use serde_json::{Value, json};

async fn quota_of(e: &Env, acc: i64) -> Value {
    let (st, q) = e.read.get("/quotas").await;
    assert_eq!(st, 200, "{q}");
    q["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["account_id"] == acc)
        .unwrap_or_else(|| panic!("no quota for account {acc}: {q}"))
        .clone()
}

async fn answer(e: &Env, s: i64, cid: &str, text: &str, n: usize) {
    let (st, b) = e.say(s, cid, text).await;
    assert_eq!(st, 201, "{b}");
    e.wait_agent_done(s, n).await;
    // Accounting runs right after the message is closed.
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_unknown_limit_nulls() {
    let e = Env::start().await;
    let (acc, s) = e.model_session().await;
    let (st, empty) = e.read.get("/quotas").await;
    assert_eq!(
        (st, empty["items"].as_array().map(Vec::len)),
        (200, Some(0))
    );
    answer(&e, s, "a", "hello", 1).await;
    let q = quota_of(&e, acc).await;
    for k in [
        "used", "rate", "window_h", "limits", "reset_at", "eta_at", "load",
    ] {
        assert_eq!(q[k], Value::Null, "{k}: {q}");
    }
    assert_eq!(q["kind"], "api_key");
    assert_eq!(q["usage"]["period"], "day");
    assert!(
        q["usage"]["tokens_in"].as_u64().unwrap() > 0
            && q["usage"]["tokens_out"].as_u64().unwrap() > 0
    );
    assert_eq!(q["usage"]["cost"], Value::Null);
    assert!(q["updated_at"].as_i64().unwrap() > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_usage_accumulates_and_samples() {
    let mut e = Env::start().await;
    let (acc, s) = e.model_session().await;
    answer(&e, s, "a", "one", 1).await;
    let t1 = quota_of(&e, acc).await["usage"].clone();
    answer(&e, s, "b", "two", 2).await;
    let t2 = quota_of(&e, acc).await["usage"].clone();
    assert!(
        t2["tokens_in"].as_u64() > t1["tokens_in"].as_u64(),
        "{t1} -> {t2}"
    );
    assert!(t2["tokens_out"].as_u64() > t1["tokens_out"].as_u64());
    let (tokens, samples): (i64, i64) = e
        .inspect(move |c| {
            c.query_row(
                "SELECT (SELECT usage_tokens FROM quotas WHERE account_id = ?1), (SELECT count(*) FROM quota_samples WHERE account_id = ?1)",
                [acc],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await;
    assert_eq!(
        tokens as u64,
        t2["tokens_in"].as_u64().unwrap() + t2["tokens_out"].as_u64().unwrap()
    );
    assert!(samples >= 1, "quota_samples is filled");
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_cost_null_without_price_and_computed_with_price() {
    let e = Env::start().await;
    let (acc, s) = e.model_session().await;
    answer(&e, s, "a", "no price", 1).await;
    assert_eq!(quota_of(&e, acc).await["usage"]["cost"], Value::Null);
    let (_, page) = e.admin.get("/settings/models").await;
    let patch = json!({"prices": {"scripted": {"input_per_m": 1_000_000.0, "output_per_m": 2_000_000.0, "currency": "EUR"}}});
    let (st, b) = e
        .admin
        .patch_if_match("/settings/models", patch, page["rev"].as_i64().unwrap())
        .await;
    assert_eq!(st, 200, "{b}");
    let before = quota_of(&e, acc).await["usage"].clone();
    answer(&e, s, "b", "with price", 2).await;
    let after = quota_of(&e, acc).await["usage"].clone();
    // Cost is counted only for a reply with a price: one unit per input token and two per output token.
    let m = e.messages(s).await;
    let u = &m.iter().filter(|x| x["role"] == "agent").nth(1).unwrap()["meta"]["usage"];
    let want = u["tokens_in"].as_f64().unwrap() + 2.0 * u["tokens_out"].as_f64().unwrap();
    assert_eq!(after["cost"].as_f64(), Some(want), "{before} -> {after}");
    assert_eq!(after["currency"], "EUR");
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_rate_limit_headers_applied_and_eta_only_before_reset() {
    let e = Env::start().await;
    let (acc, s) = e.model_session().await;
    answer(&e, s, "a", "#scn:limits", 1).await;
    let q = quota_of(&e, acc).await;
    assert_eq!(q["limits"]["requests_per_min"]["limit"], 100);
    assert_eq!(q["limits"]["requests_per_min"]["remaining"], 80);
    assert_eq!(q["limits"]["tokens_per_min"]["limit"], 10_000);
    assert_eq!(q["limits"]["tokens_per_min"]["remaining"], 8_000);
    assert!(
        q["limits"]["requests_per_min"]["reset_at"]
            .as_i64()
            .unwrap()
            > 0
    );
    assert!((q["used"].as_f64().unwrap() - 0.2).abs() < 1e-9, "{q}");
    assert!((q["window_h"].as_f64().unwrap() - 1.0 / 60.0).abs() < 1e-6);
    assert!(q["rate"].as_f64().unwrap() > 0.0);
    assert_eq!(q["reset_at"], q["limits"]["requests_per_min"]["reset_at"]);
    assert_eq!(q["eta_at"], Value::Null, "enough until reset: {q}");
    answer(&e, s, "b", "#scn:limits_low", 2).await;
    let q = quota_of(&e, acc).await;
    assert!((q["used"].as_f64().unwrap() - 0.95).abs() < 1e-9, "{q}");
    let (eta, reset) = (
        q["eta_at"].as_i64().unwrap(),
        q["reset_at"].as_i64().unwrap(),
    );
    assert!(eta < reset, "forecast before reset: {q}");
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_session_usage() {
    let e = Env::start().await;
    let (acc, s1) = e.model_session().await;
    let s2 = e.session("second").await;
    e.chat
        .patch(&format!("/sessions/{s2}"), json!({"cfg": {"model": acc}}))
        .await;
    answer(&e, s1, "a", "first session", 1).await;
    answer(&e, s2, "b", "second session, with somewhat longer text", 1).await;
    let day = quota_of(&e, acc).await["usage"].clone();
    let (st, q) = e.read.get(&format!("/quotas?session_id={s1}")).await;
    assert_eq!(st, 200, "{q}");
    let u = &q["items"][0]["usage"];
    assert_eq!(
        (u["period"].as_str(), u["session_id"].as_i64()),
        (Some("session"), Some(s1))
    );
    assert!(
        u["tokens_in"].as_u64() < day["tokens_in"].as_u64(),
        "session is less than a day: {u} {day}"
    );
    let m = e.messages(s1).await;
    let mu = &m.iter().find(|x| x["role"] == "agent").unwrap()["meta"]["usage"];
    assert_eq!(u["tokens_in"], mu["tokens_in"]);
    assert_eq!(u["tokens_out"], mu["tokens_out"]);
    let (st, _) = e.read.get("/quotas?session_id=abc").await;
    assert_eq!(st, 400);
    let (st, _) = e.read.get("/quotas?session_id=9999").await;
    assert_eq!(st, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn bar_quota_topic_snapshot_delta() {
    let e = Env::start().await;
    let (acc, s) = e.model_session().await;
    let mut ws = e.ws();
    let snap = ws.subscribe("bar.quota");
    assert_eq!(snap["items"].as_array().map(Vec::len), Some(0));
    answer(&e, s, "a", "hello", 1).await;
    let frames = ws.drain(Duration::from_millis(500));
    let deltas: Vec<_> = frames.iter().filter(|f| f.topic == "bar.quota").collect();
    assert_eq!(deltas.len(), 1, "{frames:?}");
    assert_eq!(deltas[0].data["account_id"], acc);
    assert_eq!(deltas[0].data["usage"]["period"], "day");
    let mut ws2 = e.ws();
    let snap = ws2.subscribe("bar.quota");
    assert_eq!(snap["items"][0]["account_id"], acc);
    let (_, rest) = e.read.get("/quotas").await;
    assert_eq!(
        rest["items"][0]["usage"], snap["items"][0]["usage"],
        "GET /quotas = snapshot"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_requires_token() {
    let e = Env::start().await;
    let r = common::raw(&e.sock, "bad", "GET", "/quotas", None, &[]).await;
    assert_eq!(r.status, 401);
}
