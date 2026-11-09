//! A1-11: a build without `test-support` has no `/_test/*` and no `test` token kind.
#![cfg(not(feature = "test-support"))]

mod common;

use common::{TOKEN_CHAT, config, start, unix_req};

#[tokio::test]
async fn release_build_has_no_test_routes() {
    let dir = tempfile::tempdir().unwrap();
    let s = start(config(dir.path())).await;
    for (m, p) in [
        ("POST", "/v1/_test/advance"),
        ("GET", "/v1/_test/advance"),
        ("POST", "/v1/_test/echo"),
    ] {
        let r = unix_req(s.socket_path(), m, p, Some(TOKEN_CHAT)).await;
        assert_eq!(r.status, 404, "{m} {p}");
    }
    s.shutdown().await;
    assert!(!agentd_auth::TokenPolicy::default().allow_test_kind);
}
