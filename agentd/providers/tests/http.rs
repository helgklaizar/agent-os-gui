//! HTTP client: timeouts, network, cancellation, statuses.

mod common;

use std::time::Duration;

use agentd_providers::http::{HttpClient, Timeouts};
use common::{Script, serve};

fn client(first: u64, idle: u64) -> HttpClient {
    HttpClient::new(Timeouts {
        connect: Duration::from_millis(500),
        first_event: Duration::from_millis(first),
        idle: Duration::from_millis(idle),
    })
}

#[tokio::test]
async fn provider_timeout_to_error() {
    let mut s = Script::sse("");
    s.chunks.clear();
    s.hang = true;
    s.head_delay = Duration::from_secs(5);
    let srv = serve(s).await;
    let e = client(200, 200)
        .post_sse(
            &format!("{}/x", srv.url),
            &[],
            b"{}".to_vec(),
            |_| {},
            |_| true,
        )
        .await
        .unwrap_err();
    assert_eq!((e.code, e.retryable), ("timeout", true));
}

#[tokio::test]
async fn provider_idle_timeout_midstream() {
    let mut s = Script::sse("data: 1\n\n");
    s.hang = true;
    let srv = serve(s).await;
    let mut got = Vec::new();
    let e = client(300, 200)
        .post_sse(
            &format!("{}/x", srv.url),
            &[],
            b"{}".to_vec(),
            |_| {},
            |ev| {
                got.push(ev.data);
                true
            },
        )
        .await
        .unwrap_err();
    assert_eq!(got, ["1"]);
    assert_eq!((e.code, e.retryable), ("timeout", true));
}

#[tokio::test]
async fn provider_connect_refused_is_network_error() {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/x", l.local_addr().unwrap());
    drop(l);
    let e = client(200, 200)
        .post_sse(&url, &[], b"{}".to_vec(), |_| {}, |_| true)
        .await
        .unwrap_err();
    assert_eq!((e.code, e.retryable), ("network", true));
}

#[tokio::test]
async fn http_status_maps_without_body_text() {
    for (st, code, retry) in [
        (401, "unauthorized", false),
        (429, "rate_limited", true),
        (500, "upstream", true),
        (529, "overloaded", true),
    ] {
        let srv = serve(Script::status(st)).await;
        let e = client(500, 500)
            .post_sse(
                &format!("{}/x", srv.url),
                &[],
                b"{}".to_vec(),
                |_| {},
                |_| true,
            )
            .await
            .unwrap_err();
        assert_eq!((e.code, e.retryable, e.status), (code, retry, Some(st)));
        assert!(!format!("{e:?}").contains("secret-detail"));
    }
}

#[tokio::test]
async fn http_sends_headers_and_body_and_reads_events() {
    let srv = serve(Script::sse("event: a\ndata: 1\n\ndata: 2\n\n")).await;
    let mut events = Vec::new();
    client(500, 500)
        .post_sse(
            &format!("{}/v1/path", srv.url),
            &[("x-test", "yes".into())],
            b"{\"a\":1}".to_vec(),
            |r| assert_eq!(r.status, 200),
            |ev| {
                events.push(ev.data);
                true
            },
        )
        .await
        .unwrap();
    assert_eq!(events, ["1", "2"]);
    let seen = srv.seen.lock().unwrap();
    assert!(seen[0].request_line.starts_with("POST /v1/path "));
    assert_eq!(seen[0].header("x-test"), Some("yes"));
    assert_eq!(seen[0].body, "{\"a\":1}");
}

#[tokio::test]
async fn http_drop_future_closes_connection() {
    let mut s = Script::sse("data: 1\n\n");
    s.hang = true;
    let srv = serve(s).await;
    let c = client(2000, 5000);
    let url = format!("{}/x", srv.url);
    let fut = c.post_sse(&url, &[], b"{}".to_vec(), |_| {}, |_| true);
    let _ = tokio::time::timeout(Duration::from_millis(300), fut).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        *srv.closed.lock().unwrap(),
        1,
        "the server saw the connection close"
    );
}

#[tokio::test]
async fn http_get_status() {
    let srv = serve(Script::status(200)).await;
    assert_eq!(
        client(500, 500)
            .get_status(&format!("{}/m", srv.url), &[])
            .await
            .unwrap(),
        200
    );
}
