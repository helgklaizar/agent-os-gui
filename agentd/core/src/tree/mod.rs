//! Session tree, personas and accounts (`docs/reference/api.md` 5.1, 5.6; `docs/reference/db.md` 3.1, 3.4): input types with shape validation and ports
//! implemented by `db` and called by `api`. No I/O here.

mod input;
mod parse;
mod parse_admin;

use serde_json::Value;

use crate::{FieldError, PortFuture};

pub use input::{
    AccountInput, FolderInput, MAX_NAME_CHARS, MAX_PATH_CHARS, PersonaInput, SessionFilter,
    SessionInput,
};
pub use parse::{check_path, parse_folder, parse_session, parse_thread};
pub use parse_admin::{parse_account, parse_persona};

/// Tree operation error; texts contain no user data.
#[derive(Debug, PartialEq)]
pub enum TreeError {
    /// No object with this `id` (or it is soft-deleted).
    NotFound,
    /// State conflict: `reason` is `folder_cycle` and the like (`409`).
    Conflict(&'static str),
    /// Fields failed validation (`422`).
    Invalid(Vec<FieldError>),
    Unavailable,
}

impl TreeError {
    #[must_use]
    pub fn field(field: &str, message: &str) -> TreeError {
        TreeError::Invalid(vec![FieldError::new(field, message)])
    }
}

pub type TreeResult<T> = Result<T, TreeError>;

/// Publishing to WS topics (`sessions`, `connections`, `settings`): called only after the transaction commits.
pub trait Notifier: Send + Sync + 'static {
    /// `delta` to topic subscribers.
    fn publish(&self, topic: &str, data: Value);
    /// `event` to topic subscribers (one-off event: message stream, typing indicator); defaults to `delta`.
    fn publish_event(&self, topic: &str, data: Value) {
        self.publish(topic, data);
    }
    /// How many connections are subscribed to the topic (needed to tell whether the user has the session open).
    fn watchers(&self, _topic: &str) -> usize {
        0
    }
    /// Tokens of connections subscribed to the topic (who received the event): needed to accept a command result only from
    /// the recipient.
    fn watcher_tokens(&self, _topic: &str) -> Vec<i64> {
        Vec::new()
    }
}

/// Erasing an account secret when the account is deleted (`docs/reference/db.md` 5); implemented by the secret store.
pub trait SecretEraser: Send + Sync + 'static {
    fn erase<'a>(&'a self, secret_ref: &'a str) -> PortFuture<'a, ()>;
}

/// Store of the session tree, personas and accounts. Objects are returned as `docs/reference/api.md` 6 schemas (`Folder`, `Session`,
/// `Persona`, `Account`): `secret_ref` and `deleted_at` never leave the store.
pub trait TreeStore: Send + Sync + 'static {
    fn list_folders(&self) -> PortFuture<'_, TreeResult<Value>>;
    fn create_folder(&self, input: FolderInput) -> PortFuture<'_, TreeResult<Value>>;
    fn patch_folder(&self, id: i64, input: FolderInput) -> PortFuture<'_, TreeResult<Value>>;
    fn delete_folder(&self, id: i64) -> PortFuture<'_, TreeResult<()>>;

    fn list_sessions(&self, filter: SessionFilter) -> PortFuture<'_, TreeResult<Value>>;
    fn get_session(&self, id: i64) -> PortFuture<'_, TreeResult<Value>>;
    fn create_session(&self, input: SessionInput) -> PortFuture<'_, TreeResult<Value>>;
    fn create_thread(&self, parent_id: i64, name: String) -> PortFuture<'_, TreeResult<Value>>;
    fn patch_session(&self, id: i64, input: SessionInput) -> PortFuture<'_, TreeResult<Value>>;
    fn delete_session(&self, id: i64) -> PortFuture<'_, TreeResult<()>>;
    /// Resets `unread`; the delta is sent only if the counter was non-zero.
    fn mark_read(&self, id: i64) -> PortFuture<'_, TreeResult<()>>;
    /// Grows on an agent message in an inactive session.
    fn bump_unread(&self, id: i64) -> PortFuture<'_, TreeResult<()>>;

    fn list_personas(&self) -> PortFuture<'_, TreeResult<Value>>;
    fn create_persona(&self, input: PersonaInput) -> PortFuture<'_, TreeResult<Value>>;
    fn patch_persona(&self, id: i64, input: PersonaInput) -> PortFuture<'_, TreeResult<Value>>;
    fn delete_persona(&self, id: i64) -> PortFuture<'_, TreeResult<()>>;

    fn list_accounts(&self) -> PortFuture<'_, TreeResult<Value>>;
    fn create_account(&self, input: AccountInput) -> PortFuture<'_, TreeResult<Value>>;
    fn patch_account(&self, id: i64, input: AccountInput) -> PortFuture<'_, TreeResult<Value>>;
    fn delete_account(&self, id: i64) -> PortFuture<'_, TreeResult<()>>;
    /// Account connection status after key verification (`connected` or `error`); a `connections` delta only on change.
    fn set_account_status(&self, id: i64, status: &'static str) -> PortFuture<'_, TreeResult<()>>;
    /// Links an account to its secret (`secret_ref`) or unlinks it; `NotFound` if there is no such account.
    fn link_account_secret(
        &self,
        id: i64,
        secret_ref: Option<String>,
    ) -> PortFuture<'_, TreeResult<()>>;

    /// `snapshot` of the `sessions` topic: `{folders, sessions}`.
    fn sessions_snapshot(&self) -> PortFuture<'_, TreeResult<Value>>;
    /// `snapshot` of the `connections` topic: `{items}`.
    fn connections_snapshot(&self) -> PortFuture<'_, TreeResult<Value>>;
}
