//! Secrets (`docs/reference/api.md` 5.7): value store port and reference validation. Values are written and erased but not
//! read out; only the provider reads them, at request time.

use serde::Serialize;

use crate::{PortFuture, Secret};

/// Secret value length limit, bytes.
pub const MAX_SECRET_BYTES: usize = 8 * 1024;

/// `GET /secrets` list entry: without the value (`SecretInfo` schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretInfo {
    #[serde(rename = "ref")]
    pub secret_ref: String,
    pub set: bool,
    pub updated_at: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SecretError {
    /// Reference or value failed validation (`422`).
    Invalid(&'static str),
    Unavailable,
}

/// `provider.<name>` or `account.<id>`: name — lowercase Latin letters, digits, `_` and `-`, up to 32 characters; id — integer.
#[must_use]
pub fn valid_secret_ref(r: &str) -> bool {
    let Some((kind, rest)) = r.split_once('.') else {
        return false;
    };
    match kind {
        "provider" => {
            !rest.is_empty()
                && rest.len() <= 32
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        }
        "account" => {
            !rest.is_empty()
                && rest.len() <= 18
                && rest.bytes().all(|b| b.is_ascii_digit())
                && !rest.starts_with('0')
        }
        _ => false,
    }
}

/// Account number from the reference `account.<id>`.
#[must_use]
pub fn account_of_ref(r: &str) -> Option<i64> {
    r.strip_prefix("account.")
        .filter(|_| valid_secret_ref(r))
        .and_then(|n| n.parse().ok())
}

/// Secret store (`keys.env`, implemented by the `providers` crate).
pub trait SecretStore: Send + Sync + 'static {
    fn list(&self) -> PortFuture<'_, Result<Vec<SecretInfo>, SecretError>>;
    /// Writes a value; the reference and value are validated.
    fn put<'a>(
        &'a self,
        secret_ref: &'a str,
        value: Secret<String>,
        now: i64,
    ) -> PortFuture<'a, Result<(), SecretError>>;
    /// `true` if the secret existed.
    fn delete<'a>(&'a self, secret_ref: &'a str) -> PortFuture<'a, Result<bool, SecretError>>;
    /// Value for the provider; it never leaves through the API.
    fn get<'a>(&'a self, secret_ref: &'a str) -> PortFuture<'a, Option<Secret<String>>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_format() {
        for ok in [
            "provider.anthropic",
            "provider.open-ai_2",
            "account.3",
            "account.120",
        ] {
            assert!(valid_secret_ref(ok), "{ok}");
        }
        for bad in [
            "",
            "provider.",
            "provider.A",
            "provider.a b",
            "provider.a/b",
            "provider..",
            "account.",
            "account.0",
            "account.03",
            "account.x",
            "account.-1",
            "keys.env",
            "provider",
            "..",
            "account.1.2",
            "provider.a.b",
        ] {
            assert!(!valid_secret_ref(bad), "{bad}");
        }
        assert!(!valid_secret_ref(&format!("provider.{}", "a".repeat(33))));
        assert_eq!(account_of_ref("account.7"), Some(7));
        assert_eq!(account_of_ref("provider.x"), None);
    }
}
