//! Module tokens: `kind = module`, `scope = chat`, a mandatory method list, `admin` is never issued,
//! revoking by module closes only its tokens.

mod common;

use agentd_auth::AuthError;
use agentd_core::{AuthFailure, Scope, TokenAdmin, TokenKind};
use common::{env, new_token};

fn list() -> Vec<String> {
    vec![
        "GET /sessions".into(),
        "POST /sessions/*/tool-results".into(),
    ]
}

#[tokio::test]
async fn module_token_has_chat_scope_and_allowlist() {
    let e = env();
    let t = e.store.issue_module("weather", list(), 1).await.unwrap();
    let p = e.store.verify(t.value.expose()).await.unwrap();
    assert_eq!((p.kind, p.scope), (TokenKind::Module, Scope::Chat));
    assert_eq!(p.allowlist.as_deref(), Some(list().as_slice()));
    assert!(t.expires_at.is_some(), "panel lifetime is bounded");
}

#[tokio::test]
async fn module_token_never_admin_and_needs_allowlist() {
    let e = env();
    let mut n = new_token(TokenKind::Module, Scope::Admin);
    n.allowlist = Some(list());
    assert!(matches!(
        e.store.issue(n).await,
        Err(AuthError::ScopeNotAllowed)
    ));
    let mut n = new_token(TokenKind::Module, Scope::Chat);
    n.allowlist = None;
    assert!(matches!(
        e.store.issue(n).await,
        Err(AuthError::AllowlistRequired)
    ));
    assert!(
        e.store.issue_module("m", list(), 365).await.is_err(),
        "lifetime outside the allowed range"
    );
}

#[tokio::test]
async fn module_token_revoke_is_per_module() {
    let e = env();
    let a = e.store.issue_module("weather", list(), 1).await.unwrap();
    let b = e.store.issue_module("notes", list(), 1).await.unwrap();
    let ids = e.store.revoke_module("weather").await.unwrap();
    assert_eq!(ids, [a.id]);
    assert!(matches!(
        e.store.verify(a.value.expose()).await,
        Err(AuthFailure::Invalid)
    ));
    assert!(
        e.store.verify(b.value.expose()).await.is_ok(),
        "another module is untouched"
    );
    assert!(
        e.store.revoke_module("weather").await.unwrap().is_empty(),
        "repeat is harmless"
    );
}
