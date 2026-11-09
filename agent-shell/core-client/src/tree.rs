//! Session tree calls: folders, sessions, threads, personas (read), `POST /sessions/{id}/read`.
//! Responses are returned as JSON: parsed by the controllers (`controllers::tree_types`), the core client does not duplicate their shapes.

use serde_json::{Value, json};

use crate::client::HttpClient;
use crate::error::ClientError;
use crate::http::Request;

fn id_or_null(id: Option<i64>) -> Value {
    id.map_or(Value::Null, Value::from)
}

impl HttpClient {
    /// `GET /v1/folders`.
    pub async fn list_folders(&self) -> Result<Vec<Value>, ClientError> {
        self.call(Request::get("/v1/folders")).await?.json()
    }

    /// `GET /v1/sessions` (sessions and threads, without archived).
    pub async fn list_sessions(&self) -> Result<Vec<Value>, ClientError> {
        self.call(Request::get("/v1/sessions")).await?.json()
    }

    /// `GET /v1/personas`.
    pub async fn list_personas(&self) -> Result<Vec<Value>, ClientError> {
        self.call(Request::get("/v1/personas")).await?.json()
    }

    pub async fn create_folder(
        &self,
        name: &str,
        parent_id: Option<i64>,
        persona_id: Option<i64>,
    ) -> Result<Value, ClientError> {
        let body = json!({ "name": name, "parent_id": id_or_null(parent_id), "persona_id": id_or_null(persona_id) });
        self.call(Request::post("/v1/folders", body)).await?.json()
    }

    pub async fn create_session(
        &self,
        name: &str,
        folder_id: Option<i64>,
        persona_id: Option<i64>,
    ) -> Result<Value, ClientError> {
        let body = json!({ "name": name, "folder_id": id_or_null(folder_id), "persona_id": id_or_null(persona_id) });
        self.call(Request::post("/v1/sessions", body)).await?.json()
    }

    pub async fn create_thread(&self, parent_id: i64, name: &str) -> Result<Value, ClientError> {
        let path = format!("/v1/sessions/{parent_id}/threads");
        self.call(Request::post(&path, json!({ "name": name })))
            .await?
            .json()
    }

    /// "Move": the body has only `folder_id`, the name is untouched.
    pub async fn move_session(
        &self,
        id: i64,
        folder_id: Option<i64>,
    ) -> Result<Value, ClientError> {
        let body = json!({ "folder_id": id_or_null(folder_id) });
        self.call(Request::patch(&format!("/v1/sessions/{id}"), body))
            .await?
            .json()
    }

    /// "Rename": the body has only `name`, the folder is untouched.
    pub async fn rename_session(&self, id: i64, name: &str) -> Result<Value, ClientError> {
        self.call(Request::patch(
            &format!("/v1/sessions/{id}"),
            json!({ "name": name }),
        ))
        .await?
        .json()
    }

    pub async fn rename_folder(&self, id: i64, name: &str) -> Result<Value, ClientError> {
        self.call(Request::patch(
            &format!("/v1/folders/{id}"),
            json!({ "name": name }),
        ))
        .await?
        .json()
    }

    /// `PATCH /v1/folders/{id}` with arbitrary fields (`persona_id`, `parent_id`, `open`).
    pub async fn patch_folder(&self, id: i64, body: Value) -> Result<Value, ClientError> {
        self.call(Request::patch(&format!("/v1/folders/{id}"), body))
            .await?
            .json()
    }

    /// `PATCH /v1/sessions/{id}` with arbitrary fields (`path`, `cfg`, `persona_id`).
    pub async fn patch_session(&self, id: i64, body: Value) -> Result<Value, ClientError> {
        self.call(Request::patch(&format!("/v1/sessions/{id}"), body))
            .await?
            .json()
    }

    pub async fn delete_session(&self, id: i64) -> Result<(), ClientError> {
        self.call(Request::delete(&format!("/v1/sessions/{id}")))
            .await?;
        Ok(())
    }

    pub async fn delete_folder(&self, id: i64) -> Result<(), ClientError> {
        self.call(Request::delete(&format!("/v1/folders/{id}")))
            .await?;
        Ok(())
    }

    /// `POST /v1/sessions/{id}/read`: open the session, reset `unread`.
    pub async fn mark_session_read(&self, id: i64) -> Result<(), ClientError> {
        self.call(Request::post(&format!("/v1/sessions/{id}/read"), json!({})))
            .await?;
        Ok(())
    }
}
