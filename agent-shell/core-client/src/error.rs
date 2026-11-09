//! Core client errors.

use crate::api::ApiError;
use crate::token::TokenError;

/// Client error. Texts contain no tokens or response bodies.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("token unavailable: {0}")]
    Token(#[from] TokenError),
    #[error("no connection to the core: {0}")]
    Transport(String),
    #[error("core response not parsed: {0}")]
    Protocol(String),
    #[error("core returned error {}", .0.status)]
    Api(Box<ApiError>),
    #[error("core did not respond in time")]
    Timeout,
    #[error("client stopped")]
    Stopped,
}

impl ClientError {
    /// Core response with an error code, if that is what it is.
    pub fn api(&self) -> Option<&ApiError> {
        match self {
            ClientError::Api(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}
