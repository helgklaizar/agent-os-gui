//! Tunnels (`/tunnels`) and the file editor (`/files`: only `AGENT.md` and `memory/`).

mod s7_common;

use fake_agentd::{Kind, Scope};
use s7_common::*;
use serde_json::{Value, json};

const MARKER: &str = "FIXTURE-SECRET-DO-NOT-LEAK";

#[tokio::test(flavor = "multi_thread")]
async fn tunnel_add_toggle_delete_persist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let r = rig_db(&db).await;
    assert_eq!(r.get("/v1/tunnels").await, json!([]));
    let w = r
        .call(
            "POST",
            "/v1/tunnels",
            Some(json!({"type": "wireguard", "addr": "10.0.0.1:51820", "descr": "<b>home</b>"})),
        )
        .await;
    assert_eq!(w.status, 201, "{}", text(&w));
    let t = w.json();
    let id = t["id"].as_i64().unwrap_or(-1);
    assert_eq!(
        (
            t["type"].as_str(),
            t["enabled"].clone(),
            t["status"].as_str()
        ),
        (Some("wireguard"), json!(false), Some("down"))
    );
    assert_eq!(t["descr"], "<b>home</b>", "text is returned literally");
    let on = r
        .call(
            "PATCH",
            &format!("/v1/tunnels/{id}"),
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!((on.status, on.json()["status"].clone()), (200, json!("up")));
    assert_eq!(on.json()["addr"], "10.0.0.1:51820", "the rest stayed");
    let edit = r
        .call(
            "PATCH",
            &format!("/v1/tunnels/{id}"),
            Some(json!({"addr": "h:2", "descr": "x", "type": "ssh"})),
        )
        .await;
    assert_eq!(edit.json()["type"], "ssh");
    drop(r);
    let r = rig_db(&db).await;
    let list = r.get("/v1/tunnels").await;
    assert_eq!(list[0]["addr"], "h:2", "after a restart");
    assert_eq!(list[0]["enabled"], true);
    assert_eq!(
        r.call("DELETE", &format!("/v1/tunnels/{id}"), None)
            .await
            .status,
        204
    );
    assert_eq!(r.get("/v1/tunnels").await, json!([]));
    assert_eq!(
        r.call("DELETE", &format!("/v1/tunnels/{id}"), None)
            .await
            .status,
        404
    );
    assert_eq!(
        r.call("PATCH", "/v1/tunnels/9", Some(json!({"enabled": true})))
            .await
            .status,
        404
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tunnel_validation_and_scope() {
    let r = rig().await;
    for (body, field) in [
        (json!({"type": "openvpn", "addr": "a"}), "type"),
        (json!({"addr": "a"}), "type"),
        (json!({"type": "ssh"}), "addr"),
        (json!({"type": "ssh", "addr": ""}), "addr"),
        (json!({"type": "ssh", "addr": "x".repeat(256)}), "addr"),
        (
            json!({"type": "ssh", "addr": "a", "descr": "d".repeat(201)}),
            "descr",
        ),
        (
            json!({"type": "ssh", "addr": "a", "enabled": "on"}),
            "enabled",
        ),
    ] {
        let w = r.call("POST", "/v1/tunnels", Some(body.clone())).await;
        assert_eq!(w.status, 422, "{body}: {}", text(&w));
        assert!(
            fields(&w).contains(&field.to_string()),
            "{body}: {:?}",
            fields(&w)
        );
    }
    let chat = r.token(Scope::Chat, Kind::External);
    for (m, p, b) in [
        ("GET", "/v1/tunnels", None),
        (
            "POST",
            "/v1/tunnels",
            Some(json!({"type": "ssh", "addr": "a"})),
        ),
        ("PATCH", "/v1/tunnels/1", Some(json!({"enabled": true}))),
        ("DELETE", "/v1/tunnels/1", None),
    ] {
        let w = r.call_as(&chat, m, p, b).await;
        assert_eq!(
            (w.status, code(&w)),
            (403, "insufficient_scope".into()),
            "{m} {p}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn files_list_only_agentmd_and_memory() {
    let r = rig().await;
    let list = r.get("/v1/files").await;
    let paths: Vec<&str> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f["path"].as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["AGENT.md", "memory/ideas.md", "memory/notes.md"],
        "{list}"
    );
    for f in list.as_array().into_iter().flatten() {
        let kind = if f["path"] == "AGENT.md" {
            "prompt"
        } else {
            "memory"
        };
        assert_eq!(f["kind"], kind);
        assert!(
            f["size"].as_u64().is_some_and(|s| s > 0) && f["rev"].is_string(),
            "{f}"
        );
    }
    assert!(
        !list.to_string().contains("keys.env") && !list.to_string().contains("permissions.json")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn files_read_write_with_if_match() {
    let r = rig().await;
    let got = r
        .call("GET", "/v1/files/content?path=memory/notes.md", None)
        .await;
    assert_eq!(got.status, 200);
    let etag = got.header("etag").map(str::to_string).unwrap_or_default();
    assert!(!etag.is_empty());
    let rev = etag.trim_matches('"').to_string();
    let put = |body: &'static str, if_match: Option<String>| {
        let mut req = Req::new("PUT", "/v1/files/content?path=memory/notes.md")
            .token(&r.admin)
            .raw("text/plain", body.as_bytes().to_vec());
        if let Some(v) = if_match {
            req = req.header("If-Match", &v);
        }
        let sock = r.d.socket_path().to_path_buf();
        async move { send(&sock, req).await }
    };
    assert_eq!(put("x", None).await.status, 428, "without If-Match");
    assert_eq!(put("x", Some("\"stale\"".into())).await.status, 412);
    let ok = put("new note <script>", Some(etag.clone())).await;
    assert_eq!(ok.status, 200, "{}", text(&ok));
    assert_eq!(ok.json()["path"], "memory/notes.md");
    assert_ne!(ok.header("etag"), Some(etag.as_str()), "rev changed");
    let again = r
        .call("GET", "/v1/files/content?path=memory/notes.md", None)
        .await;
    assert_eq!(text(&again), "new note <script>");
    assert!(
        again
            .header("content-type")
            .is_some_and(|c| c.starts_with("text/plain"))
    );
    assert_eq!(
        put("y", Some(format!("\"{rev}\""))).await.status,
        412,
        "stale rev"
    );
    let missing = r
        .call("GET", "/v1/files/content?path=memory/none.md", None)
        .await;
    assert_eq!(missing.status, 404);
    let new_file = Req::new("PUT", "/v1/files/content?path=memory/fresh.md")
        .token(&r.admin)
        .header("If-Match", "\"0\"")
        .raw("text/plain", b"new".to_vec());
    assert_eq!(
        send(r.d.socket_path(), new_file).await.status,
        200,
        "a new file in memory/ is created with If-Match 0"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn files_deny_everything_else_and_never_leak_marker() {
    let r = rig().await;
    let mut all = String::new();
    for p in [
        "keys.env",
        "permissions.json",
        "../keys.env",
        "memory/../keys.env",
        "memory/../../etc/passwd",
        "/etc/passwd",
        "memory",
        "memory/",
        "memory/sub/../../keys.env",
        "AGENT.md/../keys.env",
        "memory/notes.txt",
        "agentos/keys.env",
        "memory/%2e%2e/keys.env",
        "",
    ] {
        for method in ["GET", "PUT"] {
            let path = format!("/v1/files/content?path={p}");
            let mut req = Req::new(method, &path).token(&r.admin);
            if method == "PUT" {
                req = req
                    .header("If-Match", "\"0\"")
                    .raw("text/plain", b"pwn".to_vec());
            }
            let w = send(r.d.socket_path(), req).await;
            assert!(
                matches!(w.status, 403 | 400),
                "{method} {p:?}: {} {}",
                w.status,
                text(&w)
            );
            if !p.is_empty() {
                assert_eq!(w.status, 403, "{method} {p:?}");
                assert_eq!(code(&w), "permission_denied");
            }
            all.push_str(&text(&w));
        }
    }
    let no_path = r.call("GET", "/v1/files/content", None).await;
    assert_eq!(no_path.status, 400);
    all.push_str(&text(&r.call("GET", "/v1/files", None).await));
    all.push_str(&text(
        &r.call("GET", "/v1/files/content?path=AGENT.md", None).await,
    ));
    all.push_str(&text(
        &r.call("GET", "/v1/files/content?path=memory/ideas.md", None)
            .await,
    ));
    assert!(!all.contains(MARKER), "the keys.env decoy did not leak");
    let on_disk = std::fs::read_to_string(r.d.dir().join("agentos/keys.env")).unwrap_or_default();
    assert!(
        on_disk.contains(MARKER),
        "the decoy lies on disk (otherwise the test proves nothing)"
    );
    assert!(
        !std::fs::read_to_string(r.d.dir().join("agentos/permissions.json"))
            .unwrap_or_default()
            .contains("pwn")
    );
    assert!(
        !std::fs::read_to_string(r.d.dir().join("agentos/keys.env"))
            .map(|t| t.contains("pwn"))
            .unwrap_or(true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn files_symlink_escape_is_denied() {
    let r = rig().await;
    let link = r.d.dir().join("agentos/memory/link.md");
    std::os::unix::fs::symlink(r.d.dir().join("agentos/keys.env"), &link).expect("symlink");
    let w = r
        .call("GET", "/v1/files/content?path=memory/link.md", None)
        .await;
    assert_eq!(w.status, 403, "{}", text(&w));
    assert!(!text(&w).contains(MARKER));
    let list = r.get("/v1/files").await;
    assert!(
        list.as_array()
            .into_iter()
            .flatten()
            .all(|f| f["path"] != "memory/link.md")
    );
    let _: Value = list;
}
