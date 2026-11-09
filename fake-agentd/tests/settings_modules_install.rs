//! Installing modules by manifest and by address, permissions (`POST /v1/modules`).

mod s7_common;

use fake_agentd::{Kind, Scope};
use s7_common::*;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn install_by_manifest_validates_like_core() {
    let r = rig().await;
    let ok = r
        .call(
            "POST",
            "/v1/modules",
            Some(json!({"manifest": manifest("my-mod")})),
        )
        .await;
    assert_eq!(ok.status, 201, "{}", text(&ok));
    let mut cases: Vec<(Value, &str)> = Vec::new();
    let mut m = manifest("a1");
    m["placement"] = json!("sidebar");
    cases.push((m, "placement"));
    let mut m = manifest("a2");
    m["surprise"] = json!(true);
    cases.push((m, "surprise"));
    let mut m = manifest("a3");
    m.as_object_mut().map(|o| o.remove("id"));
    cases.push((m, "id"));
    let mut m = manifest("Bad_ID");
    m["id"] = json!("Bad_ID");
    cases.push((m, "id"));
    let mut m = manifest("a4");
    m["entry"] = json!("../x.html");
    cases.push((m, "entry"));
    let mut m = manifest("a5");
    m["permissions"] = json!(["GET /secrets"]);
    cases.push((m, "permissions[0]"));
    let mut m = manifest("a6");
    m["network"] = json!(["https://x.y"]);
    cases.push((m, "network[0]"));
    let mut m = manifest("a7");
    m["fs"] = json!(["/etc"]);
    cases.push((m, "fs[0]"));
    for (m, field) in cases {
        let w = r
            .call("POST", "/v1/modules", Some(json!({"manifest": m})))
            .await;
        assert_eq!(w.status, 422, "{m}: {}", text(&w));
        assert!(
            fields(&w).contains(&field.to_string()),
            "{field}: {:?}",
            fields(&w)
        );
    }
    let both = r
        .call(
            "POST",
            "/v1/modules",
            Some(json!({"manifest": manifest("x1"), "catalog_id": "quick-btn"})),
        )
        .await;
    assert_eq!(both.status, 422, "exactly one of three");
    let none = r.call("POST", "/v1/modules", Some(json!({}))).await;
    assert_eq!(none.status, 422);
    let installed = r.get("/v1/modules").await;
    let n = installed
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["installed"] == true)
        .count();
    assert_eq!(n, 1, "after the refusals only one is installed");
}

#[tokio::test(flavor = "multi_thread")]
async fn install_by_url_goes_through_observer_check() {
    let r = rig().await;
    let url = |u: &str| json!({ "url": u });
    let ok = r
        .call(
            "POST",
            "/v1/modules",
            Some(url("https://modules.example.test/notes-panel.json")),
        )
        .await;
    assert_eq!(ok.status, 201, "{}", text(&ok));
    assert_eq!(ok.json()["id"], "notes-panel");
    for (u, want) in [
        ("https://modules.example.test/evil-widget.json", 422),
        ("http://modules.example.test/quick-btn.json", 422),
        ("https://user:pw@modules.example.test/quick-btn.json", 422),
        ("https://other.example.test/quick-btn.json", 422),
        ("ftp://modules.example.test/quick-btn.json", 422),
        ("https://modules.example.test/missing.json", 422),
        ("https://modules.example.test/quick-btn.json?x=evil", 422),
    ] {
        let w = r.call("POST", "/v1/modules", Some(url(u))).await;
        assert_eq!(w.status, want, "{u}: {}", text(&w));
        assert_eq!(fields(&w), vec!["url".to_string()], "{u}");
        if u.contains("user:pw") {
            assert!(
                w.json()["error"]["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("login")),
                "{}",
                text(&w)
            );
        }
        if u.contains("evil") {
            assert!(
                w.json()["error"]["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("watcher")),
                "{}",
                text(&w)
            );
        }
    }
    let ok_q = r
        .call(
            "POST",
            "/v1/modules",
            Some(url(
                "https://modules.example.test/weather-card.json?v=2#top",
            )),
        )
        .await;
    assert_eq!(
        ok_q.status,
        201,
        "the query and fragment do not interfere: {}",
        text(&ok_q)
    );
    let ask = r
        .call(
            "POST",
            "/v1/modules",
            Some(url("https://modules.example.test/ask-quick-btn.json")),
        )
        .await;
    assert_eq!(ask.status, 202, "{}", text(&ask));
    let b = ask.json();
    assert_eq!(
        (b["status"].as_str(), b["kind"].as_str()),
        (Some("pending"), Some("module"))
    );
    assert!(b["id"].is_number() && b["detail"].is_string());
    let all = r.get("/v1/modules").await;
    let qb = all
        .as_array()
        .and_then(|l| l.iter().find(|m| m["id"] == "quick-btn"))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(
        qb["installed"],
        json!(false),
        "before the watcher decides the module is not installed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn modules_scopes() {
    let r = rig().await;
    let chat = r.token(Scope::Chat, Kind::External);
    let read = r.call_as(&chat, "GET", "/v1/modules", None).await;
    assert_eq!(read.status, 200);
    for (m, p, b) in [
        (
            "POST",
            "/v1/modules",
            Some(json!({"catalog_id": "quick-btn"})),
        ),
        (
            "PATCH",
            "/v1/modules/quick-btn",
            Some(json!({"enabled": true})),
        ),
        ("DELETE", "/v1/modules/quick-btn", None),
    ] {
        let w = r.call_as(&chat, m, p, b).await;
        assert_eq!(
            (w.status, code(&w)),
            (403, "insufficient_scope".into()),
            "{m} {p}"
        );
    }
}
