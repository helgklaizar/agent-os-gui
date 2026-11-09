//! A6-10: `/files` — editing `AGENT.md` and `memory/`: path normalisation, no escaping the roots, no symlinks,
//! `ETag` and `If-Match` (`412`, `428`), atomic write with `0600`, `admin` only.

mod common;
mod openapi;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use common::{Env, raw, raw_hdr};
use openapi::{check, schemas};

fn agentos(e: &Env) -> std::path::PathBuf {
    let p = e.home.join(".agentos");
    fs::create_dir_all(p.join("memory")).unwrap();
    p
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'.' || b == b'/' || b == b'_' {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

async fn get(e: &Env, path: &str) -> common::RawReply {
    raw(
        &e.sock,
        &e.admin_token,
        "GET",
        &format!("/files/content?path={path}"),
        None,
        b"",
    )
    .await
}

async fn put(e: &Env, path: &str, if_match: Option<&str>, body: &[u8]) -> common::RawReply {
    let extra: Vec<(&str, &str)> = if_match.map(|v| ("If-Match", v)).into_iter().collect();
    raw_hdr(
        &e.sock,
        &e.admin_token,
        "PUT",
        &format!("/files/content?path={path}"),
        Some("text/plain"),
        &extra,
        body,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn files_list_and_read_with_etag() {
    let e = Env::start().await;
    let all = schemas();
    let root = agentos(&e);
    fs::write(root.join("AGENT.md"), "I am an assistant").unwrap();
    fs::create_dir_all(root.join("memory/sub")).unwrap();
    fs::write(root.join("memory/a.md"), "fact one").unwrap();
    fs::write(root.join("memory/sub/b.txt"), "fact two").unwrap();
    fs::write(root.join("keys.env"), "KEY=secretvalue").unwrap();
    fs::write(root.join("memory/skip.bin"), "x").unwrap();
    let (st, list) = e.admin.get("/files").await;
    assert_eq!(st, 200, "{list}");
    let items = list.as_array().unwrap();
    items.iter().for_each(|i| check("FileInfo", i, &all));
    let paths: Vec<&str> = items.iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        ["AGENT.md", "memory/a.md", "memory/sub/b.txt"],
        "only the allowed entries: {list}"
    );
    assert_eq!(items[0]["kind"], "prompt");
    assert_eq!(items[1]["kind"], "memory");
    let r = get(&e, "memory/a.md").await;
    assert_eq!(r.status, 200);
    assert_eq!(String::from_utf8_lossy(&r.body), "fact one");
    assert_eq!(
        r.header("etag").unwrap(),
        format!("\"{}\"", items[1]["rev"].as_str().unwrap())
    );
    assert_eq!(get(&e, "memory/missing.md").await.status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn files_put_requires_if_match_and_writes_atomically() {
    let e = Env::start().await;
    let root = agentos(&e);
    fs::write(root.join("AGENT.md"), "old").unwrap();
    let cur = get(&e, "AGENT.md")
        .await
        .header("etag")
        .unwrap()
        .to_string();
    assert_eq!(
        put(&e, "AGENT.md", None, "new".as_bytes()).await.status,
        428
    );
    assert_eq!(
        put(&e, "AGENT.md", Some("\"ffff\""), "new".as_bytes())
            .await
            .status,
        412
    );
    assert_eq!(
        fs::read_to_string(root.join("AGENT.md")).unwrap(),
        "old",
        "the file is untouched before the check"
    );
    let ok = put(&e, "AGENT.md", Some(&cur), "new".as_bytes()).await;
    assert_eq!(ok.status, 200, "{}", String::from_utf8_lossy(&ok.body));
    assert_eq!(fs::read_to_string(root.join("AGENT.md")).unwrap(), "new");
    let info = ok.json();
    assert_eq!(
        (info["path"].as_str(), info["kind"].as_str()),
        (Some("AGENT.md"), Some("prompt"))
    );
    assert_ne!(ok.header("etag").unwrap(), cur, "ETag changed");
    assert_eq!(
        put(&e, "AGENT.md", Some(&cur), b"x").await.status,
        412,
        "the old ETag is no longer valid"
    );
    let mode = fs::metadata(root.join("AGENT.md"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);

    // New file: If-Match "0" (no file yet), subdirectories are created.
    let new = put(
        &e,
        "memory/pantry/note.md",
        Some("\"0\""),
        "note".as_bytes(),
    )
    .await;
    assert_eq!(new.status, 200, "{}", String::from_utf8_lossy(&new.body));
    assert_eq!(
        fs::read_to_string(root.join("memory/pantry/note.md")).unwrap(),
        "note"
    );
    assert_eq!(
        put(&e, "memory/pantry/note.md", Some("\"0\""), b"x")
            .await
            .status,
        412,
        "the file already exists"
    );
    let leftovers: Vec<_> = fs::read_dir(root.join("memory/pantry"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(leftovers.len(), 1, "no temporary files left");
}

#[tokio::test(flavor = "multi_thread")]
async fn files_path_escape_is_denied() {
    let e = Env::start().await;
    let root = agentos(&e);
    fs::write(root.join("keys.env"), "KEY=secretvalue").unwrap();
    fs::write(root.join("sessions.json"), "{}").unwrap();
    for p in [
        "../keys.env",
        "%2e%2e/keys.env",
        "memory/../keys.env",
        "memory/%2e%2e/keys.env",
        "memory//..//keys.env",
        "memory/../../.agentos/keys.env",
        "/etc/passwd",
        "keys.env",
        "sessions.json",
        ".agentos/keys.env",
        "agent.md",
        "Agent.md",
        "Memory/a.md",
        "AGENT.md/..",
        "AGENT.md/x",
        "memory",
        "memory/",
        "memory/a.exe",
        "memory/%00.md",
        "memory/a.md%00.png",
        "C:/windows",
        "..",
        "",
    ] {
        let q = if p.contains('%') {
            p.to_string()
        } else {
            enc(p)
        };
        let r = get(&e, &q).await;
        assert!(
            r.status == 403 || r.status == 422 || r.status == 400,
            "GET {p:?} → {}",
            r.status
        );
        assert!(
            !String::from_utf8_lossy(&r.body).contains("secretvalue"),
            "{p:?}: leak"
        );
        let w = put(&e, &q, Some("\"0\""), b"x").await;
        assert!(
            w.status == 403 || w.status == 422 || w.status == 400,
            "PUT {p:?} → {}",
            w.status
        );
    }
    assert_eq!(
        fs::read_to_string(root.join("keys.env")).unwrap(),
        "KEY=secretvalue"
    );
    assert!(!e.home.join("x").exists() && !root.join("x").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn files_symlinks_never_leave_the_roots() {
    use std::os::unix::fs::symlink;
    let e = Env::start().await;
    let root = agentos(&e);
    let outside = e.dir.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.md"), "outside the roots").unwrap();
    symlink(outside.join("secret.md"), root.join("memory/link.md")).unwrap();
    symlink(&outside, root.join("memory/dir")).unwrap();
    symlink(outside.join("secret.md"), root.join("AGENT.md")).unwrap();
    for p in ["memory/link.md", "memory/dir/secret.md", "AGENT.md"] {
        assert_eq!(get(&e, p).await.status, 403, "GET {p}");
        assert_eq!(
            put(&e, p, Some("\"0\""), "write".as_bytes()).await.status,
            403,
            "PUT {p}"
        );
    }
    assert_eq!(
        put(&e, "memory/dir/new.md", Some("\"0\""), "write".as_bytes())
            .await
            .status,
        403
    );
    assert_eq!(
        fs::read_to_string(outside.join("secret.md")).unwrap(),
        "outside the roots",
        "symlink target unchanged"
    );
    assert!(!outside.join("new.md").exists());
    let (_, list) = e.admin.get("/files").await;
    assert!(
        list.as_array().unwrap().is_empty(),
        "symlinks are not listed: {list}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn files_admin_only_and_size_limit() {
    let e = Env::start().await;
    agentos(&e);
    for api in [&e.read, &e.chat] {
        assert_eq!(api.get("/files").await.0, 403);
    }
    let r = raw(
        &e.sock,
        &e.chat_token,
        "GET",
        "/files/content?path=AGENT.md",
        None,
        b"",
    )
    .await;
    assert_eq!(r.status, 403);
    let big = vec![b'a'; 256 * 1024 + 1];
    assert_eq!(
        put(&e, "memory/big.md", Some("\"0\""), &big).await.status,
        413
    );
    assert!(!e.home.join(".agentos/memory/big.md").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn files_put_is_recorded_in_action_log() {
    let mut e = Env::start().await;
    let root = agentos(&e);
    fs::write(root.join("AGENT.md"), "old").unwrap();
    let cur = get(&e, "AGENT.md")
        .await
        .header("etag")
        .unwrap()
        .to_string();
    assert_eq!(
        put(&e, "AGENT.md", Some(&cur), "new".as_bytes())
            .await
            .status,
        200
    );
    assert_eq!(
        put(&e, "memory/n.md", Some("\"0\""), "note".as_bytes())
            .await
            .status,
        200
    );
    assert_eq!(
        put(&e, "AGENT.md", Some("\"ffff\""), b"x").await.status,
        412
    );
    let rows: Vec<(String, String, String, Option<String>)> = e
        .inspect(|c| {
            c.prepare("SELECT action, actor, target, args_json FROM actions_log WHERE action IN ('config_write','memory_write') ORDER BY id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect()
        })
        .await;
    let seen: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|r| (r.0.as_str(), r.1.as_str(), r.2.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            ("config_write", "user", "file:AGENT.md"),
            ("memory_write", "user", "file:memory/n.md")
        ],
        "two successful writes in the log, the 412 refusal is not logged"
    );
    assert!(
        rows.iter()
            .all(|r| !r.3.clone().unwrap_or_default().contains("new")),
        "file text does not reach the log"
    );
}
