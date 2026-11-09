//! A7-06: the `bar.weather` topic (Open-Meteo without a key, 30 minute period; in the test build 1 s and an address from a file): values from
//! the source, `null` when unavailable and without coordinates; no fake values; the request carries coordinates only.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::Env;
use core_client::api::FrameKind;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Fake Open-Meteo: records the request path, replies with the given status and body.
async fn fake(status: u16, body: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut c, _)) = l.accept().await else {
                return;
            };
            let s3 = s2.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = c.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some(path) = head.lines().next().and_then(|l| l.split(' ').nth(1)) {
                    s3.lock().unwrap().push(path.to_string());
                }
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                drop(c.write_all(reply.as_bytes()).await);
            });
        }
    });
    (url, seen)
}

async fn set_location(e: &Env) {
    let (_, page) = e.admin.get("/settings/account").await;
    let (st, v) = e
        .admin
        .patch_if_match(
            "/settings/account",
            json!({"lat": 32.8, "lon": 35.0}),
            page["rev"].as_i64().unwrap_or(0),
        )
        .await;
    assert_eq!(st, 200, "{v}");
}

fn point_to(e: &Env, url: &str) {
    std::fs::write(e.home.join("weather.url"), url).unwrap();
}

fn temp_of(v: &Value) -> Option<f64> {
    v["temp"].as_f64()
}

#[tokio::test(flavor = "multi_thread")]
async fn weather_without_location_is_all_null_and_no_request() {
    let e = Env::start().await;
    let (url, seen) = fake(
        200,
        r#"{"current":{"temperature_2m":21.5,"weather_code":3}}"#,
    )
    .await;
    point_to(&e, &url);
    let mut ws = e.ws();
    let snap = ws.subscribe("bar.weather");
    assert_eq!(
        (temp_of(&snap), &snap["code"], &snap["desc"]),
        (None, &Value::Null, &Value::Null),
        "{snap}"
    );
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(
        seen.lock().unwrap().is_empty(),
        "no requests without coordinates"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn weather_takes_values_from_source_and_sends_only_coordinates() {
    let e = Env::start().await;
    let (url, seen) = fake(
        200,
        r#"{"current":{"temperature_2m":21.5,"weather_code":3}}"#,
    )
    .await;
    point_to(&e, &url);
    set_location(&e).await;
    let mut ws = e.ws();
    ws.subscribe("bar.weather");
    let f = ws.until(Duration::from_secs(12), |f| {
        f.topic == "bar.weather" && f.kind == FrameKind::Delta && temp_of(&f.data).is_some()
    });
    let d = &f.last().unwrap().data;
    assert_eq!(
        (temp_of(d), d["code"].as_i64()),
        (Some(21.5), Some(3)),
        "{d}"
    );
    assert!(d["desc"].as_str().is_some_and(|s| !s.is_empty()));
    let req = seen.lock().unwrap()[0].clone();
    assert!(
        req.contains("latitude=32.8") && req.contains("longitude=35"),
        "{req}"
    );
    assert!(
        !req.contains("token") && !req.contains("name") && !req.contains("key"),
        "{req}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn weather_source_failure_gives_nulls_not_stale_values() {
    let e = Env::start().await;
    let (good, _) = fake(
        200,
        r#"{"current":{"temperature_2m":10.0,"weather_code":0}}"#,
    )
    .await;
    point_to(&e, &good);
    set_location(&e).await;
    let mut ws = e.ws();
    ws.subscribe("bar.weather");
    ws.until(Duration::from_secs(12), |f| {
        f.kind == FrameKind::Delta && temp_of(&f.data).is_some()
    });
    let (bad, _) = fake(500, "{}").await;
    point_to(&e, &bad);
    let f = ws.until(Duration::from_secs(12), |f| {
        f.kind == FrameKind::Delta && temp_of(&f.data).is_none()
    });
    assert_eq!(f.last().unwrap().data["code"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn weather_unavailable_null() {
    // The source does not respond at all (port closed): frames and `snapshot` are `null`, not old or invented values.
    let e = Env::start().await;
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    point_to(&e, &closed);
    set_location(&e).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let mut ws = e.ws();
    let snap = ws.subscribe("bar.weather");
    assert_eq!(
        (temp_of(&snap), &snap["code"], &snap["desc"]),
        (None, &Value::Null, &Value::Null),
        "{snap}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn weather_stale_value_not_shown() {
    // A value existed, then the source vanished: a new subscriber gets `null` in `snapshot`; the stale value is not shown.
    let e = Env::start().await;
    let (good, _) = fake(
        200,
        r#"{"current":{"temperature_2m":12.5,"weather_code":1}}"#,
    )
    .await;
    point_to(&e, &good);
    set_location(&e).await;
    let mut ws = e.ws();
    ws.subscribe("bar.weather");
    ws.until(Duration::from_secs(12), |f| {
        f.kind == FrameKind::Delta && temp_of(&f.data).is_some()
    });
    let (bad, _) = fake(503, "{}").await;
    point_to(&e, &bad);
    ws.until(Duration::from_secs(12), |f| {
        f.kind == FrameKind::Delta && temp_of(&f.data).is_none()
    });
    let mut late = e.ws();
    let snap = late.subscribe("bar.weather");
    assert_eq!(temp_of(&snap), None, "stale value in snapshot: {snap}");
    assert_eq!(snap["desc"], Value::Null);
}
