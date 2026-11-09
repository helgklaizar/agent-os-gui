//! Token verification port: the implementation is supplied by `bin` (`agentd-auth`), in tests a separate implementation in `tests/`.

use std::future::Future;
use std::pin::Pin;

use crate::{Scope, TokenKind};

impl Principal {
    /// Web token with decision rights: kind `web`, scope `decide`.
    #[must_use]
    pub fn can_decide(&self) -> bool {
        self.kind == crate::TokenKind::Web && self.scope == crate::Scope::Decide
    }
}

/// Who came: the outcome of a successful token check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub token_id: i64,
    pub kind: TokenKind,
    pub scope: Scope,
    /// Allowed methods (`kind = agent|module`), otherwise `None`.
    pub allowlist: Option<Vec<String>>,
    /// When the token stops being valid (unix seconds); `None` — no expiry. An open WS is closed with code `4401`.
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthFailure {
    /// Token unknown, expired or revoked (not distinguished from outside).
    Invalid,
    /// Token store unavailable (the core answers `503 core_down`).
    Unavailable,
}

pub type AuthFuture<'a> = Pin<Box<dyn Future<Output = Result<Principal, AuthFailure>> + Send + 'a>>;

pub trait Authenticator: Send + Sync + 'static {
    fn authenticate<'a>(&'a self, token: &'a str) -> AuthFuture<'a>;
}

#[cfg(test)]
mod grant_tests {
    use super::*;
    use crate::{Scope, TokenKind};

    fn who(kind: TokenKind, scope: Scope) -> Principal {
        Principal {
            token_id: 1,
            kind,
            scope,
            allowlist: None,
            expires_at: None,
        }
    }

    #[test]
    fn can_decide_needs_web_kind_and_decide_scope() {
        assert!(who(TokenKind::Web, Scope::Decide).can_decide());
        assert!(!who(TokenKind::Web, Scope::Chat).can_decide());
        assert!(!who(TokenKind::Web, Scope::Read).can_decide());
        for kind in [
            TokenKind::Gui,
            TokenKind::External,
            TokenKind::Agent,
            TokenKind::Module,
            TokenKind::Test,
        ] {
            assert!(!who(kind, Scope::Decide).can_decide(), "{kind:?}");
            assert!(!who(kind, Scope::Chat).can_decide(), "{kind:?}");
        }
    }

    #[test]
    fn scope_order_puts_decide_between_chat_and_admin() {
        assert!(Scope::Decide.satisfies(Scope::Chat));
        assert!(Scope::Decide.satisfies(Scope::Read));
        assert!(!Scope::Decide.satisfies(Scope::Admin));
        assert!(Scope::Admin.satisfies(Scope::Decide));
        assert!(!Scope::Chat.satisfies(Scope::Decide));
        assert_eq!(Scope::parse("decide"), Some(Scope::Decide));
        assert_eq!(TokenKind::parse("web"), Some(TokenKind::Web));
        assert_eq!(TokenKind::Web.as_str(), "web");
    }
}
