//! Personas through the API (`POST/PATCH/DELETE /personas`), cascades to references and priority.

mod s7_common;

use fake_agentd::{Kind, Scope};
use s7_common::*;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn persona_crud_and_validation() {
    let r = rig().await;
    let a1 = new_account(&r, "anthropic", "Claude").await;
    let a2 = new_account(&r, "ollama", "Local").await;
    let p = new_persona(
        &r,
        json!({"name": "Work", "color": "#112233", "lang": "en", "mem_mode": "shared",
        "notif_mode": "digest", "signature": "â W", "style": "brief", "model_acc": a1,
        "work_hours": "09:00-18:00", "quiet_hours": "22:00-07:00", "account_ids": [a1, a2]}),
    )
    .await;
    let id = p["id"].as_i64().unwrap_or(-1);
    assert_eq!(
        (
            p["lang"].as_str(),
            p["mem_mode"].as_str(),
            p["notif_mode"].as_str()
        ),
        (Some("en"), Some("shared"), Some("digest"))
    );
    assert_eq!(p["account_ids"], json!([a1, a2]));
    let listed = r.get("/v1/personas").await;
    assert!(
        listed
            .as_array()
            .is_some_and(|l| l.iter().any(|x| x["id"] == id && x["name"] == "Work"))
    );
    let patched = r
        .call(
            "PATCH",
            &format!("/v1/personas/{id}"),
            Some(json!({"name": "Home", "account_ids": [a2], "model_acc": null})),
        )
        .await;
    assert_eq!(patched.status, 200, "{}", text(&patched));
    assert_eq!(patched.json()["name"], "Home");
    assert_eq!(patched.json()["account_ids"], json!([a2]));
    assert_eq!(patched.json()["model_acc"], Value::Null);
    assert_eq!(patched.json()["lang"], "en", "the untouched one stayed");
    for (body, field) in [
        (json!({"name": ""}), "name"),
        (json!({"name": "x".repeat(81)}), "name"),
        (json!({"name": "a", "lang": "de"}), "lang"),
        (json!({"name": "a", "mem_mode": "x"}), "mem_mode"),
        (json!({"name": "a", "notif_mode": "x"}), "notif_mode"),
        (json!({"name": "a", "account_ids": [9999]}), "account_ids"),
    ] {
        let w = r.call("POST", "/v1/personas", Some(body.clone())).await;
        assert_eq!(w.status, 422, "{body}: {}", text(&w));
        assert!(
            fields(&w).contains(&field.to_string()),
            "{body}: {:?}",
            fields(&w)
        );
    }
    let missing = r
        .call("PATCH", "/v1/personas/9999", Some(json!({"name": "a"})))
        .await;
    assert_eq!(missing.status, 404);
    let chat = r.token(Scope::Chat, Kind::External);
    let denied = r
        .call_as(&chat, "POST", "/v1/personas", Some(json!({"name": "z"})))
        .await;
    assert_eq!(denied.status, 403);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_persona_clears_references_and_priority() {
    let r = rig().await;
    let keep = new_persona(&r, json!({"name": "Stays"})).await["id"]
        .as_i64()
        .unwrap_or(-1);
    let gone = new_persona(&r, json!({"name": "Leaves"})).await["id"]
        .as_i64()
        .unwrap_or(-1);
    let acc = new_account(&r, "tg", "tg").await;
    r.call(
        "PATCH",
        &format!("/v1/personas/{gone}"),
        Some(json!({"account_ids": [acc]})),
    )
    .await;
    let folder = r
        .call(
            "POST",
            "/v1/folders",
            Some(json!({"name": "F", "persona_id": gone})),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap_or(-1);
    let session = r
        .call(
            "POST",
            "/v1/sessions",
            Some(json!({"name": "S", "persona_id": gone})),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap_or(-1);
    let w = r
        .patch_page(
            "personas",
            json!({"balance": {"priority_persona_id": gone}}),
        )
        .await;
    assert_eq!(w.status, 200, "{}", text(&w));
    // deleting does not touch another persona's priority
    let before = r.get("/v1/settings/personas").await;
    let del_keep_rev = before["rev"].as_i64().unwrap_or(-1);
    let del = r
        .call("DELETE", &format!("/v1/personas/{gone}"), None)
        .await;
    assert_eq!(del.status, 204);
    let page = r.get("/v1/settings/personas").await;
    assert_eq!(
        page["balance"]["priority_persona_id"],
        Value::Null,
        "priority reset"
    );
    assert_eq!(page["rev"], del_keep_rev + 1, "rev grew");
    assert_eq!(
        page["balance"]["fallback"], "local",
        "the rest of the balance is intact"
    );
    let conn = r.d.db().reader().expect("reader");
    let n = |sql: &str| -> i64 { conn.query_row(sql, [], |x| x.get(0)).unwrap_or(-1) };
    assert_eq!(
        n(&format!(
            "SELECT count(*) FROM persona_accounts WHERE persona_id = {gone}"
        )),
        0
    );
    assert_eq!(
        n(&format!(
            "SELECT count(*) FROM folders WHERE id = {folder} AND persona_id IS NULL"
        )),
        1
    );
    assert_eq!(
        n(&format!(
            "SELECT count(*) FROM sessions WHERE id = {session} AND persona_id IS NULL"
        )),
        1
    );
    assert_eq!(
        n("SELECT count(*) FROM accounts WHERE slug <> ''"),
        n("SELECT count(*) FROM accounts"),
        "accounts intact"
    );
    let list = r.get("/v1/personas").await;
    assert!(list.as_array().is_some_and(|l| l.iter().all(|x| x["id"] != gone) && l.iter().any(|x| x["id"] == keep)));
    assert_eq!(
        r.call("DELETE", &format!("/v1/personas/{gone}"), None)
            .await
            .status,
        404
    );
    // the priority of the remaining persona is not reset when another is deleted
    r.patch_page(
        "personas",
        json!({"balance": {"priority_persona_id": keep}}),
    )
    .await;
    let other = new_persona(&r, json!({"name": "Third"})).await["id"]
        .as_i64()
        .unwrap_or(-1);
    r.call("DELETE", &format!("/v1/personas/{other}"), None)
        .await;
    assert_eq!(
        r.get("/v1/settings/personas").await["balance"]["priority_persona_id"],
        keep
    );
}
