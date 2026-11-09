//! Secrets are write-only, `admin` rights, no key rotation, client routes exist in OpenAPI.

mod settings_common;

use controllers::SettingsPage;
use controllers::settings_types::Secret;
use controllers::settings_view::SettingsMsg;
use core_client::http::Request;
use core_client::{HttpClient, Token};
use fake_agentd::{Kind, Scope, TokenSpec};
use serde_json::json;
use settings_common::*;

const FIXTURE_SECRET: &str = "FIXTURE-SECRET-VALUE-DO-NOT-LEAK";

#[tokio::test(flavor = "multi_thread")]
async fn secrets_are_write_only() {
    let mut r = Rig::start().await;
    r.http
        .put_secret("account.1", FIXTURE_SECRET)
        .await
        .expect("write");
    assert_eq!(
        r.d.secret_value("account.1").as_deref(),
        Some(FIXTURE_SECRET),
        "value written"
    );
    let get = r.http.call(Request::get("/v1/secrets/account.1")).await;
    assert!(
        matches!(status(&get), Some(404 | 405)),
        "GET of a secret does not exist: {get:?}"
    );
    let list = r.http.list_secrets().await.expect("list");
    assert!(
        list.iter()
            .any(|s| s["ref"] == "account.1" && s["set"] == true)
    );
    assert!(!json!(list).to_string().contains(FIXTURE_SECRET));
    // write error bodies do not repeat the value
    for bad in ["nope", "account.x", "provider."] {
        let e = r
            .http
            .call(Request::put(
                &format!("/v1/secrets/{bad}"),
                json!({"value": FIXTURE_SECRET}),
            ))
            .await;
        let body = format!("{e:?}");
        assert!(status(&e).is_some_and(|s| s >= 400), "{bad}: {body}");
        assert!(!body.contains(FIXTURE_SECRET), "{bad}: value in the error");
    }
    // no read response contains the value
    r.open(SettingsPage::Models).await;
    for path in [
        "/v1/settings/models",
        "/v1/settings/connections",
        "/v1/settings/account",
        "/v1/accounts",
        "/v1/personas",
        "/v1/modules",
        "/v1/tunnels",
        "/v1/files",
        "/v1/permissions",
        "/v1/secrets",
        "/v1/quotas",
    ] {
        let resp = r.http.call(Request::get(path)).await.expect(path);
        assert!(
            !String::from_utf8_lossy(&resp.body).contains(FIXTURE_SECRET),
            "{path}"
        );
    }
    // a secret value cannot be put into a page document
    let page = r.http.settings_page("models").await.expect("page");
    let put = r
        .http
        .patch_settings_page(
            "models",
            page.rev,
            &json!({"accounts": {"1": {"api_key": FIXTURE_SECRET}}}),
        )
        .await;
    assert_eq!(status(&put), Some(422), "secret in the document rejected");
    // from the dialog: the field is empty after saving, the value is not in the view or the effects
    r.select(SettingsPage::Models).await;
    r.msg(SettingsMsg::EditSecret {
        id: "s:account.1".into(),
        value: Secret::new(FIXTURE_SECRET),
    })
    .await;
    r.save().await;
    assert!(!r.view().dirty);
    assert_eq!(r.field("s:account.1").map(|f| f.value).as_deref(), Some(""));
    assert!(!format!("{:?}", r.view()).contains(FIXTURE_SECRET));
    assert!(
        !format!("{:?}", r.effects).contains(FIXTURE_SECRET),
        "no value in the effects (GUI logs)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_only_endpoints_reject_chat_token() {
    let r = Rig::start().await;
    let token =
        r.d.issue(TokenSpec::new(Scope::Chat, Kind::Test))
            .expect("token")
            .token;
    let chat = HttpClient::new(
        core_client::Endpoint::Unix(r.d.socket_path().to_path_buf()),
        Some(Token::new(&token)),
    );
    let attempts = [
        Request::put("/v1/permissions/global/0/send", json!({"mode": "deny"})),
        Request::put("/v1/secrets/account.1", json!({"value": "x"})),
        Request::patch("/v1/settings/account", json!({"name": "x"})).header("If-Match", "0"),
        Request::post("/v1/tunnels", json!({"type": "ssh", "addr": "h"})),
        Request::post("/v1/modules", json!({"catalog_id": "weather-card"})),
        Request::post("/v1/personas", json!({"name": "x"})),
    ];
    for req in attempts {
        let what = format!("{} {}", req.method, req.path);
        let resp = chat.call(req).await;
        assert_eq!(status(&resp), Some(403), "{what}: chat token rejected");
    }
    assert_eq!(r.d.secret_value("account.1"), None, "secret not written");
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_no_key_rotation_in_ui() {
    let mut r = Rig::start().await;
    r.open(SettingsPage::Personas).await;
    for page in [
        SettingsPage::Personas,
        SettingsPage::Models,
        SettingsPage::Connections,
        SettingsPage::Security,
    ] {
        r.select(page).await;
        for row in r.view().rows {
            if let controllers::settings_view::Row::Field(f) = row {
                assert!(
                    !f.id.contains("rotate") && !f.label.to_lowercase().contains("rotat"),
                    "{}",
                    f.id
                );
            }
        }
    }
    // the core does not create the field: balance.rotate is an unknown field
    let page = r.http.settings_page("personas").await.expect("page");
    let bad = r
        .http
        .patch_settings_page("personas", page.rev, &json!({"balance": {"rotate": true}}))
        .await;
    assert_eq!(status(&bad), Some(422));
}

/// Routes called by the settings client: method and path per `openapi-draft.yaml`.
const CLIENT_ROUTES: [(&str, &str); 24] = [
    ("get", "/settings/{page}"),
    ("patch", "/settings/{page}"),
    ("put", "/secrets/{ref}"),
    ("get", "/secrets"),
    ("get", "/accounts"),
    ("post", "/accounts"),
    ("delete", "/accounts/{id}"),
    ("post", "/accounts/{id}/connect"),
    ("get", "/personas"),
    ("post", "/personas"),
    ("patch", "/personas/{id}"),
    ("delete", "/personas/{id}"),
    ("get", "/modules"),
    ("post", "/modules"),
    ("patch", "/modules/{module_id}"),
    ("delete", "/modules/{module_id}"),
    ("get", "/tunnels"),
    ("post", "/tunnels"),
    ("patch", "/tunnels/{id}"),
    ("delete", "/tunnels/{id}"),
    ("get", "/files"),
    ("get", "/files/content"),
    ("put", "/files/content"),
    ("put", "/permissions/{scope}/{scope_id}/{action}"),
];

#[test]
fn contract_settings_secrets_match_openapi() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/reference/openapi-draft.yaml");
    let text = std::fs::read_to_string(&path).expect("openapi-draft.yaml");
    let mut missing = Vec::new();
    for (method, route) in CLIENT_ROUTES {
        let head = format!("  {route}:");
        let Some(start) = text.lines().position(|l| l == head) else {
            missing.push(format!("{method} {route}: no such path in OpenAPI"));
            continue;
        };
        let has = text
            .lines()
            .skip(start + 1)
            .take_while(|l| l.starts_with("    ") || l.is_empty())
            .any(|l| l == format!("    {method}:"));
        if !has {
            missing.push(format!("{method} {route}: no such method in OpenAPI"));
        }
    }
    assert!(
        missing.is_empty(),
        "the client calls what is not in the contract: {missing:?}"
    );
    // the secret is writeOnly, there is no value in SecretInfo
    assert!(text.contains("writeOnly: true"));
}
