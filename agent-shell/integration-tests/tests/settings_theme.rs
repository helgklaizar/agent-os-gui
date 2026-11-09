//! `GET`/`PATCH /v1/settings/theme`: `If-Match`, `412`, `428` (S1-06, D38).

mod common;

use common::{http, issue, quiet, send};
use core_client::ClientError;
use core_client::api::{ErrorCode, Theme};
use core_client::http::Request;
use fake_agentd::{Kind, Scope};
use serde_json::json;

#[tokio::test]
async fn settings_theme_if_match_412_428() {
    let d = quiet().await;
    let c = http(&d, Some(&issue(&d, Scope::Chat, Kind::Test)));
    let page = c.theme().await;
    assert!(
        matches!(&page, Ok(p) if p.rev == 1 && p.theme == Theme::Auto),
        "initial theme: {page:?}"
    );
    let first = send(&c, Request::get("/v1/settings/theme")).await;
    assert_eq!(
        first.header("etag").map(|e| e.trim_matches('"')),
        Some("1"),
        "ETag = rev"
    );

    let no_header = c
        .call(Request::patch(
            "/v1/settings/theme",
            json!({ "theme": "dark" }),
        ))
        .await;
    assert!(
        matches!(&no_header, Err(ClientError::Api(e)) if e.status == 428 && e.code == ErrorCode::PreconditionRequired),
        "{no_header:?}"
    );
    let stale = c.set_theme(7, Theme::Dark).await;
    assert!(
        matches!(&stale, Err(ClientError::Api(e)) if e.status == 412 && e.code == ErrorCode::PreconditionFailed),
        "{stale:?}"
    );
    assert!(
        matches!(c.theme().await, Ok(p) if p.rev == 1 && p.theme == Theme::Auto),
        "a refusal does not change state"
    );

    let ok = c.set_theme(1, Theme::Dark).await;
    assert!(
        matches!(&ok, Ok(p) if p.rev == 2 && p.theme == Theme::Dark),
        "fresh version accepted, rev grows: {ok:?}"
    );
    let again = c.set_theme(1, Theme::Light).await;
    assert!(
        matches!(&again, Err(ClientError::Api(e)) if e.status == 412),
        "old rev after a write: {again:?}"
    );
    assert!(matches!(c.set_theme(2, Theme::Light).await, Ok(p) if p.rev == 3));

    let bad_value = c
        .call(
            Request::patch("/v1/settings/theme", json!({ "theme": "neon" }))
                .header("If-Match", "3"),
        )
        .await;
    assert!(
        matches!(&bad_value, Err(ClientError::Api(e)) if e.status == 422 && e.code == ErrorCode::ValidationFailed),
        "{bad_value:?}"
    );
    let garbage = c
        .call(
            Request::patch("/v1/settings/theme", json!({ "theme": "dark" }))
                .header("If-Match", "abc"),
        )
        .await;
    assert!(
        matches!(&garbage, Err(ClientError::Api(e)) if e.status == 400),
        "unintelligible If-Match: {garbage:?}"
    );
}
