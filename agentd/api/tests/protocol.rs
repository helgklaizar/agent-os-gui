//! A1-08: `/health`, `503 migrating`, `X-Request-Id`, `/metrics`.

mod common;

use agentd_api::{MigrationStatus, MigrationView, Server};
use common::{
    TOKEN_CHAT, TOKEN_READ, config, db_services, start_with, tcp_req, unix_get, unix_req,
};

#[tokio::test]
async fn health_anonymous_fields() {
    let dir = tempfile::tempdir().unwrap();
    let (_db, services) = db_services(dir.path());
    let server = start_with(config(dir.path()), services).await;
    let resp = unix_get(server.socket_path(), "/v1/health", &[])
        .await
        .expect("response");
    assert_eq!(resp.status, 200);
    let v = resp.json();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["api"]["major"], 1);
    assert_eq!(v["api"]["minor"], 0);
    for k in ["min_client", "min_gui", "max_gui"] {
        assert!(v[k].as_str().is_some_and(|s| !s.is_empty()), "{k}: {v}");
    }
    assert_eq!(
        v["user_version"],
        agentd_db::MIGRATIONS.last().unwrap().version,
        "user_version from the DB"
    );
    let caps: Vec<&str> = v["capabilities"]
        .as_array()
        .expect("capabilities is an array")
        .iter()
        .filter_map(|c| c.as_str())
        .collect();
    assert!(
        caps.contains(&"if-match") && caps.contains(&"ws-seq"),
        "{caps:?}"
    );
    assert!(v["migration"].is_null());
    assert_eq!(
        resp.header("x-agentd-version"),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert!(resp.header("x-request-id").is_some());
    server.shutdown().await;
}

#[tokio::test]
async fn health_migrating_503() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start_booting(config(dir.path())).await.unwrap();
    let sock = server.socket_path().to_path_buf();
    let health = || async {
        unix_get(&sock, "/v1/health", &[])
            .await
            .expect("health always responds")
    };
    // Start without migration: status starting, other routes return core_down.
    let h = health().await;
    assert_eq!(
        (h.status, h.json()["status"].clone()),
        (200, "starting".into())
    );
    let r = unix_req(&sock, "GET", "/v1/sessions", Some(TOKEN_CHAT)).await;
    assert_eq!(
        (r.status, r.json()["error"]["code"].clone()),
        (503, "core_down".into())
    );

    let ctl = server.health();
    ctl.set_migration(Some(MigrationView {
        status: MigrationStatus::Running,
        from: 0,
        to: 8,
        step: Some("0004_chat".into()),
        progress: Some(0.5),
    }));
    let h = health().await;
    assert_eq!(h.status, 200);
    assert_eq!(h.json()["status"], "migrating");
    assert_eq!(h.json()["migration"]["step"], "0004_chat");
    assert_eq!(h.json()["migration"]["progress"], 0.5);
    assert_eq!(h.json()["migration"]["status"], "running");

    // With and without a token: 503 migrating, Retry-After, migration object in details.
    for token in [Some(TOKEN_CHAT), None] {
        let r = unix_req(&sock, "GET", "/v1/sessions", token).await;
        assert_eq!(r.status, 503, "{token:?}");
        let e = &r.json()["error"];
        assert_eq!(e["code"], "migrating");
        assert_eq!(e["retryable"], true);
        assert_eq!(e["details"]["migration"]["progress"], 0.5);
        assert!(
            r.header("retry-after")
                .is_some_and(|v| v.parse::<u64>().is_ok_and(|n| n >= 1))
        );
        assert_eq!(e["request_id"].as_str(), r.header("x-request-id"));
    }
    // WS is not accepted during migration.
    let r = unix_get(&sock, "/v1/ws", &[]).await.unwrap();
    assert_eq!(
        (r.status, r.json()["error"]["code"].clone()),
        (503, "migrating".into())
    );

    ctl.set_migration(Some(MigrationView {
        status: MigrationStatus::Failed,
        from: 0,
        to: 8,
        step: Some("0004_chat".into()),
        progress: None,
    }));
    assert_eq!(health().await.json()["migration"]["status"], "failed");
    let r = unix_req(&sock, "GET", "/v1/sessions", Some(TOKEN_CHAT)).await;
    assert_eq!(
        r.json()["error"]["details"]["migration"]["status"],
        "failed"
    );

    // Migration done: services attached, status ok, routes check the token again.
    ctl.set_migration(Some(MigrationView {
        status: MigrationStatus::Done,
        from: 0,
        to: 8,
        step: None,
        progress: Some(1.0),
    }));
    server.set_ready(agentd_api::Services::new(common::auth()));
    assert_eq!(health().await.json()["status"], "ok");
    assert_eq!(
        unix_req(&sock, "GET", "/v1/sessions", None).await.status,
        401
    );
    server.shutdown().await;
}

#[tokio::test]
async fn request_id_echo_and_generate() {
    let dir = tempfile::tempdir().unwrap();
    let (_db, services) = db_services(dir.path());
    let server = start_with(config(dir.path()), services).await;
    let rid = uuid::Uuid::now_v7().to_string();
    let hdr = format!("X-Request-Id: {rid}");
    let r = unix_get(server.socket_path(), "/v1/health", &[&hdr])
        .await
        .unwrap();
    assert_eq!(
        r.header("x-request-id"),
        Some(rid.as_str()),
        "echo of a valid UUID v7"
    );
    // The error carries the same id in body and header.
    let r = unix_get(server.socket_path(), "/v1/sessions", &[&hdr])
        .await
        .unwrap();
    assert_eq!(r.status, 401);
    assert_eq!(r.json()["error"]["request_id"].as_str(), Some(rid.as_str()));
    assert_eq!(r.header("x-request-id"), Some(rid.as_str()));
    for bad in [
        "X-Request-Id:".to_string(),
        "X-Request-Id: garbage".to_string(),
        "X-Request-Id: f47ac10b-58cc-4372-a567-0e02b2c3d479".to_string(),
        format!("X-Request-Id: {}", uuid::Uuid::now_v7().simple()),
    ] {
        let r = unix_get(server.socket_path(), "/v1/sessions", &[&bad])
            .await
            .unwrap();
        let got = r
            .header("x-request-id")
            .expect("header is always present")
            .to_string();
        let parsed = uuid::Uuid::parse_str(&got).expect("own id is a UUID");
        assert_eq!(parsed.get_version_num(), 7, "{bad}");
        assert_ne!(
            Some(got.as_str()),
            bad.split_once(": ").map(|x| x.1),
            "{bad}"
        );
        assert_eq!(r.json()["error"]["request_id"].as_str(), Some(got.as_str()));
    }
    server.shutdown().await;
}

#[tokio::test]
async fn metrics_content_and_access() {
    let dir = tempfile::tempdir().unwrap();
    let (_db, services) = db_services(dir.path());
    let server = start_with(config(dir.path()), services).await;
    let p = server.socket_path();
    unix_req(
        p,
        "GET",
        "/v1/sessions/secret-name-123/bookmarks",
        Some(TOKEN_CHAT),
    )
    .await;
    unix_req(p, "POST", "/v1/sessions", Some(TOKEN_READ)).await;
    unix_req(p, "GET", "/v1/sessions", None).await;
    unix_req(p, "GET", "/v1/health", None).await;
    let r = unix_req(p, "GET", "/v1/metrics", Some(TOKEN_CHAT)).await;
    assert_eq!(r.status, 200);
    assert!(
        r.header("content-type")
            .is_some_and(|c| c.starts_with("text/plain"))
    );
    let text = r.body;
    for want in [
        "http_requests_total{route=\"/sessions/{id}/bookmarks\",status=\"400\"} 1",
        "http_requests_total{route=\"/sessions\",status=\"403\"} 1",
        "http_requests_total{route=\"unmatched\",status=\"401\"} 1",
        "http_requests_total{route=\"/health\",status=\"200\"} 1",
        "http_errors_total{code=\"insufficient_scope\"} 1",
        "http_errors_total{code=\"unauthorized\"} 1",
        "http_request_duration_ms_count{route=\"/sessions/{id}/bookmarks\"} 1",
        format!(
            "agentd_user_version {}",
            agentd_db::MIGRATIONS.last().unwrap().version
        )
        .as_str(),
        "agentd_db_write_queue_len 0",
        "ws_connections 0",
    ] {
        assert!(
            text.lines().any(|l| l == want),
            "missing line \"{want}\" in:\n{text}"
        );
    }
    for secret in ["secret-name-123", TOKEN_CHAT, TOKEN_READ] {
        assert!(!text.contains(secret), "metrics must not contain {secret}");
    }
    // Admin only and local only.
    let r = unix_req(p, "GET", "/v1/metrics", Some(TOKEN_READ)).await;
    assert_eq!(r.status, 403);
    server.shutdown().await;
    let mut cfg = config(dir.path());
    cfg.socket_path = dir.path().join("second.sock");
    cfg.tcp_enabled = true;
    let (_db2, services) = db_services(&dir.path().join("two"));
    let server = start_with(cfg, services).await;
    let addr = loop {
        if let Some(a) = server.tcp().local_addr() {
            break a;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    assert_eq!(
        tcp_req(addr, "GET", "/v1/metrics", Some(TOKEN_CHAT))
            .await
            .status,
        403
    );
    server.shutdown().await;
}
