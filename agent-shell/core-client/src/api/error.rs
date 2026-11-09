//! Core errors: the api.md section 3 table codes and response body parsing.

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

/// Error code from the api.md section 3 table; an unknown code is kept as is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    BadRequest,
    Unauthorized,
    PermissionDenied,
    InsufficientScope,
    ForbiddenMethod,
    ForbiddenOrigin,
    NotFound,
    Conflict,
    ClientIdConflict,
    PreconditionFailed,
    PayloadTooLarge,
    ValidationFailed,
    PreconditionRequired,
    RateLimited,
    Internal,
    CoreDown,
    Migrating,
    Unknown(String),
}

const CODES: [(&str, ErrorCode); 17] = [
    ("bad_request", ErrorCode::BadRequest),
    ("unauthorized", ErrorCode::Unauthorized),
    ("permission_denied", ErrorCode::PermissionDenied),
    ("insufficient_scope", ErrorCode::InsufficientScope),
    ("forbidden_method", ErrorCode::ForbiddenMethod),
    ("forbidden_origin", ErrorCode::ForbiddenOrigin),
    ("not_found", ErrorCode::NotFound),
    ("conflict", ErrorCode::Conflict),
    ("client_id_conflict", ErrorCode::ClientIdConflict),
    ("precondition_failed", ErrorCode::PreconditionFailed),
    ("payload_too_large", ErrorCode::PayloadTooLarge),
    ("validation_failed", ErrorCode::ValidationFailed),
    ("precondition_required", ErrorCode::PreconditionRequired),
    ("rate_limited", ErrorCode::RateLimited),
    ("internal", ErrorCode::Internal),
    ("core_down", ErrorCode::CoreDown),
    ("migrating", ErrorCode::Migrating),
];

impl ErrorCode {
    pub fn parse(s: &str) -> ErrorCode {
        CODES
            .iter()
            .find(|(name, _)| *name == s)
            .map_or_else(|| ErrorCode::Unknown(s.to_string()), |(_, c)| c.clone())
    }

    pub fn as_str(&self) -> &str {
        match self {
            ErrorCode::Unknown(s) => s,
            known => CODES
                .iter()
                .find(|(_, c)| c == known)
                .map_or("", |(name, _)| name),
        }
    }

    /// Code by HTTP status when the error body was not parsed (per the api.md section 3 table; the generic one is used for 403).
    pub fn from_status(status: u16) -> ErrorCode {
        match status {
            400 => ErrorCode::BadRequest,
            401 => ErrorCode::Unauthorized,
            403 => ErrorCode::InsufficientScope,
            404 => ErrorCode::NotFound,
            409 => ErrorCode::Conflict,
            412 => ErrorCode::PreconditionFailed,
            413 => ErrorCode::PayloadTooLarge,
            422 => ErrorCode::ValidationFailed,
            428 => ErrorCode::PreconditionRequired,
            429 => ErrorCode::RateLimited,
            503 => ErrorCode::CoreDown,
            500..=599 => ErrorCode::Internal,
            _ => ErrorCode::BadRequest,
        }
    }

    /// Whether to retry, per the api.md section 3 table.
    pub fn default_retryable(&self) -> bool {
        matches!(
            self,
            ErrorCode::RateLimited
                | ErrorCode::Internal
                | ErrorCode::CoreDown
                | ErrorCode::Migrating
        )
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(ErrorCode::parse(&String::deserialize(d)?))
    }
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorPayload,
}

#[derive(Deserialize)]
struct ErrorPayload {
    code: ErrorCode,
    #[serde(default)]
    message: String,
    #[serde(default)]
    details: Value,
    #[serde(default)]
    retryable: Option<bool>,
    #[serde(default)]
    request_id: Option<String>,
}

/// A parsed core response with a status >= 400.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub status: u16,
    pub code: ErrorCode,
    pub message: String,
    pub details: Value,
    pub retryable: bool,
    /// From the body (`error.request_id`) or the `X-Request-Id` header.
    pub request_id: Option<String>,
    /// From the `Retry-After` header.
    pub retry_after: Option<Duration>,
}

impl ApiError {
    /// Parses the body per the api.md section 3 format; a body that is not JSON or has no `error` gives a code by status, the text of a foreign body is not used.
    pub fn from_response(
        status: u16,
        body: &[u8],
        request_id_header: Option<&str>,
        retry_after_header: Option<&str>,
    ) -> ApiError {
        let retry_after = retry_after_header
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let header_id = request_id_header.map(str::to_string);
        match serde_json::from_slice::<ErrorEnvelope>(body) {
            Ok(ErrorEnvelope { error: e }) => ApiError {
                status,
                retryable: e.retryable.unwrap_or_else(|| e.code.default_retryable()),
                code: e.code,
                message: e.message,
                details: e.details,
                request_id: e.request_id.or(header_id),
                retry_after,
            },
            Err(_) => {
                let code = ErrorCode::from_status(status);
                ApiError {
                    status,
                    retryable: code.default_retryable(),
                    code,
                    message: format!("Core response not parsed (HTTP {status})"),
                    details: json!({}),
                    request_id: header_id,
                    retry_after,
                }
            }
        }
    }

    /// `details.reason` if present (`token_in_url`, `already_decided`).
    pub fn reason(&self) -> Option<&str> {
        self.details.get("reason").and_then(Value::as_str)
    }
}
