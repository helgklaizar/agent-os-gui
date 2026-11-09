//! Synchronous wrapper over [`HttpClient`] for consumers without tokio (`ui`): its own runtime, a call blocks the thread.
//!
//! Call only from a worker thread, not from the GTK main loop or from a tokio async context (`block_on` panics there).

use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use crate::api::{Health, PersonasPage, Theme, ThemePage};
use crate::client::HttpClient;
use crate::error::ClientError;
use crate::token::Token;
use crate::transport::Endpoint;

pub struct BlockingClient {
    http: HttpClient,
    runtime: Runtime,
}

impl BlockingClient {
    pub fn new(endpoint: Endpoint, token: Token) -> Result<BlockingClient, ClientError> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ClientError::Transport(format!("runtime not created: {e}")))?;
        Ok(BlockingClient {
            http: HttpClient::new(endpoint, Some(token)),
            runtime,
        })
    }

    /// Runs an async call on the HTTP client in the wrapper's runtime (for commands not among the methods below).
    pub fn run<'a, F, T>(&'a self, call: impl FnOnce(&'a HttpClient) -> F) -> T
    where
        F: std::future::Future<Output = T> + 'a,
    {
        self.runtime.block_on(call(&self.http))
    }

    /// `GET /v1/settings/theme`.
    pub fn theme(&self) -> Result<ThemePage, ClientError> {
        self.runtime.block_on(self.http.theme())
    }

    /// `PATCH /v1/settings/theme` with `If-Match: <rev>`; `412` and `428` arrive as [`ClientError::Api`].
    pub fn set_theme(&self, rev: i64, theme: Theme) -> Result<ThemePage, ClientError> {
        self.runtime.block_on(self.http.set_theme(rev, theme))
    }

    /// `GET /v1/health` (core version for the About dialog).
    pub fn health(&self) -> Result<Health, ClientError> {
        self.runtime.block_on(self.http.health())
    }

    /// `GET /v1/settings/personas`.
    pub fn personas(&self) -> Result<PersonasPage, ClientError> {
        self.runtime.block_on(self.http.personas())
    }

    /// `POST /v1/logout`.
    pub fn logout(&self) -> Result<(), ClientError> {
        self.runtime.block_on(self.http.logout())
    }

    pub fn list_folders(&self) -> Result<Vec<Value>, ClientError> {
        self.runtime.block_on(self.http.list_folders())
    }

    pub fn list_sessions(&self) -> Result<Vec<Value>, ClientError> {
        self.runtime.block_on(self.http.list_sessions())
    }

    pub fn list_personas(&self) -> Result<Vec<Value>, ClientError> {
        self.runtime.block_on(self.http.list_personas())
    }

    pub fn create_folder(
        &self,
        name: &str,
        parent_id: Option<i64>,
        persona_id: Option<i64>,
    ) -> Result<Value, ClientError> {
        self.runtime
            .block_on(self.http.create_folder(name, parent_id, persona_id))
    }

    pub fn create_session(
        &self,
        name: &str,
        folder_id: Option<i64>,
        persona_id: Option<i64>,
    ) -> Result<Value, ClientError> {
        self.runtime
            .block_on(self.http.create_session(name, folder_id, persona_id))
    }

    pub fn create_thread(&self, parent_id: i64, name: &str) -> Result<Value, ClientError> {
        self.runtime
            .block_on(self.http.create_thread(parent_id, name))
    }

    pub fn move_session(&self, id: i64, folder_id: Option<i64>) -> Result<Value, ClientError> {
        self.runtime.block_on(self.http.move_session(id, folder_id))
    }

    pub fn rename_session(&self, id: i64, name: &str) -> Result<Value, ClientError> {
        self.runtime.block_on(self.http.rename_session(id, name))
    }

    pub fn rename_folder(&self, id: i64, name: &str) -> Result<Value, ClientError> {
        self.runtime.block_on(self.http.rename_folder(id, name))
    }

    pub fn delete_session(&self, id: i64) -> Result<(), ClientError> {
        self.runtime.block_on(self.http.delete_session(id))
    }

    pub fn delete_folder(&self, id: i64) -> Result<(), ClientError> {
        self.runtime.block_on(self.http.delete_folder(id))
    }

    pub fn mark_session_read(&self, id: i64) -> Result<(), ClientError> {
        self.runtime.block_on(self.http.mark_session_read(id))
    }

    pub fn list_messages(
        &self,
        session_id: i64,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Value>, ClientError> {
        self.runtime
            .block_on(self.http.list_messages(session_id, before, limit))
    }

    pub fn stop_session(&self, session_id: i64) -> Result<(), ClientError> {
        self.runtime.block_on(self.http.stop_session(session_id))
    }

    pub fn upload_attachment(
        &self,
        session_id: i64,
        filename: &str,
        mime: &str,
        data: &[u8],
    ) -> Result<Value, ClientError> {
        self.runtime.block_on(
            self.http
                .upload_attachment(session_id, filename, mime, data),
        )
    }

    pub fn decide_permission(
        &self,
        request_id: i64,
        decision: &str,
        rule: Option<Value>,
    ) -> Result<Value, ClientError> {
        self.runtime
            .block_on(self.http.decide_permission(request_id, decision, rule))
    }

    pub fn permission_request(&self, request_id: i64) -> Result<Value, ClientError> {
        self.runtime
            .block_on(self.http.permission_request(request_id))
    }

    pub fn patch_session(&self, id: i64, body: Value) -> Result<Value, ClientError> {
        self.runtime.block_on(self.http.patch_session(id, body))
    }

    pub fn list_all_permissions(&self) -> Result<Value, ClientError> {
        self.runtime.block_on(self.http.list_all_permissions())
    }

    pub fn list_events(&self, query: &crate::events::EventQuery) -> Result<Value, ClientError> {
        self.runtime.block_on(self.http.list_events(query))
    }

    pub fn read_event(&self, id: i64) -> Result<(), ClientError> {
        self.runtime.block_on(self.http.read_event(id))
    }

    pub fn read_events(&self, filter: &crate::events::EventsRead) -> Result<i64, ClientError> {
        self.runtime.block_on(self.http.read_events(filter))
    }

    pub fn event_subtypes(&self) -> Result<Vec<Value>, ClientError> {
        self.runtime.block_on(self.http.event_subtypes())
    }
}
