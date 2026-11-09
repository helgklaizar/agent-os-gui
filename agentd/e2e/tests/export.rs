//! A6-11: `POST /sessions/{id}/export`: encrypted export only (`age`), recipients from settings, `export/` with `0700`,
//! files `0600`, decrypting with the test key returns the whole session; without recipients: `422 backup_not_configured`.

mod common;

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;

use age::secrecy::ExposeSecret;
use common::Env;
use serde_json::{Value, json};

async fn set_recipients(e: &Env, primary: &str, recovery: Option<&str>) {
    let (_, page) = e.admin.get("/settings/schedule").await;
    let mut r = json!({"primary": primary});
    if let Some(x) = recovery {
        r["recovery"] = json!(x);
    }
    let (st, v) = e
        .admin
        .patch_if_match(
            "/settings/schedule",
            json!({"backup": {"recipient": r}}),
            page["rev"].as_i64().unwrap_or(0),
        )
        .await;
    assert_eq!(st, 200, "{v}");
}

fn decrypt(path: &std::path::Path, id: &age::x25519::Identity) -> Value {
    let f = fs::File::open(path).unwrap();
    let d = age::Decryptor::new(f).unwrap();
    let mut out = Vec::new();
    d.decrypt(std::iter::once(id as &dyn age::Identity))
        .unwrap()
        .read_to_end(&mut out)
        .unwrap();
    serde_json::from_slice(&out).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn export_is_encrypted_for_both_recipients_and_complete() {
    let e = Env::start().await;
    let (a, b) = (
        age::x25519::Identity::generate(),
        age::x25519::Identity::generate(),
    );
    set_recipients(
        &e,
        &a.to_public().to_string(),
        Some(&b.to_public().to_string()),
    )
    .await;
    let (_, s) = e.model_session().await;
    e.say(s, "c1", "secret phrase for export").await;
    for _ in 0..100 {
        if e.messages(s).await.iter().any(|m| m["role"] == "agent") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let (st, v) = e
        .chat
        .post(&format!("/sessions/{s}/export"), json!({}))
        .await;
    assert_eq!(st, 200, "{v}");
    let path = std::path::PathBuf::from(v["path"].as_str().unwrap());
    assert_eq!(
        v["size"].as_u64().unwrap(),
        fs::metadata(&path).unwrap().len()
    );
    assert!(path.starts_with(e.home.join(".agentos/export")), "{path:?}");
    assert!(
        path.extension().is_some_and(|x| x == "age"),
        "encrypted file only"
    );
    let raw = fs::read(&path).unwrap();
    assert!(
        !String::from_utf8_lossy(&raw).contains("secret phrase"),
        "no plaintext in the file"
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    for id in [&a, &b] {
        let doc = decrypt(&path, id);
        assert_eq!(doc["session"]["id"], s);
        let bodies: Vec<&str> = doc["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["body_md"].as_str())
            .collect();
        assert!(
            bodies.iter().any(|b| b.contains("secret phrase")),
            "{bodies:?}"
        );
    }
    let stranger = age::x25519::Identity::generate();
    let f = fs::File::open(&path).unwrap();
    let d = age::Decryptor::new(f).unwrap();
    assert!(
        d.decrypt(std::iter::once(&stranger as &dyn age::Identity))
            .is_err(),
        "a foreign key does not open it"
    );
    let all: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(all.len(), 1, "no unencrypted or temporary files");
    let _ = a.to_string().expose_secret();
}

#[tokio::test(flavor = "multi_thread")]
async fn export_without_recipients_or_with_bad_key_is_422() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    let (st, v) = e
        .chat
        .post(&format!("/sessions/{s}/export"), json!({}))
        .await;
    assert_eq!(st, 422, "{v}");
    assert_eq!(v["error"]["details"]["reason"], "backup_not_configured");
    set_recipients(&e, "not a key", None).await;
    let (st, v) = e
        .chat
        .post(&format!("/sessions/{s}/export"), json!({}))
        .await;
    assert_eq!(st, 422, "{v}");
    assert!(
        !e.home.join(".agentos/export").exists()
            || fs::read_dir(e.home.join(".agentos/export"))
                .unwrap()
                .next()
                .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn export_unknown_session_and_scope() {
    let e = Env::start().await;
    let a = age::x25519::Identity::generate();
    set_recipients(&e, &a.to_public().to_string(), None).await;
    assert_eq!(e.chat.post("/sessions/9999/export", json!({})).await.0, 404);
    let (_, s) = e.model_session().await;
    assert_eq!(
        e.read
            .post(&format!("/sessions/{s}/export"), json!({}))
            .await
            .0,
        403
    );
}
