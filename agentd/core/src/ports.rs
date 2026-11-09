//! Ports implemented by `db` and `auth` and called by `api`: tokens, settings, DB state.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use serde::Serialize;
use serde_json::Value;

use crate::{Scope, Secret, TokenKind};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Field validation error: `field` is the request field name, `message` a constant wording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

impl FieldError {
    #[must_use]
    pub fn new(field: &str, message: &str) -> FieldError {
        FieldError {
            field: field.to_string(),
            message: message.to_string(),
        }
    }
}

/// A token without value and without hash (`docs/reference/api.md`, `Token` schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenInfo {
    pub id: i64,
    pub name: String,
    pub kind: TokenKind,
    pub scope: Scope,
    pub allowlist: Option<Vec<String>>,
    pub created_by: Option<i64>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub last_used: Option<i64>,
    pub revoked: bool,
}

/// Request to issue an external client token (`POST /tokens`).
#[derive(Debug, Clone)]
pub struct ExternalTokenRequest {
    pub name: String,
    pub scope: Scope,
    pub ttl_days: Option<u32>,
    pub created_by: Option<i64>,
}

#[derive(Debug)]
pub struct IssuedExternal {
    pub info: TokenInfo,
    /// Shown once, only the hash is stored.
    pub value: Secret<String>,
}

/// Module panel token: the value is shown once.
#[derive(Debug)]
pub struct IssuedModuleToken {
    pub id: i64,
    pub value: Secret<String>,
    pub expires_at: Option<i64>,
}

#[derive(Debug)]
pub enum TokenAdminError {
    Invalid(Vec<FieldError>),
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeOutcome {
    Revoked,
    AlreadyRevoked,
    NotFound,
}

/// Token management for the `/tokens` routes.
pub trait TokenAdmin: Send + Sync + 'static {
    /// Browser client token with the right to decide permission requests: `scope = chat`, default lifetime
    /// 7 days, maximum 30. The default implementation is unavailable.
    fn issue_web(
        &self,
        _req: ExternalTokenRequest,
    ) -> PortFuture<'_, Result<IssuedExternal, TokenAdminError>> {
        Box::pin(async { Err(TokenAdminError::Unavailable) })
    }

    fn list(&self) -> PortFuture<'_, Result<Vec<TokenInfo>, TokenAdminError>>;
    fn issue_external(
        &self,
        req: ExternalTokenRequest,
    ) -> PortFuture<'_, Result<IssuedExternal, TokenAdminError>>;
    fn revoke(&self, id: i64) -> PortFuture<'_, Result<RevokeOutcome, TokenAdminError>>;
    /// Module panel token: `kind = module`, `scope = chat`, `allowlist` from the manifest.
    fn issue_module(
        &self,
        _module_id: &str,
        _allowlist: Vec<String>,
        _ttl_days: u32,
    ) -> PortFuture<'_, Result<IssuedModuleToken, TokenAdminError>> {
        Box::pin(async { Err(TokenAdminError::Unavailable) })
    }
    /// Revokes the module's active tokens (label `module:<id>`); returns their ids.
    fn revoke_module(&self, _module_id: &str) -> PortFuture<'_, Result<Vec<i64>, TokenAdminError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Settings page document and its `rev` (also the `ETag`).
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsDoc {
    pub doc: Value,
    pub rev: i64,
}

#[derive(Debug)]
pub enum SettingsError {
    /// The page is not in the registry.
    UnknownPage,
    /// The page is read-only.
    ReadOnly,
    /// `If-Match` did not match the current `rev` (`412`).
    Stale {
        current: i64,
    },
    /// The document failed validation after merging (`422`).
    Invalid(Vec<FieldError>),
    Unavailable,
}

/// Settings pages (`docs/reference/api.md` 5.7). `theme` and `indicators` are separate pages with their own schema.
pub trait SettingsStore: Send + Sync + 'static {
    fn get<'a>(&'a self, page: &'a str) -> PortFuture<'a, Result<SettingsDoc, SettingsError>>;
    /// JSON merge patch (RFC 7396) with an `if_match` check; success — the saved document with the new `rev`.
    fn patch<'a>(
        &'a self,
        page: &'a str,
        patch: Value,
        if_match: i64,
    ) -> PortFuture<'a, Result<SettingsDoc, SettingsError>>;
    /// Browser link list by the `settings` key; no record — `default` and `rev = 0`.
    fn get_list<'a>(
        &'a self,
        key: &'a str,
        default: Value,
    ) -> PortFuture<'a, Result<SettingsDoc, SettingsError>>;
    /// Replaces the list wholesale with an `if_match` check in one writer transaction.
    fn put_list<'a>(
        &'a self,
        key: &'a str,
        list: Value,
        if_match: i64,
    ) -> PortFuture<'a, Result<SettingsDoc, SettingsError>>;
    /// `rev` of all written pages (snapshot of the `settings` topic).
    fn revisions(&self) -> PortFuture<'_, Result<BTreeMap<String, i64>, SettingsError>>;
}

/// DB state for `/health` and `/metrics`.
pub trait StatusSource: Send + Sync + 'static {
    fn user_version(&self) -> PortFuture<'_, Option<i64>>;
    /// How many records wait in the writer queue.
    fn write_queue_len(&self) -> usize;
}
