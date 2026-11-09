//! Permissions on `GET /search` (api.md section 5.5, scope `read`) on `fake-agentd` over a real socket.

mod common;
#[allow(dead_code)]
#[path = "contract/rawhttp.rs"]
mod rawhttp;

use fake_agentd::{Config, FakeAgentd, Kind, Scope, TokenSpec};
use rawhttp::{Req, send};
use serde_json::json;

async fn start() -> (tempfile::TempDir, FakeAgentd) {
    let (d, path) = common::temp_db_path();
    common::fixture::seed_search(&common::open(&path));
    let fa = FakeAgentd::start(Config::temp().no_ticker().db(path))
        .await
        .expect("fake-agentd");
    (d, fa)
}

fn issue(fa: &FakeAgentd, spec: TokenSpec) -> String {
    fa.issue(spec).expect("token issued").token
}

async fn status_of(fa: &FakeAgentd, token: &str) -> (u16, serde_json::Value) {
    let r = send(
        fa.socket_path(),
        Req::new("GET", "/v1/search?q=a").token(token),
    )
    .await;
    (r.status, r.json())
}

/// There is no scope below `read`, so a permissions refusal comes from a module token that lacks `GET /search` in its allowlist
/// (D45.2: `403 forbidden_method`).
#[tokio::test(flavor = "multi_thread")]
async fn search_outside_module_allowlist_is_403() {
    let (_d, fa) = start().await;
    let other = issue(
        &fa,
        TokenSpec::new(Scope::Chat, Kind::Module).allowlist(&["GET /settings/theme"]),
    );
    let (status, body) = status_of(&fa, &other).await;
    assert_eq!(status, 403);
    assert_eq!(body["error"]["code"], "forbidden_method");
    assert!(
        !body.to_string().contains(&other),
        "the token does not appear in the response"
    );

    let empty = issue(
        &fa,
        TokenSpec::new(Scope::Read, Kind::Module).allowlist(&[]),
    );
    assert_eq!(
        status_of(&fa, &empty).await.0,
        403,
        "an empty allowlist lets nothing through"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn search_in_module_allowlist_is_200() {
    let (_d, fa) = start().await;
    let allowed = issue(
        &fa,
        TokenSpec::new(Scope::Read, Kind::Module).allowlist(&["GET /search"]),
    );
    assert_eq!(status_of(&fa, &allowed).await.0, 200);
}

/// `read` tokens search in any kind; the indicator frame is an `admin` service route, `read` gets `insufficient_scope`.
#[tokio::test(flavor = "multi_thread")]
async fn search_is_read_scope_and_test_frame_is_admin_only() {
    let (_d, fa) = start().await;
    for kind in [Kind::Test, Kind::External] {
        let t = issue(&fa, TokenSpec::new(Scope::Read, kind));
        assert_eq!(
            status_of(&fa, &t).await.0,
            200,
            "read token {kind:?} searches"
        );
    }
    let read = issue(&fa, TokenSpec::new(Scope::Read, Kind::Test));
    let r = send(
        fa.socket_path(),
        Req::new("POST", "/v1/_test/indicator-frame")
            .token(&read)
            .json(json!({"id": "cpu", "state": "ok"})),
    )
    .await;
    assert_eq!(r.status, 403);
    assert_eq!(r.json()["error"]["code"], "insufficient_scope");
}
