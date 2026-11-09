//! Token storage on top of `agentd-db` (single writer, separate readers).
//! The raw value is never stored or logged: issuing returns it once in a `Secret`.

use std::sync::Arc;

use agentd_core::{
    AuthFailure, AuthFuture, Authenticator, Clock, Principal, Scope, Secret, TokenKind,
};
use agentd_db::Db;
use subtle::ConstantTimeEq;

use crate::{AuthError, token};

const DAY: i64 = 86_400;
/// `last_used_at` is updated at most once a minute: every request must not occupy the write queue.
const LAST_USED_STEP: i64 = 60;
/// Web token lifetime: default and maximum, one day.
pub const WEB_DEFAULT_TTL_DAYS: u32 = 7;
pub const WEB_MAX_TTL_DAYS: u32 = 30;

#[derive(Debug, Clone)]
pub struct TokenPolicy {
    /// The `test` kind is allowed only in the test build.
    pub allow_test_kind: bool,
    pub external_default_ttl_days: u32,
    pub external_max_ttl_days: u32,
}

impl Default for TokenPolicy {
    fn default() -> Self {
        TokenPolicy {
            allow_test_kind: false,
            external_default_ttl_days: 30,
            external_max_ttl_days: 90,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewToken {
    pub label: String,
    pub kind: TokenKind,
    pub scope: Scope,
    pub ttl_days: Option<u32>,
    pub allowlist: Option<Vec<String>>,
    pub created_by: Option<i64>,
}

#[derive(Debug)]
pub struct IssuedToken {
    pub id: i64,
    /// Shown once; the DB stores only the hash.
    pub value: Secret<String>,
    pub expires_at: Option<i64>,
}

/// Agent token lifetime, days: a unit restart issues a new one and revokes the previous.
pub const AGENT_TTL_DAYS: u32 = 30;

pub struct TokenStore {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
    policy: TokenPolicy,
    /// Test build: fixed token source (seed, counter).
    #[cfg(feature = "test-support")]
    fixed: Option<(u64, std::sync::atomic::AtomicU64)>,
}

struct Row {
    id: i64,
    token_hash: String,
    scope: String,
    kind: String,
    allowlist_json: Option<String>,
    expires_at: Option<i64>,
    last_used_at: Option<i64>,
}

impl TokenStore {
    pub(crate) fn db(&self) -> &Db {
        &self.db
    }

    pub(crate) fn now_unix(&self) -> i64 {
        self.clock.now_unix()
    }

    #[must_use]
    pub fn new(db: Arc<Db>, clock: Arc<dyn Clock>, policy: TokenPolicy) -> TokenStore {
        TokenStore {
            db,
            clock,
            policy,
            #[cfg(feature = "test-support")]
            fixed: None,
        }
    }

    /// Test build: tokens are issued deterministically (one seed, one sequence).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn with_fixed_seed(
        db: Arc<Db>,
        clock: Arc<dyn Clock>,
        policy: TokenPolicy,
        seed: u64,
    ) -> TokenStore {
        let mut s = TokenStore::new(db, clock, policy);
        s.fixed = Some((seed, std::sync::atomic::AtomicU64::new(0)));
        s
    }

    fn new_value(&self) -> String {
        #[cfg(feature = "test-support")]
        if let Some((seed, n)) = &self.fixed {
            return token::generate_fixed(
                *seed,
                n.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            );
        }
        token::generate()
    }

    /// Lifetime in days by token kind: `external` defaults to 30, maximum 90; `gui` has none.
    fn ttl_days(&self, new: &NewToken) -> Result<Option<u32>, AuthError> {
        let max = self.policy.external_max_ttl_days;
        match (new.kind, new.ttl_days) {
            (TokenKind::Gui, None) => Ok(None),
            (TokenKind::Gui, Some(_)) => Err(AuthError::TtlNotAllowed),
            (TokenKind::External, None) => Ok(Some(self.policy.external_default_ttl_days)),
            // A web token always has a lifetime: 7 days by default, maximum 30.
            (TokenKind::Web, None) => Ok(Some(WEB_DEFAULT_TTL_DAYS)),
            (TokenKind::Web, Some(d)) if d == 0 || d > WEB_MAX_TTL_DAYS => {
                Err(AuthError::TtlTooLong)
            }
            (_, Some(d)) if d == 0 || (new.kind != TokenKind::Test && d > max) => {
                Err(AuthError::TtlTooLong)
            }
            (_, d) => Ok(d),
        }
    }

    fn check_rules(&self, new: &NewToken) -> Result<(), AuthError> {
        let label_len = new.label.chars().count();
        if !(1..=100).contains(&label_len) {
            return Err(AuthError::InvalidLabel);
        }
        let scope_ok = match new.kind {
            TokenKind::Gui => new.scope == Scope::Admin,
            TokenKind::External | TokenKind::Agent | TokenKind::Module => new.scope <= Scope::Chat,
            TokenKind::Web => matches!(new.scope, Scope::Chat | Scope::Decide),
            TokenKind::Test => true,
        };
        if new.kind == TokenKind::Test && !self.policy.allow_test_kind {
            return Err(AuthError::KindNotAllowed);
        }
        if !scope_ok {
            return Err(AuthError::ScopeNotAllowed);
        }
        if matches!(new.kind, TokenKind::Agent | TokenKind::Module) && new.allowlist.is_none() {
            return Err(AuthError::AllowlistRequired);
        }
        Ok(())
    }

    /// Issues a token: kind and scope rules, lifetime, only the hash is stored.
    pub async fn issue(&self, new: NewToken) -> Result<IssuedToken, AuthError> {
        self.check_rules(&new)?;
        let ttl = self.ttl_days(&new)?;
        let now = self.clock.now_unix();
        let expires_at = ttl.map(|d| now + i64::from(d) * DAY);
        let value = self.new_value();
        let hash = token::sha256_hex(&value);
        let allowlist = new
            .allowlist
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| AuthError::InvalidAllowlist)?;
        let (label, kind, scope, created_by) =
            (new.label.clone(), new.kind, new.scope, new.created_by);
        let id = self
            .db
            .write(move |tx| {
                tx.execute(
                    "INSERT INTO api_token(token_hash, label, scope, kind, allowlist_json, created_by, created_at, expires_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    agentd_db::params![hash, label, scope.as_str(), kind.as_str(), allowlist, created_by, now, expires_at],
                )?;
                Ok(tx.last_insert_rowid())
            })
            .await?;
        // Issue log: only id, kind and scope; the value and label are not written. The `actions_log` table is separate.
        tracing::info!(
            event = "token_issued",
            token_id = id,
            kind = kind.as_str(),
            scope = scope.as_str(),
            "token issued"
        );
        Ok(IssuedToken {
            id,
            value: Secret::new(value),
            expires_at,
        })
    }

    /// Module panel token: `kind = module`, `scope = chat`, method list from the manifest, label `module:<id>`.
    /// There is no outward issue route; `admin` cannot be issued to a module under any arguments.
    pub async fn issue_module(
        &self,
        module_id: &str,
        allowlist: Vec<String>,
        ttl_days: u32,
    ) -> Result<IssuedToken, AuthError> {
        self.issue(NewToken {
            label: format!("module:{module_id}"),
            kind: TokenKind::Module,
            scope: Scope::Chat,
            ttl_days: Some(ttl_days),
            allowlist: Some(allowlist),
            created_by: None,
        })
        .await
    }

    /// Revokes the module's active tokens by label; returns their ids.
    pub async fn revoke_module_tokens(&self, module_id: &str) -> Result<Vec<i64>, AuthError> {
        self.revoke_by_label(TokenKind::Module, format!("module:{module_id}"))
            .await
    }

    /// Session agent process token: `kind = agent`, `scope = chat`, label `agent:<session>`, method list is
    /// only this session's messages (concrete paths: another session is closed by `forbidden_method`). Lifetime is `AGENT_TTL_DAYS`.
    /// `admin`, permission decisions and settings are unavailable to an agent under any arguments.
    pub async fn issue_agent(&self, session_id: i64) -> Result<IssuedToken, AuthError> {
        self.issue(NewToken {
            label: format!("agent:{session_id}"),
            kind: TokenKind::Agent,
            scope: Scope::Chat,
            ttl_days: Some(AGENT_TTL_DAYS),
            allowlist: Some(vec![
                format!("GET /sessions/{session_id}/messages"),
                format!("POST /sessions/{session_id}/messages"),
            ]),
            created_by: None,
        })
        .await
    }

    /// Revokes the session agent's active tokens; returns their ids.
    pub async fn revoke_agent_tokens(&self, session_id: i64) -> Result<Vec<i64>, AuthError> {
        self.revoke_by_label(TokenKind::Agent, format!("agent:{session_id}"))
            .await
    }

    async fn revoke_by_label(&self, kind: TokenKind, label: String) -> Result<Vec<i64>, AuthError> {
        let now = self.clock.now_unix();
        let kind = kind.as_str();
        Ok(self
            .db
            .write(move |tx| {
                let ids: Vec<i64> = {
                    let mut stmt = tx.prepare(
                        "SELECT id FROM api_token WHERE kind = ?1 AND label = ?2 AND revoked_at IS NULL",
                    )?;
                    let rows = stmt.query_map(agentd_db::params![kind, label], |r| r.get(0))?;
                    rows.collect::<Result<_, _>>()?
                };
                tx.execute(
                    "UPDATE api_token SET revoked_at = ?1 WHERE kind = ?2 AND label = ?3 AND revoked_at IS NULL",
                    agentd_db::params![now, kind, label],
                )?;
                Ok(ids)
            })
            .await?)
    }

    /// Revocation is irreversible: an already revoked token is unchanged (`false`).
    pub async fn revoke(&self, id: i64) -> Result<bool, AuthError> {
        let now = self.clock.now_unix();
        let changed = self
            .db
            .write(move |tx| {
                tx.execute(
                    "UPDATE api_token SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
                    agentd_db::params![now, id],
                )
            })
            .await?;
        if changed > 0 {
            tracing::info!(event = "token_revoked", token_id = id, "token revoked");
        }
        Ok(changed > 0)
    }

    /// Revokes all active tokens of `kind` (a new GUI token revokes the previous ones). Returns their ids.
    pub async fn revoke_active(&self, kind: TokenKind) -> Result<Vec<i64>, AuthError> {
        let now = self.clock.now_unix();
        let kind = kind.as_str();
        let ids = self
            .db
            .write(move |tx| {
                let ids: Vec<i64> = {
                    let mut stmt = tx.prepare(
                        "SELECT id FROM api_token WHERE kind = ?1 AND revoked_at IS NULL",
                    )?;
                    let rows = stmt.query_map([kind], |r| r.get(0))?;
                    rows.collect::<Result<_, _>>()?
                };
                tx.execute(
                    "UPDATE api_token SET revoked_at = ?1 WHERE kind = ?2 AND revoked_at IS NULL",
                    agentd_db::params![now, kind],
                )?;
                Ok(ids)
            })
            .await?;
        for id in &ids {
            tracing::info!(
                scope = "auth",
                event = "token_revoked",
                token_id = *id,
                "token revoked"
            );
        }
        Ok(ids)
    }

    /// Checks a presented token. Wrong format is rejected without touching the DB; the hash is compared in constant time.
    pub async fn verify(&self, raw: &str) -> Result<Principal, AuthFailure> {
        if !token::is_valid_format(raw) {
            return Err(AuthFailure::Invalid);
        }
        let hash = token::sha256_hex(raw);
        let lookup = hash.clone();
        let row = self
            .db
            .read(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT id, token_hash, scope, kind, allowlist_json, expires_at, last_used_at \
                     FROM api_token WHERE token_hash = ?1 AND revoked_at IS NULL",
                )?;
                let mut rows = stmt.query([lookup])?;
                let Some(r) = rows.next()? else {
                    return Ok(None);
                };
                Ok(Some(Row {
                    id: r.get(0)?,
                    token_hash: r.get(1)?,
                    scope: r.get(2)?,
                    kind: r.get(3)?,
                    allowlist_json: r.get(4)?,
                    expires_at: r.get(5)?,
                    last_used_at: r.get(6)?,
                }))
            })
            .await
            .map_err(|_| AuthFailure::Unavailable)?;
        let Some(row) = row else {
            return Err(AuthFailure::Invalid);
        };
        let same: bool = row.token_hash.as_bytes().ct_eq(hash.as_bytes()).into();
        let now = self.clock.now_unix();
        if !same || row.expires_at.is_some_and(|e| now >= e) {
            return Err(AuthFailure::Invalid);
        }
        let principal = principal_of(&row)?;
        self.touch(row.id, row.last_used_at, now).await;
        Ok(principal)
    }

    async fn touch(&self, id: i64, last: Option<i64>, now: i64) {
        if last.is_some_and(|l| now - l < LAST_USED_STEP) {
            return;
        }
        let res = self
            .db
            .write(move |tx| {
                tx.execute(
                    "UPDATE api_token SET last_used_at = ?1 WHERE id = ?2",
                    agentd_db::params![now, id],
                )
            })
            .await;
        if res.is_err() {
            // The token check does not fail because of this: `last_used_at` is informational for the token list.
            tracing::warn!(
                event = "last_used_update_failed",
                token_id = id,
                "last_used_at not updated"
            );
        }
    }
}

fn principal_of(row: &Row) -> Result<Principal, AuthFailure> {
    let (Some(scope), Some(kind)) = (Scope::parse(&row.scope), TokenKind::parse(&row.kind)) else {
        tracing::error!(
            event = "token_row_corrupt",
            token_id = row.id,
            "token row with unknown kind or scope"
        );
        return Err(AuthFailure::Invalid);
    };
    let allowlist = match &row.allowlist_json {
        None => None,
        Some(j) => match serde_json::from_str::<Vec<String>>(j) {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::error!(
                    event = "token_row_corrupt",
                    token_id = row.id,
                    "allowlist_json is not a list of strings"
                );
                return Err(AuthFailure::Invalid);
            }
        },
    };
    Ok(Principal {
        token_id: row.id,
        kind,
        scope,
        allowlist,
        expires_at: row.expires_at,
    })
}

impl Authenticator for TokenStore {
    fn authenticate<'a>(&'a self, token: &'a str) -> AuthFuture<'a> {
        Box::pin(self.verify(token))
    }
}
