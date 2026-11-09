//! Core client calls for the bar: core version for About, Logout, `balance.fallback`.

mod common;

use common::{http, issue, quiet, tcp, unix};
use core_client::blocking::BlockingClient;
use core_client::{ClientError, Token};
use fake_agentd::{Config, Kind, Scope};

fn blocking(d: &fake_agentd::FakeAgentd) -> BlockingClient {
    match BlockingClient::new(unix(d), Token::new(d.admin_token())) {
        Ok(c) => c,
        Err(e) => panic!("client not created: {e}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn blocking_health_returns_core_version() {
    let d = quiet().await;
    let c = blocking(&d);
    let h = tokio::task::spawn_blocking(move || c.health()).await;
    let Ok(Ok(health)) = h else { panic!("{h:?}") };
    assert_eq!(health.version, "0.0.0-fake");
}

#[tokio::test(flavor = "multi_thread")]
async fn blocking_health_exposes_gui_range_and_database_version() {
    let d = quiet().await;
    let c = blocking(&d);
    let h = tokio::task::spawn_blocking(move || c.health()).await;
    let Ok(Ok(health)) = h else { panic!("{h:?}") };
    assert_eq!(health.min_client.as_deref(), Some("0.0.0"));
    assert_eq!(health.min_gui.as_deref(), Some("0.0.0"));
    assert_eq!(health.max_gui.as_deref(), Some("0.x"));
    assert!(health.user_version.is_some_and(|v| v > 0), "{health:?}");
    assert!(health.capabilities.iter().any(|c| c == "if-match"));
    assert_eq!(health.api.minor, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn blocking_logout_posts_and_core_records_it() {
    let d = quiet().await;
    let c = blocking(&d);
    let r = tokio::task::spawn_blocking(move || c.logout()).await;
    assert!(matches!(r, Ok(Ok(()))), "{r:?}");
    let seen = d
        .requests()
        .iter()
        .any(|r| r.method == "POST" && r.path == "/v1/logout" && r.status == 204);
    assert!(
        seen,
        "the core accepted POST /v1/logout: {:?}",
        d.requests()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn logout_denied_without_token_wrong_scope_and_over_tcp() {
    let d = start_tcp().await;
    let anon = http(&d, None).logout().await;
    assert!(
        matches!(&anon, Err(ClientError::Api(e)) if e.status == 401),
        "{anon:?}"
    );
    let reader = http(&d, Some(&issue(&d, Scope::Read, Kind::Test)));
    let r = reader.logout().await;
    assert!(
        matches!(&r, Err(ClientError::Api(e)) if e.status == 403),
        "{r:?}"
    );
    let over_tcp = core_client::HttpClient::new(tcp(&d), Some(Token::new(d.admin_token())))
        .logout()
        .await;
    assert!(
        matches!(&over_tcp, Err(ClientError::Api(e)) if e.status == 403),
        "Logout is forbidden over TCP: {over_tcp:?}"
    );
    assert!(
        !d.requests()
            .iter()
            .any(|r| r.path == "/v1/logout" && r.status == 204),
        "no refused request went through"
    );
}

async fn start_tcp() -> fake_agentd::FakeAgentd {
    common::start(Config::temp().no_ticker().with_tcp()).await
}

#[tokio::test(flavor = "multi_thread")]
async fn blocking_personas_returns_default_fallback() {
    let d = quiet().await;
    let c = blocking(&d);
    let r = tokio::task::spawn_blocking(move || c.personas()).await;
    let Ok(Ok(page)) = r else { panic!("{r:?}") };
    assert_eq!(page.rev, 1);
    assert_eq!(
        page.balance.and_then(|b| b.fallback).as_deref(),
        Some("local")
    );
}
