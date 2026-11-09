//! A6-05: tunnels: `GET/POST /tunnels`, `PATCH/DELETE /tunnels/{id}` (`admin` only), field validation without secrets,
//! computed `status`, external access off by default.

mod common;
mod openapi;

use common::{Env, fields};
use openapi::{check, schemas};
use serde_json::{Value, json};

fn wg() -> Value {
    json!({"type": "wireguard", "addr": "vpn.example.test:51820", "descr": "home"})
}

#[tokio::test(flavor = "multi_thread")]
async fn tunnel_crud_status_and_contract() {
    let e = Env::start().await;
    let all = schemas();
    let (st, t) = e.admin.post("/tunnels", wg()).await;
    assert_eq!(st, 201, "{t}");
    check("Tunnel", &t, &all);
    assert_eq!(
        (t["enabled"].as_bool(), t["type"].as_str()),
        (Some(false), Some("wireguard"))
    );
    let id = t["id"].as_i64().unwrap();
    let ts = e
        .admin
        .make(
            "/tunnels",
            json!({"type": "tailscale", "addr": "tailnet.example.test"}),
        )
        .await;
    assert_eq!(
        ts["status"], "external",
        "tailscale is not managed by the core"
    );
    let (st, list) = e.admin.get("/tunnels").await;
    assert_eq!((st, list.as_array().unwrap().len()), (200, 2));
    list.as_array()
        .unwrap()
        .iter()
        .for_each(|x| check("Tunnel", x, &all));
    let (st, p) = e
        .admin
        .patch(
            &format!("/tunnels/{id}"),
            json!({"descr": "work", "addr": "vpn2.example.test:51820"}),
        )
        .await;
    assert_eq!(
        (st, p["descr"].as_str(), p["addr"].as_str()),
        (200, Some("work"), Some("vpn2.example.test:51820")),
        "{p}"
    );
    assert_eq!(
        e.admin.patch("/tunnels/999", json!({"descr": "x"})).await.0,
        404
    );
    assert_eq!(e.admin.delete(&format!("/tunnels/{id}")).await.0, 204);
    assert_eq!(e.admin.delete(&format!("/tunnels/{id}")).await.0, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn tunnel_validation_rejects_bad_type_empty_addr_and_secrets() {
    let e = Env::start().await;
    let cases = [
        (json!({"type": "openvpn", "addr": "h"}), "type"),
        (json!({"type": "ssh", "addr": "  "}), "addr"),
        (json!({"type": "ssh"}), "addr"),
        (
            json!({"type": "ssh", "addr": "user:hunter2@host.example.test"}),
            "addr",
        ),
        (
            json!({"type": "ngrok", "addr": "https://h/?token=abcdef"}),
            "addr",
        ),
        (
            json!({"type": "wireguard", "addr": "h", "descr": "password: qwerty"}),
            "descr",
        ),
        (
            json!({"type": "ssh", "addr": "h", "enabled": "yes"}),
            "enabled",
        ),
        (json!({"type": "ssh", "addr": "h", "zzz": 1}), "zzz"),
        (json!({"type": "ssh", "addr": "-oProxyCommand=x"}), "addr"),
    ];
    for (body, field) in cases {
        let (st, v) = e.admin.post("/tunnels", body.clone()).await;
        assert_eq!(st, 422, "{body}: {v}");
        assert!(fields(&v).contains(&field.to_string()), "{field}: {v}");
    }
    let (_, list) = e.admin.get("/tunnels").await;
    assert!(list.as_array().unwrap().is_empty(), "nothing created");
}

#[tokio::test(flavor = "multi_thread")]
async fn tunnel_admin_only_and_external_access_off_by_default() {
    let e = Env::start().await;
    for api in [&e.read, &e.chat] {
        assert_eq!(api.get("/tunnels").await.0, 403);
        assert_eq!(api.post("/tunnels", wg()).await.0, 403);
    }
    let (_, page) = e.admin.get("/settings/tunnels").await;
    assert_eq!(
        page["api_external"], false,
        "external access is off by default"
    );
}
