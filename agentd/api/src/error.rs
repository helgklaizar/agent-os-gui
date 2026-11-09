//! Single error format (docs/reference/api.md 3): `{error: {code, message, details, retryable, request_id}}`.
//! Handlers return `ApiError`; one layer (`layers::envelope`) builds the body with `request_id` so the field is always present.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Value,
    pub retryable: bool,
    pub retry_after: Option<u64>,
    /// Allowed methods for the `Allow` header (`405`).
    pub allow: Option<String>,
}

impl ApiError {
    #[must_use]
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            code,
            message: message.into(),
            details: json!({}),
            retryable: false,
            retry_after: None,
            allow: None,
        }
    }

    #[must_use]
    pub fn details(mut self, details: Value) -> ApiError {
        self.details = details;
        self
    }

    #[must_use]
    pub fn retry_after(mut self, secs: u64) -> ApiError {
        self.retryable = true;
        self.retry_after = Some(secs.max(1));
        self
    }

    #[must_use]
    pub fn unauthorized() -> ApiError {
        ApiError::new(401, "unauthorized", "A valid token is required")
    }

    #[must_use]
    pub fn forbidden_origin() -> ApiError {
        ApiError::new(
            403,
            "forbidden_origin",
            "Origin or Host is not in the allowed list",
        )
    }

    #[must_use]
    pub fn not_found() -> ApiError {
        ApiError::new(404, "not_found", "Route not found")
    }

    #[must_use]
    pub fn bad_request(message: impl Into<String>) -> ApiError {
        ApiError::new(400, "bad_request", message)
    }

    #[must_use]
    pub fn insufficient_scope(reason: &str, required: &str) -> ApiError {
        ApiError::new(
            403,
            "insufficient_scope",
            "Token does not cover this operation",
        )
        .details(json!({ "reason": reason, "required": required }))
    }

    #[must_use]
    pub fn forbidden_method() -> ApiError {
        ApiError::new(403, "forbidden_method", "Method is not in the token list")
    }

    #[must_use]
    pub fn permission_denied(reason: &str) -> ApiError {
        ApiError::new(403, "permission_denied", "Action denied by policy")
            .details(json!({ "reason": reason }))
    }

    #[must_use]
    pub fn method_not_allowed(allow: &[&str]) -> ApiError {
        let mut e = ApiError::new(
            405,
            "method_not_allowed",
            "Method not supported by the route",
        );
        e.allow = Some(allow.join(", "));
        e
    }

    #[must_use]
    pub fn not_implemented() -> ApiError {
        ApiError::new(
            404,
            "not_found",
            "The route is described in the contract but the core does not implement it yet",
        )
        .details(json!({ "reason": "not_implemented" }))
    }

    #[must_use]
    pub fn conflict(reason: &str) -> ApiError {
        ApiError::new(409, "conflict", "State conflict").details(json!({ "reason": reason }))
    }

    #[must_use]
    pub fn precondition_failed(current_rev: i64) -> ApiError {
        ApiError::new(
            412,
            "precondition_failed",
            "If-Match does not match the current version",
        )
        .details(json!({ "rev": current_rev }))
    }

    /// `412` for resources with a hash instead of a number (files): `details.rev` is the current hash.
    #[must_use]
    pub fn precondition_failed_hash(current: &str) -> ApiError {
        ApiError::new(
            412,
            "precondition_failed",
            "If-Match does not match the current version",
        )
        .details(json!({ "rev": current }))
    }

    #[must_use]
    pub fn precondition_required() -> ApiError {
        ApiError::new(428, "precondition_required", "If-Match header is required")
    }

    #[must_use]
    pub fn payload_too_large(limit: usize) -> ApiError {
        ApiError::new(
            413,
            "payload_too_large",
            "Request body exceeds the allowed size",
        )
        .details(json!({ "limit_bytes": limit }))
    }

    /// `422`: `details.fields` is a list of `{field, message}`.
    #[must_use]
    pub fn validation_failed(fields: &[agentd_core::FieldError]) -> ApiError {
        ApiError::new(422, "validation_failed", "Fields failed validation")
            .details(json!({ "fields": fields }))
    }

    #[must_use]
    pub fn internal() -> ApiError {
        let mut e = ApiError::new(500, "internal", "Internal core error");
        e.retryable = true;
        e
    }

    #[must_use]
    pub fn core_down(message: &str) -> ApiError {
        ApiError::new(503, "core_down", message).retry_after(1)
    }

    /// `503 migrating` with the migration object in `details.migration`.
    #[must_use]
    pub fn migrating(migration: serde_json::Value) -> ApiError {
        ApiError::new(503, "migrating", "The core is applying database migrations")
            .details(json!({ "migration": migration }))
            .retry_after(2)
    }

    /// Response body; `request_id` is filled in by the envelope layer.
    pub(crate) fn body(&self, request_id: &str) -> Value {
        json!({ "error": {
            "code": self.code,
            "message": self.message,
            "details": self.details,
            "retryable": self.retryable,
            "request_id": request_id,
        } })
    }

    pub(crate) fn allow_header(&self) -> Option<HeaderValue> {
        self.allow
            .as_deref()
            .and_then(|a| HeaderValue::from_str(a).ok())
    }

    pub(crate) fn retry_after_header(&self) -> Option<HeaderValue> {
        self.retry_after
            .and_then(|s| HeaderValue::from_str(&s.to_string()).ok())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut resp = Response::new(axum::body::Body::empty());
        *resp.status_mut() = self.status;
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        resp.extensions_mut().insert(self);
        resp
    }
}
