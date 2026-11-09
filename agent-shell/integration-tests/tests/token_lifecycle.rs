//! Token lifecycle in `fake-agentd`: only the hash is stored, expiry, kinds and limits, allowlist, a GUI token without a file.

mod common;

use std::time::Duration;

use common::{error_of, http, quiet, send, start};
use core_client::ClientError;
use core_client::api::{ErrorCode, Scope as ApiScope, TokenCreate, TokenKind};
use core_client::http::Request;
use fake_agentd::{Config, Kind, Scope, TokenSpec};
use serde_json::json;

#[tokio::test]
async fn api_token_stores_hash_only() {
    let d = quiet().await;
    let admin = http(&d, Some(d.admin_token()));
    let issued = admin
        .issue_token(&TokenCreate {
            name: "phone".into(),
            scope: ApiScope::Chat,
            ttl_days: Some(30),
            kind: None,
            allowlist: None,
        })
        .await;
    let Ok(issued) = issued else {
        panic!("token not issued: {issued:?}")
    };
    assert!(
        issued.token.starts_with("agt_") && issued.token.len() == 47,
        "agent format: agt_ + 43 characters"
    );
    assert_eq!(issued.info.kind, TokenKind::External, "external by default");
    assert!(
        issued.info.expires_at.is_some_and(|e| e > d.now_unix()),
        "30-day expiry"
    );

    let hashes = d.stored_token_hashes();
    assert!(
        hashes.len() >= 2,
        "admin and the issued one: {}",
        hashes.len()
    );
    let expect = common::sha256_hex(&issued.token);
    assert!(
        hashes.contains(&expect),
        "the SHA-256 of the issued token is stored"
    );
    for h in &hashes {
        assert!(
            h.len() == 64
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "64 hex only: {h}"
        );
        assert!(
            !h.contains(&issued.token) && !h.contains(d.admin_token()),
            "no raw value"
        );
    }
    let listing = admin
        .call(Request::get("/v1/tokens"))
        .await
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .unwrap_or_default();
    assert!(
        !listing.contains(&issued.token) && !listing.contains(&expect),
        "GET /tokens returns neither the value nor the hash"
    );
    let shown = format!("{:?} {:?}", d.requests(), issued);
    assert!(
        !shown.contains(&issued.token) && !shown.contains(d.admin_token()),
        "the value is in neither the request records nor Debug"
    );
    // the issued token works and is external
    let ext = http(&d, Some(&issued.token));
    assert!(ext.theme().await.is_ok());
    assert!(
        matches!(ext.list_tokens().await, Err(ClientError::Api(e)) if e.status == 403),
        "external is below admin"
    );
}

#[tokio::test]
async fn token_expires() {
    let d = start(Config::temp().no_ticker().manual_clock()).await;
    let issued =
        d.issue(TokenSpec::new(Scope::Chat, Kind::External).ttl(Duration::from_secs(24 * 3600)));
    let Ok(issued) = issued else {
        panic!("{issued:?}")
    };
    let c = http(&d, Some(&issued.token));
    let before = d.stored_token_hashes().len();
    assert_eq!(
        send(&c, Request::get("/v1/settings/theme")).await.status,
        200,
        "works before expiry"
    );
    d.advance(Duration::from_secs(24 * 3600 - 1));
    assert_eq!(
        send(&c, Request::get("/v1/settings/theme")).await.status,
        200,
        "works one second before expiry"
    );
    d.advance(Duration::from_secs(2));
    let r = send(&c, Request::get("/v1/settings/theme")).await;
    assert_eq!(
        (r.status, error_of(&r).0.as_str()),
        (401, "unauthorized"),
        "after expiry"
    );
    assert_eq!(
        d.stored_token_hashes().len(),
        before,
        "the row is not deleted"
    );
    assert!(
        d.stored_token_hashes()
            .contains(&common::sha256_hex(&issued.token))
    );
}

#[tokio::test]
async fn external_not_admin() {
    let d = quiet().await;
    let admin = http(&d, Some(d.admin_token()));
    let create = |scope: ApiScope,
                  kind: Option<TokenKind>,
                  allow: Option<Vec<String>>,
                  ttl: Option<u32>| TokenCreate {
        name: "t".into(),
        scope,
        ttl_days: ttl,
        kind,
        allowlist: allow,
    };
    let list = || Some(vec!["GET /settings/theme".to_string()]);
    let rejected = [
        (
            "external admin",
            create(ApiScope::Admin, Some(TokenKind::External), None, None),
        ),
        (
            "external, admin by default",
            create(ApiScope::Admin, None, None, None),
        ),
        (
            "agent admin",
            create(ApiScope::Admin, Some(TokenKind::Agent), list(), None),
        ),
        (
            "module admin",
            create(ApiScope::Admin, Some(TokenKind::Module), list(), None),
        ),
        (
            "agent without allowlist",
            create(ApiScope::Chat, Some(TokenKind::Agent), None, None),
        ),
        (
            "module without allowlist",
            create(ApiScope::Read, Some(TokenKind::Module), None, None),
        ),
        ("ttl above 90", create(ApiScope::Chat, None, None, Some(91))),
        ("ttl zero", create(ApiScope::Chat, None, None, Some(0))),
        (
            "empty name",
            TokenCreate {
                name: String::new(),
                ..create(ApiScope::Chat, None, None, None)
            },
        ),
        (
            "name longer than 100",
            TokenCreate {
                name: "x".repeat(101),
                ..create(ApiScope::Chat, None, None, None)
            },
        ),
    ];
    let before = d.stored_token_hashes().len();
    for (name, body) in rejected {
        let r = admin.issue_token(&body).await;
        assert!(
            matches!(&r, Err(ClientError::Api(e)) if e.status == 422 && e.code == ErrorCode::ValidationFailed),
            "{name}: {r:?}"
        );
    }
    assert_eq!(
        d.stored_token_hashes().len(),
        before,
        "rejected tokens are not stored"
    );
    let accepted = [
        create(ApiScope::Chat, Some(TokenKind::External), None, Some(90)),
        create(ApiScope::Admin, Some(TokenKind::Gui), None, None),
        create(ApiScope::Admin, Some(TokenKind::Test), None, None),
        create(ApiScope::Chat, Some(TokenKind::Agent), list(), None),
        create(ApiScope::Read, Some(TokenKind::Module), list(), None),
    ];
    for body in accepted {
        let r = admin.issue_token(&body).await;
        assert!(r.is_ok(), "{:?}: {r:?}", body.kind);
    }
    assert_eq!(d.stored_token_hashes().len(), before + 5);
    // the same prohibition inside the store, without HTTP
    assert!(
        d.issue(TokenSpec::new(Scope::Admin, Kind::External))
            .is_err()
    );
    assert!(
        d.issue(TokenSpec::new(Scope::Admin, Kind::Module).allowlist(&["GET /x"]))
            .is_err()
    );
    assert!(d.issue(TokenSpec::new(Scope::Chat, Kind::Agent)).is_err());
}

#[tokio::test]
async fn token_kind_allowlist() {
    let d = quiet().await;
    let module =
        d.issue(TokenSpec::new(Scope::Chat, Kind::Module).allowlist(&["GET /settings/theme"]));
    let Ok(module) = module else {
        panic!("{module:?}")
    };
    let c = http(&d, Some(&module.token));
    assert_eq!(
        send(&c, Request::get("/v1/settings/theme")).await.status,
        200,
        "method from the allowlist"
    );
    let r = send(
        &c,
        Request::patch("/v1/settings/theme", json!({ "theme": "dark" })).header("If-Match", "1"),
    )
    .await;
    assert_eq!(
        (r.status, error_of(&r).0.as_str()),
        (403, "forbidden_method"),
        "method outside the allowlist with sufficient scope"
    );
    let r = send(&c, Request::get("/v1/tokens")).await;
    assert_eq!(
        (r.status, error_of(&r).0.as_str()),
        (403, "insufficient_scope"),
        "insufficient scope gives another code"
    );
    assert!(
        matches!(http(&d, Some(d.admin_token())).theme().await, Ok(p) if p.rev == 1),
        "the refusal did not change the theme"
    );
    // agent token: the same rule
    let agent = d.issue(TokenSpec::new(Scope::Chat, Kind::Agent).allowlist(&["POST /_test/echo"]));
    let Ok(agent) = agent else {
        panic!("{agent:?}")
    };
    let a = http(&d, Some(&agent.token));
    assert!(
        a.echo("c-allow", "x").await.is_ok(),
        "echo from the allowlist"
    );
    let r = send(&a, Request::get("/v1/settings/theme")).await;
    assert_eq!(
        (r.status, error_of(&r).0.as_str()),
        (403, "forbidden_method")
    );
}
