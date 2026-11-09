//! Token store in the SQLite `api_token` table (db.md section 3.13, `0001_init.sql`): only SHA-256, the raw value
//! is neither in the DB nor in the WAL log. The [`check`] validation repeats the DDL `CHECK` and stays the first line: the error
//! message is clearer, and the DB holds the second (the `trg_api_token_*` triggers and `CHECK` reject what the code lets through).

use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Row, params};
use subtle::ConstantTimeEq;

use crate::db::{Db, DbError};
use crate::token::{generate, sha256_hex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    Read,
    Chat,
    Admin,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Chat => "chat",
            Scope::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Scope> {
        [Scope::Read, Scope::Chat, Scope::Admin]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Gui,
    External,
    Test,
    Agent,
    Module,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Gui => "gui",
            Kind::External => "external",
            Kind::Test => "test",
            Kind::Agent => "agent",
            Kind::Module => "module",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        [
            Kind::Gui,
            Kind::External,
            Kind::Test,
            Kind::Agent,
            Kind::Module,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone)]
pub struct TokenSpec {
    pub label: String,
    pub scope: Scope,
    pub kind: Kind,
    /// `None` - no expiry (GUI and test tokens).
    pub ttl: Option<Duration>,
    pub allowlist: Option<Vec<String>>,
}

impl TokenSpec {
    pub fn new(scope: Scope, kind: Kind) -> TokenSpec {
        TokenSpec {
            label: "test".to_string(),
            scope,
            kind,
            ttl: None,
            allowlist: None,
        }
    }

    pub fn ttl(mut self, ttl: Duration) -> TokenSpec {
        self.ttl = Some(ttl);
        self
    }

    pub fn allowlist(mut self, methods: &[&str]) -> TokenSpec {
        self.allowlist = Some(methods.iter().map(|m| m.to_string()).collect());
        self
    }
}

/// Issue result: the raw value is visible once.
#[derive(Clone, PartialEq, Eq)]
pub struct IssuedToken {
    pub id: i64,
    pub token: String,
}

impl std::fmt::Debug for IssuedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuedToken")
            .field("id", &self.id)
            .field("token", &"<hidden>")
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("token failed validation: {0}")]
    Invalid(String),
    #[error("DB: {0}")]
    Db(String),
}

pub(crate) fn db_err(e: impl std::fmt::Display) -> StoreError {
    StoreError::Db(e.to_string())
}

/// A `CHECK`, `UNIQUE` or trigger violation in the DB is a validation error; anything else is a storage failure.
pub(crate) fn map_db(e: DbError) -> StoreError {
    match e {
        DbError::Sqlite(rusqlite::Error::SqliteFailure(f, msg))
            if f.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            StoreError::Invalid(msg.unwrap_or_else(|| f.to_string()))
        }
        other => db_err(other),
    }
}

/// Table row: it has no raw value.
#[derive(Debug, Clone)]
pub struct TokenRow {
    pub id: i64,
    pub hash: String,
    pub label: String,
    pub scope: Scope,
    pub kind: Kind,
    pub allowlist: Option<Vec<String>>,
    pub created_by: Option<i64>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub last_used_at: Option<i64>,
}

pub struct TokenStore {
    db: Arc<Db>,
}

fn check(spec: &TokenSpec) -> Result<(), StoreError> {
    let bad = |m: &str| Err(StoreError::Invalid(m.to_string()));
    if !(1..=100).contains(&spec.label.chars().count()) {
        return bad("label: 1 to 100 characters");
    }
    if matches!(spec.kind, Kind::External | Kind::Agent | Kind::Module)
        && spec.scope == Scope::Admin
    {
        return bad("scope: admin is not allowed for external, agent and module");
    }
    if matches!(spec.kind, Kind::Agent | Kind::Module) && spec.allowlist.is_none() {
        return bad("allowlist: required for agent and module");
    }
    if spec.ttl.is_some_and(|t| t.as_secs() == 0) {
        return bad("ttl: expiry must be greater than zero");
    }
    Ok(())
}

const COLUMNS: &str = "id, token_hash, label, scope, kind, allowlist_json, created_by, created_at, expires_at, revoked_at, last_used_at";

fn row_of(r: &Row<'_>) -> rusqlite::Result<TokenRow> {
    let bad = |what: &str, v: String| {
        rusqlite::Error::InvalidParameterName(format!("api_token.{what}: {v}"))
    };
    let scope: String = r.get(3)?;
    let kind: String = r.get(4)?;
    let allowlist: Option<String> = r.get(5)?;
    Ok(TokenRow {
        id: r.get(0)?,
        hash: r.get(1)?,
        label: r.get(2)?,
        scope: Scope::parse(&scope).ok_or_else(|| bad("scope", scope.clone()))?,
        kind: Kind::parse(&kind).ok_or_else(|| bad("kind", kind.clone()))?,
        allowlist: allowlist
            .map(|a| serde_json::from_str(&a).map_err(|e| bad("allowlist_json", e.to_string())))
            .transpose()?,
        created_by: r.get(6)?,
        created_at: r.get(7)?,
        expires_at: r.get(8)?,
        revoked_at: r.get(9)?,
        last_used_at: r.get(10)?,
    })
}

impl TokenStore {
    pub fn new(db: Arc<Db>) -> TokenStore {
        TokenStore { db }
    }

    pub fn issue(
        &self,
        spec: &TokenSpec,
        now: i64,
        created_by: Option<i64>,
    ) -> Result<IssuedToken, StoreError> {
        check(spec)?;
        let token = generate();
        let hash = sha256_hex(&token);
        let allowlist = spec
            .allowlist
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(db_err)?;
        let expires_at = spec.ttl.map(|t| now + t.as_secs() as i64);
        let id = self
            .db
            .write(|c| {
                c.execute(
                    "INSERT INTO api_token (token_hash, label, scope, kind, allowlist_json, created_by, created_at, expires_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        hash,
                        spec.label,
                        spec.scope.as_str(),
                        spec.kind.as_str(),
                        allowlist,
                        created_by,
                        now,
                        expires_at
                    ],
                )?;
                Ok(c.last_insert_rowid())
            })
            .map_err(map_db)?;
        Ok(IssuedToken { id, token })
    }

    /// Row by the presented value. Hashes of all rows are compared in constant time, without early exit.
    pub fn lookup(&self, raw: &str) -> Result<Option<TokenRow>, StoreError> {
        let probe = sha256_hex(raw);
        let mut found = None;
        for row in self.rows()? {
            if bool::from(row.hash.as_bytes().ct_eq(probe.as_bytes())) {
                found = Some(row);
            }
        }
        Ok(found)
    }

    pub fn touch(&self, id: i64, now: i64) -> Result<(), StoreError> {
        self.db
            .write(|c| {
                c.execute(
                    "UPDATE api_token SET last_used_at = ?1 WHERE id = ?2",
                    params![now, id],
                )
            })
            .map(|_| ())
            .map_err(map_db)
    }

    /// Revocation: `revoked_at` is set once and never changes (trigger `trg_api_token_no_unrevoke`). `false` - no such id.
    pub fn revoke(&self, id: i64, now: i64) -> Result<bool, StoreError> {
        self.db
            .write(|c| {
                c.execute(
                    "UPDATE api_token SET revoked_at = COALESCE(revoked_at, ?1) WHERE id = ?2",
                    params![now, id],
                )
            })
            .map(|n| n > 0)
            .map_err(map_db)
    }

    pub fn rows(&self) -> Result<Vec<TokenRow>, StoreError> {
        let conn = self.db.reader().map_err(db_err)?;
        let mut st = conn
            .prepare(&format!("SELECT {COLUMNS} FROM api_token ORDER BY id"))
            .map_err(db_err)?;
        let rows = st.query_map([], row_of).map_err(db_err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_err)
    }

    pub fn hashes(&self) -> Result<Vec<String>, StoreError> {
        Ok(self.rows()?.into_iter().map(|r| r.hash).collect())
    }
}
