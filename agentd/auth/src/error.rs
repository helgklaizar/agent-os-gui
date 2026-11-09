//! Errors of token issue and revoke.

use agentd_db::DbError;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("scope is not allowed for this token kind")]
    ScopeNotAllowed,
    #[error("token lifetime exceeds the maximum")]
    TtlTooLong,
    #[error("this token kind needs a list of allowed methods")]
    AllowlistRequired,
    #[error("this token kind has no lifetime setting")]
    TtlNotAllowed,
    #[error("the allowed methods list cannot be serialized")]
    InvalidAllowlist,
    #[error("token kind is not allowed in this build")]
    KindNotAllowed,
    #[error("token label is empty or longer than 100 characters")]
    InvalidLabel,
    #[error("DB: {0}")]
    Db(#[from] DbError),
}
