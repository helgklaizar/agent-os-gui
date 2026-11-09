//! A2-05: session settings `cfg` (shape, merge, `perms`, `model`) and the `path` path.

mod common;

use common::{Env, fields};
use serde_json::{Value, json};

async fn session(e: &Env, body: Value) -> Value {
    e.chat.make("/sessions", body).await
}

#[tokio::test(flavor = "multi_thread")]
async fn session_cfg_perms_form() {
    // Since A4-02 a non-empty `perms` is allowed if it holds "action - mode" pairs; the permission engine checks the meaning
    // (`session_cfg_perms_only_tighten` in `permissions_api.rs`).
    let e = Env::start().await;
    let s = session(&e, json!({"name": "s"})).await;
    let path = format!("/sessions/{}", s["id"]);
    let (st, v) = e
        .chat
        .patch(&path, json!({"cfg": {"perms": {"shell": "maybe"}}}))
        .await;
    assert_eq!(
        (st, fields(&v)),
        (422, vec!["cfg.perms".to_string()]),
        "{v}"
    );
    let (st, v) = e.chat.patch(&path, json!({"cfg": {"perms": {}}})).await;
    assert_eq!((st, v["cfg"]["perms"].clone()), (200, json!({})));
}

#[tokio::test(flavor = "multi_thread")]
async fn session_cfg_model_must_exist() {
    let e = Env::start().await;
    let a = e
        .admin
        .make("/accounts", json!({"type": "anthropic", "label": "k"}))
        .await;
    let s = session(&e, json!({"name": "s"})).await;
    let path = format!("/sessions/{}", s["id"]);
    let (st, v) = e.chat.patch(&path, json!({"cfg": {"model": 999}})).await;
    assert_eq!(
        (st, fields(&v)),
        (422, vec!["cfg.model".to_string()]),
        "{v}"
    );
    let (st, v) = e
        .chat
        .patch(&path, json!({"cfg": {"model": a["id"]}}))
        .await;
    assert_eq!((st, v["cfg"]["model"].clone()), (200, a["id"].clone()));
    e.admin.delete(&format!("/accounts/{}", a["id"])).await;
    let (st, v) = e
        .chat
        .patch(&path, json!({"cfg": {"model": a["id"]}}))
        .await;
    assert_eq!(
        (st, fields(&v)),
        (422, vec!["cfg.model".to_string()]),
        "deleted account: {v}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_cfg_shape_and_merge() {
    let e = Env::start().await;
    let s = session(&e, json!({"name": "s"})).await;
    let path = format!("/sessions/{}", s["id"]);
    for bad in [
        json!({"mem": "yes"}),
        json!({"notif": "loud"}),
        json!({"ctx": "all"}),
        json!({"auto": 1}),
        json!({"model": "x"}),
        json!({"unknown": 1}),
    ] {
        let (st, v) = e.chat.patch(&path, json!({"cfg": bad})).await;
        assert_eq!(st, 422, "{bad}: {v}");
    }
    assert_eq!(e.chat.patch(&path, json!({"cfg": 5})).await.0, 422);
    e.chat
        .patch(&path, json!({"cfg": {"mem": true, "notif": "off"}}))
        .await;
    let (_, v) = e
        .chat
        .patch(&path, json!({"cfg": {"auto": true, "mem": null}}))
        .await;
    assert_eq!(
        v["cfg"],
        json!({"notif": "off", "auto": true}),
        "keys are merged, null removes a key"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_path_validation() {
    let e = Env::start().await;
    let ok = e.home.join("code/proj").to_string_lossy().into_owned();
    let s = session(&e, json!({"name": "s", "path": ok})).await;
    assert_eq!(
        s["path"].as_str(),
        Some(ok.as_str()),
        "a nonexistent directory is not a refusal"
    );
    let long = format!("/{}", "a".repeat(5000));
    for bad in [
        "relative/dir",
        "../x",
        "/home/../etc",
        "/ok/with\0nul",
        "",
        long.as_str(),
    ] {
        let (st, v) = e
            .chat
            .post("/sessions", json!({"name": "x", "path": bad}))
            .await;
        assert_eq!(
            (st, fields(&v)),
            (422, vec!["path".to_string()]),
            "{bad:?}: {v}"
        );
    }
    // A symlink from the home directory to the outside.
    let outside = e.dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let link = e.home.join("link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let (st, v) = e
        .chat
        .post(
            "/sessions",
            json!({"name": "x", "path": link.to_string_lossy()}),
        )
        .await;
    assert_eq!((st, fields(&v)), (422, vec!["path".to_string()]), "{v}");
    // A link inside home is fine.
    let inside = e.home.join("real");
    std::fs::create_dir_all(&inside).unwrap();
    let link2 = e.home.join("link2");
    std::os::unix::fs::symlink(&inside, &link2).unwrap();
    assert_eq!(
        e.chat
            .post(
                "/sessions",
                json!({"name": "x", "path": link2.to_string_lossy()})
            )
            .await
            .0,
        201
    );
    // PATCH is checked the same way.
    assert_eq!(
        e.chat
            .patch(&format!("/sessions/{}", s["id"]), json!({"path": "rel"}))
            .await
            .0,
        422
    );
}
