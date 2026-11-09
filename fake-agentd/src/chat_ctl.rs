//! Mock chat control from tests: watcher, permission rules, open requests.

use crate::{DbError, FakeAgentd, chat_state};

impl FakeAgentd {
    /// Watcher is available (default) or not: when unavailable, dangerous actions are blocked (fail-closed).
    pub fn set_observer_available(&self, up: bool) {
        self.shared.chat.set_observer_up(up);
    }

    /// Permission rule directly in the DB, bypassing the loosening check (test scenario setup).
    pub fn seed_permission(
        &self,
        scope: &str,
        scope_id: i64,
        action: &str,
        mode: &str,
    ) -> Result<(), DbError> {
        self.db.write(|w| {
            w.execute(
                "INSERT INTO permissions(scope, scope_id, action, mode) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(scope, scope_id, action) DO UPDATE SET mode = excluded.mode",
                rusqlite::params![scope, scope_id, action, mode],
            )
            .map(|_| ())
        })
    }

    /// Permission requests in `pending` status (`PermissionRequest` per the schema).
    pub fn pending_requests(&self) -> Vec<serde_json::Value> {
        self.shared
            .chat
            .pending()
            .iter()
            .map(chat_state::Req::json)
            .collect()
    }
}
