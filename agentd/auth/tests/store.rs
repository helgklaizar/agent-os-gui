//! Token storage and checks on top of a real DB (migration 0001, single writer).

mod common;

use agentd_auth::{AuthError, TokenPolicy, token};
use agentd_core::{AuthFailure, Scope, TokenKind};
use common::{T0, env, env_with, external, new_token};
use sha2::{Digest, Sha256};

fn sha_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

#[tokio::test]
async fn api_token_stores_hash_only() {
    let e = env();
    let issued = e.store.issue(external(Scope::Chat)).await.unwrap();
    let raw = issued.value.expose().clone();
    assert!(
        raw.starts_with("agt_") && raw.len() == 47,
        "agt_ + 43 base64url characters: {}",
        raw.len()
    );
    assert!(
        !format!("{issued:?}").contains(&raw),
        "Debug does not print the value"
    );
    let rows: Vec<(i64, String)> =
        e.db.read(|c| {
            c.prepare("SELECT id, token_hash FROM api_token")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()
        })
        .await
        .unwrap();
    assert_eq!(
        rows,
        vec![(issued.id, sha_hex(&raw))],
        "the table holds only SHA-256 in hex"
    );
    e.db.write(|tx| tx.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)"))
        .await
        .ok();
    let body = &raw[4..];
    for name in ["sessions.db", "sessions.db-wal"] {
        let bytes = std::fs::read(e.dir.path().join("agentos").join(name)).unwrap_or_default();
        assert!(
            !contains(&bytes, raw.as_bytes()) && !contains(&bytes, body.as_bytes()),
            "raw value found in {name}"
        );
    }
    assert!(
        e.store.verify(&raw).await.is_ok(),
        "the issued token passes the check"
    );
}

#[test]
fn token_format_is_exactly_the_generators() {
    for _ in 0..50 {
        let t = token::generate();
        assert!(token::is_valid_format(&t), "own accepted: {t}");
        assert_eq!(token::sha256_hex(&t), sha_hex(&t));
    }
    let good = token::generate();
    let body = &good[4..];
    for bad in [
        String::new(),
        "agt_".to_string(),
        format!("xyz_{body}"),
        format!("agt_{}", &body[..42]),
        format!("agt_{body}A"),
        format!("agt_{}=", &body[..42]),
        format!("agt_{}+", &body[..42]),
        format!("agt_{}/", &body[..42]),
        format!("agt_{} ", &body[..42]),
        format!("AGT_{body}"),
    ] {
        assert!(!token::is_valid_format(&bad), "foreign rejected: {bad:?}");
    }
    // the last character carries 4 significant bits: the generator cannot produce 'B' (index 1)
    let tail_bad = format!("agt_{}B", &body[..42]);
    assert!(!token::is_valid_format(&tail_bad));
}

#[tokio::test]
async fn expires() {
    let e = env();
    let mut n = external(Scope::Chat);
    n.ttl_days = Some(1);
    let short = e.store.issue(n).await.unwrap();
    assert_eq!(short.expires_at, Some(T0 + 86_400));
    let gui = e
        .store
        .issue(new_token(TokenKind::Gui, Scope::Admin))
        .await
        .unwrap();
    assert_eq!(gui.expires_at, None, "GUI token has no expiry");
    let (short_raw, gui_raw) = (short.value.expose().clone(), gui.value.expose().clone());
    assert!(e.store.verify(&short_raw).await.is_ok());
    e.clock.advance_secs(86_400 - 1);
    assert!(
        e.store.verify(&short_raw).await.is_ok(),
        "one second before expiry"
    );
    e.clock.advance_secs(1);
    assert_eq!(
        e.store.verify(&short_raw).await.unwrap_err(),
        AuthFailure::Invalid,
        "expired: 401"
    );
    e.clock.advance_secs(10 * 365 * 86_400);
    assert!(e.store.verify(&gui_raw).await.is_ok(), "GUI token lives");
}

#[tokio::test]
async fn external_default_30_days_max_90() {
    let e = env();
    let d = e.store.issue(external(Scope::Chat)).await.unwrap();
    assert_eq!(d.expires_at, Some(T0 + 30 * 86_400), "default is 30 days");
    let mut n = external(Scope::Chat);
    n.ttl_days = Some(90);
    assert_eq!(
        e.store.issue(n).await.unwrap().expires_at,
        Some(T0 + 90 * 86_400)
    );
    let mut n = external(Scope::Chat);
    n.ttl_days = Some(91);
    assert!(matches!(e.store.issue(n).await, Err(AuthError::TtlTooLong)));
    let mut n = external(Scope::Chat);
    n.ttl_days = Some(0);
    assert!(
        matches!(e.store.issue(n).await, Err(AuthError::TtlTooLong)),
        "zero lifetime is not unlimited"
    );
}

#[tokio::test]
async fn external_not_admin() {
    let e = env();
    assert!(matches!(
        e.store.issue(external(Scope::Admin)).await,
        Err(AuthError::ScopeNotAllowed)
    ));
    assert!(e.store.issue(external(Scope::Read)).await.is_ok());
    assert!(e.store.issue(external(Scope::Chat)).await.is_ok());
    let n: i64 =
        e.db.read(|c| c.query_row("SELECT count(*) FROM api_token", [], |r| r.get(0)))
            .await
            .unwrap();
    assert_eq!(n, 2, "the rejected token did not reach the table");
}

#[tokio::test]
async fn agent_token_scope_chat() {
    let e = env();
    let allow = Some(vec!["GET /v1/health".to_string()]);
    for kind in [TokenKind::Agent, TokenKind::Module] {
        let mut admin = new_token(kind, Scope::Admin);
        admin.allowlist = allow.clone();
        assert!(
            matches!(e.store.issue(admin).await, Err(AuthError::ScopeNotAllowed)),
            "{kind:?} is not admin"
        );
        let no_list = new_token(kind, Scope::Chat);
        assert!(
            matches!(
                e.store.issue(no_list).await,
                Err(AuthError::AllowlistRequired)
            ),
            "{kind:?} without allowlist"
        );
        let mut ok = new_token(kind, Scope::Chat);
        ok.allowlist = allow.clone();
        ok.ttl_days = Some(7);
        let issued = e.store.issue(ok).await.unwrap();
        let p = e.store.verify(issued.value.expose()).await.unwrap();
        assert_eq!(
            (p.kind, p.scope, p.allowlist.clone()),
            (kind, Scope::Chat, allow.clone())
        );
    }
}

#[tokio::test]
async fn kind_test_only_when_policy_allows_and_gui_is_admin() {
    let e = env();
    assert!(matches!(
        e.store
            .issue(new_token(TokenKind::Test, Scope::Admin))
            .await,
        Err(AuthError::KindNotAllowed)
    ));
    let t = env_with(TokenPolicy {
        allow_test_kind: true,
        ..TokenPolicy::default()
    });
    assert!(
        t.store
            .issue(new_token(TokenKind::Test, Scope::Admin))
            .await
            .is_ok()
    );
    assert!(
        matches!(
            e.store.issue(new_token(TokenKind::Gui, Scope::Chat)).await,
            Err(AuthError::ScopeNotAllowed)
        ),
        "gui is always admin"
    );
    let mut bad = external(Scope::Chat);
    bad.label = String::new();
    assert!(matches!(
        e.store.issue(bad).await,
        Err(AuthError::InvalidLabel)
    ));
}

#[tokio::test]
async fn token_revoke_is_final() {
    let e = env();
    let issued = e.store.issue(external(Scope::Chat)).await.unwrap();
    let raw = issued.value.expose().clone();
    assert!(e.store.verify(&raw).await.is_ok());
    assert!(e.store.revoke(issued.id).await.unwrap(), "first revoke");
    let first: i64 =
        e.db.read(move |c| {
            c.query_row(
                "SELECT revoked_at FROM api_token WHERE id = ?1",
                [issued.id],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(first, T0);
    e.clock.advance_secs(500);
    assert!(
        !e.store.revoke(issued.id).await.unwrap(),
        "repeat revoke changes nothing"
    );
    let second: i64 =
        e.db.read(move |c| {
            c.query_row(
                "SELECT revoked_at FROM api_token WHERE id = ?1",
                [issued.id],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(second, first, "revoked_at not overwritten");
    assert_eq!(
        e.store.verify(&raw).await.unwrap_err(),
        AuthFailure::Invalid
    );
    assert!(!e.store.revoke(987_654).await.unwrap(), "nonexistent id");
}

#[tokio::test]
async fn verify_rejects_unknown_and_malformed_without_side_effects() {
    let e = env();
    e.store.issue(external(Scope::Chat)).await.unwrap();
    for bad in [
        token::generate(),
        String::new(),
        "agt_x".to_string(),
        "Bearer".to_string(),
    ] {
        assert_eq!(
            e.store.verify(&bad).await.unwrap_err(),
            AuthFailure::Invalid,
            "{bad:?}"
        );
    }
    let n: i64 =
        e.db.read(|c| {
            c.query_row(
                "SELECT count(*) FROM api_token WHERE last_used_at IS NOT NULL",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(n, 0, "failed checks do not touch last_used_at");
}

#[tokio::test]
async fn last_used_at_updated_at_most_once_per_minute() {
    let e = env();
    let issued = e.store.issue(external(Scope::Chat)).await.unwrap();
    let raw = issued.value.expose().clone();
    let id = issued.id;
    let last = |db: std::sync::Arc<agentd_db::Db>| async move {
        db.read(move |c| {
            c.query_row(
                "SELECT last_used_at FROM api_token WHERE id = ?1",
                [id],
                |r| r.get::<_, Option<i64>>(0),
            )
        })
        .await
        .unwrap()
    };
    assert_eq!(last(e.db.clone()).await, None);
    e.store.verify(&raw).await.unwrap();
    assert_eq!(last(e.db.clone()).await, Some(T0));
    e.clock.advance_secs(10);
    e.store.verify(&raw).await.unwrap();
    assert_eq!(
        last(e.db.clone()).await,
        Some(T0),
        "no repeated write within a minute"
    );
    e.clock.advance_secs(120);
    e.store.verify(&raw).await.unwrap();
    assert_eq!(last(e.db.clone()).await, Some(T0 + 130));
}

#[test]
fn constant_time_compare_used() {
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/store.rs")).unwrap();
    assert!(
        src.contains("ConstantTimeEq") && src.contains(".ct_eq("),
        "hashes compared via subtle"
    );
    for line in src.lines().map(str::trim).filter(|l| !l.starts_with("//")) {
        assert!(
            !(line.contains("token_hash") && (line.contains(" == ") || line.contains(" != ")))
                && !(line.contains("stored") && line.contains(" == ")),
            "hash compared with the == operator: {line}"
        );
    }
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn test_kind_and_fixed_tokens_in_test_build() {
    use agentd_auth::{TokenPolicy, TokenStore};
    let e = common::env_with(TokenPolicy {
        allow_test_kind: true,
        ..TokenPolicy::default()
    });
    let a = e
        .store
        .issue(common::new_token(
            agentd_core::TokenKind::Test,
            agentd_core::Scope::Admin,
        ))
        .await
        .unwrap();
    assert!(e.store.verify(a.value.expose()).await.is_ok());
    let e2 = common::env();
    let s1 = TokenStore::with_fixed_seed(e.db.clone(), e.clock.clone(), TokenPolicy::default(), 7);
    let s2 = TokenStore::with_fixed_seed(e2.db.clone(), e.clock.clone(), TokenPolicy::default(), 7);
    let (t1, t2) = (
        s1.issue(common::external(agentd_core::Scope::Read))
            .await
            .unwrap(),
        s2.issue(common::external(agentd_core::Scope::Read))
            .await
            .unwrap(),
    );
    assert_eq!(
        t1.value.expose(),
        t2.value.expose(),
        "one seed gives the same tokens"
    );
}

#[cfg(not(feature = "test-support"))]
#[tokio::test]
async fn test_kind_rejected_by_default_policy() {
    let e = common::env();
    let r = e
        .store
        .issue(common::new_token(
            agentd_core::TokenKind::Test,
            agentd_core::Scope::Admin,
        ))
        .await;
    assert!(matches!(r, Err(agentd_auth::AuthError::KindNotAllowed)));
}
