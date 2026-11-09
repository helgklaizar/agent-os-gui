//! A7-10: `GET /v1/metrics` on a live core after scenarios with recognisable user data: counters only,
//! no request values (names, texts, tokens) appear in the exposition.

mod common;

use common::{Env, raw};

#[tokio::test(flavor = "multi_thread")]
async fn metrics_no_user_data() {
    let e = Env::start().await;
    let (st, s) = e
        .admin
        .post(
            "/sessions",
            serde_json::json!({"name": "PII-session-name-77"}),
        )
        .await;
    assert_eq!(st, 201, "{s}");
    let id = s["id"].as_i64().unwrap();
    e.say(id, "c1", "PII-message-text-88 password hunter2")
        .await;
    e.admin.get("/sessions/999999/messages").await;
    let r = raw(&e.sock, &e.admin_token, "GET", "/metrics", None, b"").await;
    assert_eq!(r.status, 200);
    let text = String::from_utf8(r.body).unwrap();
    for secret in [
        "PII-session-name-77",
        "PII-message-text-88",
        "hunter2",
        e.admin_token.as_str(),
        e.chat_token.as_str(),
    ] {
        assert!(!text.contains(secret), "user data in the metrics: {secret}");
    }
    assert!(text.contains("http_requests_total{route="), "{text}");
    for l in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let (name_labels, value) = l.rsplit_once(' ').expect(l);
        assert!(value.parse::<f64>().is_ok(), "value is not a number: {l}");
        assert!(
            name_labels.chars().all(|c| c.is_ascii() && !c.is_control()),
            "non-ASCII in a name or label: {l}"
        );
    }
}
