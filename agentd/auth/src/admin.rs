//! `TokenAdmin` port for `/tokens` routes: list without values, issue a token to an external client, revoke with outcome.

use crate::{AuthError, NewToken, TokenStore};
use agentd_core::{
    ExternalTokenRequest, FieldError, IssuedExternal, PortFuture, RevokeOutcome, Scope, TokenAdmin,
    TokenAdminError, TokenInfo, TokenKind,
};

/// `api_token` row without the hash; read through an `agentd-db` closure (`auth` has no `rusqlite` of its own).
struct Row {
    id: i64,
    label: String,
    kind: String,
    scope: String,
    allowlist_json: Option<String>,
    created_by: Option<i64>,
    created_at: i64,
    expires_at: Option<i64>,
    last_used_at: Option<i64>,
    revoked_at: Option<i64>,
}

use crate::store::{WEB_DEFAULT_TTL_DAYS, WEB_MAX_TTL_DAYS};

fn info_of(r: Row) -> Option<TokenInfo> {
    let (kind, scope) = (TokenKind::parse(&r.kind)?, Scope::parse(&r.scope)?);
    let allowlist = match r.allowlist_json {
        None => None,
        Some(j) => Some(serde_json::from_str::<Vec<String>>(&j).ok()?),
    };
    Some(TokenInfo {
        id: r.id,
        name: r.label,
        kind,
        scope,
        allowlist,
        created_by: r.created_by,
        created_at: r.created_at,
        expires_at: r.expires_at,
        last_used: r.last_used_at,
        revoked: r.revoked_at.is_some(),
    })
}

fn invalid(field: &str, message: &str) -> TokenAdminError {
    TokenAdminError::Invalid(vec![FieldError::new(field, message)])
}

impl TokenAdmin for TokenStore {
    fn list(&self) -> PortFuture<'_, Result<Vec<TokenInfo>, TokenAdminError>> {
        Box::pin(async move {
            let rows = self
                .db()
                .read(|c| {
                    let mut stmt = c.prepare(
                        "SELECT id, label, kind, scope, allowlist_json, created_by, created_at, expires_at, \
                         last_used_at, revoked_at FROM api_token ORDER BY id",
                    )?;
                    let rows = stmt.query_map([], |r| {
                        Ok(Row {
                            id: r.get(0)?,
                            label: r.get(1)?,
                            kind: r.get(2)?,
                            scope: r.get(3)?,
                            allowlist_json: r.get(4)?,
                            created_by: r.get(5)?,
                            created_at: r.get(6)?,
                            expires_at: r.get(7)?,
                            last_used_at: r.get(8)?,
                            revoked_at: r.get(9)?,
                        })
                    })?;
                    rows.collect::<Result<Vec<_>, _>>()
                })
                .await
                .map_err(|_| TokenAdminError::Unavailable)?;
            // A row with an unknown kind or broken list is DB corruption: the list is not returned partially.
            rows.into_iter()
                .map(|r| {
                    let id = r.id;
                    info_of(r).ok_or_else(|| {
                        tracing::error!(
                            scope = "auth",
                            event = "token_row_corrupt",
                            token_id = id,
                            "token row not parsed"
                        );
                        TokenAdminError::Unavailable
                    })
                })
                .collect()
        })
    }

    fn issue_external(
        &self,
        req: ExternalTokenRequest,
    ) -> PortFuture<'_, Result<IssuedExternal, TokenAdminError>> {
        Box::pin(async move {
            if req.scope > Scope::Chat {
                return Err(invalid(
                    "scope",
                    "an external client does not get admin: read or chat is allowed",
                ));
            }
            let new = NewToken {
                label: req.name.clone(),
                kind: TokenKind::External,
                scope: req.scope,
                ttl_days: req.ttl_days,
                allowlist: None,
                created_by: req.created_by,
            };
            let issued = self.issue(new).await.map_err(|e| match e {
                AuthError::InvalidLabel => invalid("name", "1 to 100 characters"),
                AuthError::TtlTooLong => invalid("ttl_days", "1 to 90"),
                AuthError::ScopeNotAllowed => {
                    invalid("scope", "scope is not allowed for an external client")
                }
                _ => TokenAdminError::Unavailable,
            })?;
            let info = TokenInfo {
                id: issued.id,
                name: req.name,
                kind: TokenKind::External,
                scope: req.scope,
                allowlist: None,
                created_by: req.created_by,
                created_at: self.now_unix(),
                expires_at: issued.expires_at,
                last_used: None,
                revoked: false,
            };
            Ok(IssuedExternal {
                info,
                value: issued.value,
            })
        })
    }

    fn issue_web(
        &self,
        req: ExternalTokenRequest,
    ) -> PortFuture<'_, Result<IssuedExternal, TokenAdminError>> {
        Box::pin(async move {
            if req.scope != Scope::Chat {
                return Err(invalid("scope", "web token with decide right: chat only"));
            }
            let ttl = req.ttl_days.unwrap_or(WEB_DEFAULT_TTL_DAYS);
            if ttl == 0 || ttl > WEB_MAX_TTL_DAYS {
                return Err(invalid("ttl_days", "1 to 30"));
            }
            let new = NewToken {
                label: req.name.clone(),
                kind: TokenKind::Web,
                scope: Scope::Decide,
                ttl_days: Some(ttl),
                allowlist: None,
                created_by: req.created_by,
            };
            let issued = self.issue(new).await.map_err(|e| match e {
                AuthError::InvalidLabel => invalid("name", "1 to 100 characters"),
                _ => TokenAdminError::Unavailable,
            })?;
            let info = TokenInfo {
                id: issued.id,
                name: req.name,
                kind: TokenKind::Web,
                scope: Scope::Decide,
                allowlist: None,
                created_by: req.created_by,
                created_at: self.now_unix(),
                expires_at: issued.expires_at,
                last_used: None,
                revoked: false,
            };
            Ok(IssuedExternal {
                info,
                value: issued.value,
            })
        })
    }

    fn issue_module(
        &self,
        module_id: &str,
        allowlist: Vec<String>,
        ttl_days: u32,
    ) -> PortFuture<'_, Result<agentd_core::IssuedModuleToken, TokenAdminError>> {
        let id = module_id.to_string();
        Box::pin(async move {
            let t = TokenStore::issue_module(self, &id, allowlist, ttl_days)
                .await
                .map_err(|_| TokenAdminError::Unavailable)?;
            Ok(agentd_core::IssuedModuleToken {
                id: t.id,
                value: t.value,
                expires_at: t.expires_at,
            })
        })
    }

    fn revoke_module(&self, module_id: &str) -> PortFuture<'_, Result<Vec<i64>, TokenAdminError>> {
        let id = module_id.to_string();
        Box::pin(async move {
            self.revoke_module_tokens(&id)
                .await
                .map_err(|_| TokenAdminError::Unavailable)
        })
    }

    fn revoke(&self, id: i64) -> PortFuture<'_, Result<RevokeOutcome, TokenAdminError>> {
        Box::pin(async move {
            let now = self.now_unix();
            let outcome = self
                .db()
                .write(move |tx| {
                    let row: Option<Option<i64>> = {
                        let mut stmt = tx.prepare("SELECT revoked_at FROM api_token WHERE id = ?1")?;
                        let mut rows = stmt.query([id])?;
                        rows.next()?.map(|r| r.get(0)).transpose()?
                    };
                    match row {
                        None => Ok(RevokeOutcome::NotFound),
                        Some(Some(_)) => Ok(RevokeOutcome::AlreadyRevoked),
                        Some(None) => {
                            tx.execute(
                                "UPDATE api_token SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
                                agentd_db::params![now, id],
                            )?;
                            Ok(RevokeOutcome::Revoked)
                        }
                    }
                })
                .await
                .map_err(|_| TokenAdminError::Unavailable)?;
            if outcome == RevokeOutcome::Revoked {
                tracing::info!(
                    scope = "auth",
                    event = "token_revoked",
                    token_id = id,
                    "token revoked"
                );
            }
            Ok(outcome)
        })
    }
}
