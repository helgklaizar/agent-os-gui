//! Reading the request body with a size limit and parsing `If-Match` (docs/reference/api.md 3).

use axum::body::Body;
use axum::http::{HeaderMap, header};
use http_body_util::{BodyExt, LengthLimitError, Limited};
use serde_json::Value;

use crate::error::ApiError;

/// Request body limit (docs/reference/api.md 9.2: working value 1 MiB). Attachments get their own limit in A3.
pub(crate) const BODY_LIMIT: usize = 1024 * 1024;

/// Body with its own limit (message: 1 MiB of text plus JSON framing).
pub(crate) async fn read_bytes_limited(body: Body, limit: usize) -> Result<Vec<u8>, ApiError> {
    match Limited::new(body, limit).collect().await {
        Ok(c) => Ok(c.to_bytes().to_vec()),
        Err(e) if e.downcast_ref::<LengthLimitError>().is_some() => {
            Err(ApiError::payload_too_large(limit))
        }
        Err(_) => Err(ApiError::bad_request("Request body could not be read")),
    }
}

/// Body as JSON; empty or non-JSON gives `400 bad_request`.
pub(crate) async fn read_json(body: Body) -> Result<Value, ApiError> {
    read_json_limited(body, BODY_LIMIT).await
}

pub(crate) async fn read_json_limited(body: Body, limit: usize) -> Result<Value, ApiError> {
    let bytes = read_bytes_limited(body, limit).await?;
    serde_json::from_slice(&bytes).map_err(|_| ApiError::bad_request("Request body is not JSON"))
}

/// `If-Match: <rev>`: an integer, quotes and weak form (`W/"3"`) allowed. No header gives `428`, garbage `400`.
pub(crate) fn if_match_rev(headers: &HeaderMap) -> Result<i64, ApiError> {
    let raw = headers
        .get(header::IF_MATCH)
        .ok_or_else(ApiError::precondition_required)?;
    raw.to_str()
        .ok()
        .map(str::trim)
        .map(|s| s.strip_prefix("W/").unwrap_or(s))
        .map(|s| s.trim_matches('"'))
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|rev| *rev >= 0)
        .ok_or_else(|| ApiError::bad_request("If-Match: integer rev expected"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hm(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::IF_MATCH, v.parse().unwrap());
        h
    }

    #[test]
    fn if_match_forms() {
        for ok in ["3", "\"3\"", "W/\"3\"", " 3 "] {
            assert_eq!(if_match_rev(&hm(ok)).unwrap(), 3, "{ok}");
        }
        for bad in ["abc", "-1", "\"\"", "3.5", "*"] {
            assert_eq!(if_match_rev(&hm(bad)).unwrap_err().status, 400, "{bad}");
        }
        assert_eq!(if_match_rev(&HeaderMap::new()).unwrap_err().status, 428);
    }
}
