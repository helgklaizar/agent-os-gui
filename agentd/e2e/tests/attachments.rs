//! A3-07: attachments through the `agentd` binary: limit, name as data, download, session binding.

mod common;

use common::Env;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn attachment_upload_limit_25mib() {
    let e = Env::start().await;
    let s = e.session("x").await;
    let big = vec![7u8; 25 * 1024 * 1024 + 1];
    let r = e
        .upload(s, "big.bin", "application/octet-stream", &big)
        .await;
    assert_eq!(
        (
            r.status,
            r.json()["error"]["code"].as_str().map(str::to_string)
        ),
        (413, Some("payload_too_large".into()))
    );
    let edge = vec![7u8; 25 * 1024 * 1024];
    let r = e
        .upload(s, "edge.bin", "application/octet-stream", &edge)
        .await;
    assert_eq!(
        r.status,
        201,
        "exactly 25 MiB is accepted: {}",
        String::from_utf8_lossy(&r.body)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attachment_name_not_path() {
    let e = Env::start().await;
    let s = e.session("x").await;
    let r = e.upload(s, "../../etc/passwd", "text/plain", b"data").await;
    assert_eq!(r.status, 201);
    let a = r.json();
    assert_eq!(a["name"], "../../etc/passwd", "name is stored as data");
    let id = a["id"].as_i64().unwrap();
    let dir = e.home.join(".agentos/attachments");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|x| x.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(
        names.contains(&id.to_string()) && names.contains(&format!("{id}.json")),
        "{names:?}"
    );
    assert!(
        names
            .iter()
            .all(|n| !n.contains('/') && !n.contains("passwd")),
        "{names:?}"
    );
    assert!(!e.home.join("etc").exists() && !e.dir.path().join("etc").exists());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(dir.join(id.to_string()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attachment_download_roundtrip() {
    let e = Env::start().await;
    let s = e.session("x").await;
    let data: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
    let r = e.upload(s, "bin.dat", "application/x-custom", &data).await;
    assert_eq!(r.status, 201);
    let a = r.json();
    assert_eq!(
        (a["name"].as_str(), a["mime"].as_str(), a["size"].as_u64()),
        (Some("bin.dat"), Some("application/x-custom"), Some(70_000))
    );
    let d = e.download(a["id"].as_i64().unwrap()).await;
    assert_eq!(d.status, 200);
    assert_eq!(d.body, data);
    assert_eq!(d.header("x-content-type-options"), Some("nosniff"));
    assert!(
        d.header("content-disposition")
            .unwrap()
            .starts_with("attachment")
    );
    assert_eq!(e.download(9999).await.status, 404);
    let (st, _) = e.read.get("/attachments/abc").await;
    assert_eq!(st, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn attachment_in_message_meta_and_session_bound() {
    let e = Env::start().await;
    let (s, other) = (e.session("a").await, e.session("b").await);
    let att = e.upload(s, "n.txt", "text/plain", b"hello").await.json();
    let id = att["id"].as_i64().unwrap();
    let (st, m) = e
        .chat
        .post(
            &format!("/sessions/{s}/messages"),
            json!({"body_md": "see file", "client_id": "c", "attachment_ids": [id]}),
        )
        .await;
    assert_eq!(st, 201, "{m}");
    assert_eq!(m["meta"]["attachments"][0], att);
    let (st, b) = e
        .chat
        .post(
            &format!("/sessions/{other}/messages"),
            json!({"body_md": "x", "client_id": "c", "attachment_ids": [id]}),
        )
        .await;
    assert_eq!(st, 422, "foreign attachment: {b}");
    let r = e.upload(9999, "n", "text/plain", b"x").await;
    assert_eq!(r.status, 404);
}
