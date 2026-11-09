//! Token descriptors for the end-to-end tests.

use agentd_auth::NewToken;
use agentd_core::{Scope as CoreScope, TokenKind};

use super::mk;

/// Method list for agent and module tokens: everything they might need; permissions are limited by `scope`.
pub(super) fn mk_listed(kind: TokenKind) -> NewToken {
    let list = [
        "GET /ws",
        "GET /sessions/{id}",
        "DELETE /sessions/{id}",
        "POST /sessions/{id}/tool-results",
        "GET /permission-requests/{id}",
        "POST /permission-requests/{id}/decision",
        "PUT /permissions/{scope}/{scope_id}/{action}",
        "PUT /secrets/{ref}",
        "PATCH /settings/{page}",
        "PATCH /settings/theme",
        "POST /sessions/{id}/clear",
        "POST /sessions/{id}/messages",
        "DELETE /folders/{id}",
    ];
    NewToken {
        allowlist: Some(list.iter().map(ToString::to_string).collect()),
        ..mk(kind, CoreScope::Chat)
    }
}
