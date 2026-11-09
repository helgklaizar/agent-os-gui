//! Tool calls: session tasks, bookmarks and quick links (`If-Match`), tool results,
//! session cleanup and export. Responses are returned as JSON, parsed by the controllers.

use serde_json::{Value, json};

use crate::client::HttpClient;
use crate::error::ClientError;
use crate::http::Request;

/// Link list and `rev` from `ETag`: it is sent in `If-Match` on the next write.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkList {
    pub items: Vec<Value>,
    pub rev: i64,
}

fn protocol(what: &str) -> ClientError {
    ClientError::Protocol(format!("{what}: unexpected response"))
}

/// `"3"` or `3` → `3`.
pub fn parse_etag(raw: &str) -> Option<i64> {
    raw.trim().trim_matches('"').parse().ok()
}

fn link_list(resp: &crate::http::RawResponse, what: &str) -> Result<LinkList, ClientError> {
    let items: Vec<Value> = resp.json()?;
    let rev = resp
        .header("etag")
        .and_then(parse_etag)
        .ok_or_else(|| protocol(what))?;
    Ok(LinkList { items, rev })
}

impl HttpClient {
    /// `GET /v1/sessions/{id}/tasks`.
    pub async fn session_tasks(&self, session_id: i64) -> Result<Vec<Value>, ClientError> {
        let path = format!("/v1/sessions/{session_id}/tasks");
        self.call(Request::get(&path)).await?.json()
    }

    /// `POST /v1/sessions/{id}/tasks`: the body is `kind`, `title`, `spec`, `cron?`.
    pub async fn create_task(&self, session_id: i64, body: Value) -> Result<Value, ClientError> {
        let path = format!("/v1/sessions/{session_id}/tasks");
        self.call(Request::post(&path, body)).await?.json()
    }

    /// `PATCH /v1/tasks/{id}`: pause and resume, `{ "state": … }`.
    pub async fn patch_task(&self, id: i64, body: Value) -> Result<Value, ClientError> {
        self.call(Request::patch(&format!("/v1/tasks/{id}"), body))
            .await?
            .json()
    }

    pub async fn delete_task(&self, id: i64) -> Result<(), ClientError> {
        self.call(Request::delete(&format!("/v1/tasks/{id}")))
            .await?;
        Ok(())
    }

    /// `GET /v1/tasks/{id}/runs?limit=…`: newest to oldest.
    pub async fn task_runs(&self, id: i64, limit: u32) -> Result<Vec<Value>, ClientError> {
        let path = format!("/v1/tasks/{id}/runs?limit={limit}");
        self.call(Request::get(&path)).await?.json()
    }

    /// `POST /v1/tasks/{id}/run`: run now, the response is a `TaskRun`.
    pub async fn run_task(&self, id: i64) -> Result<Value, ClientError> {
        self.call(Request::post(&format!("/v1/tasks/{id}/run"), json!({})))
            .await?
            .json()
    }

    /// `GET /v1/sessions/{id}/bookmarks`.
    pub async fn bookmarks(&self, session_id: i64) -> Result<LinkList, ClientError> {
        let path = format!("/v1/sessions/{session_id}/bookmarks");
        link_list(&self.call(Request::get(&path)).await?, "bookmarks")
    }

    /// `PUT /v1/sessions/{id}/bookmarks` with `If-Match: <rev>`; `412` means the list changed, re-read it.
    pub async fn put_bookmarks(
        &self,
        session_id: i64,
        rev: i64,
        items: &[Value],
    ) -> Result<LinkList, ClientError> {
        let path = format!("/v1/sessions/{session_id}/bookmarks");
        let req = Request::put(&path, json!(items)).header("If-Match", &format!("\"{rev}\""));
        link_list(&self.call(req).await?, "bookmarks")
    }

    /// `GET /v1/browser/quicklinks`.
    pub async fn quicklinks(&self) -> Result<LinkList, ClientError> {
        link_list(
            &self.call(Request::get("/v1/browser/quicklinks")).await?,
            "quick links",
        )
    }

    /// `PUT /v1/browser/quicklinks` with `If-Match: <rev>`.
    pub async fn put_quicklinks(&self, rev: i64, items: &[Value]) -> Result<LinkList, ClientError> {
        let req = Request::put("/v1/browser/quicklinks", json!(items))
            .header("If-Match", &format!("\"{rev}\""));
        link_list(&self.call(req).await?, "quick links")
    }

    /// `POST /v1/sessions/{id}/tool-results`: the result of an agent command to the browser (`kind = browser`) or a user action.
    /// Repeating the same body is idempotent (`202`).
    pub async fn post_tool_result(
        &self,
        session_id: i64,
        body: Value,
    ) -> Result<Value, ClientError> {
        let path = format!("/v1/sessions/{session_id}/tool-results");
        self.call(Request::post(&path, body)).await?.json()
    }

    /// `POST /v1/sessions/{id}/clear`.
    pub async fn clear_session(&self, session_id: i64) -> Result<(), ClientError> {
        let path = format!("/v1/sessions/{session_id}/clear");
        self.call(Request::post(&path, json!({}))).await?;
        Ok(())
    }

    /// `POST /v1/sessions/{id}/export`: `{ path, size }`.
    pub async fn export_session(&self, session_id: i64) -> Result<Value, ClientError> {
        let path = format!("/v1/sessions/{session_id}/export");
        self.call(Request::post(&path, json!({}))).await?.json()
    }
}

#[cfg(test)]
mod tests {
    use super::parse_etag;

    #[test]
    fn etag_is_parsed_with_and_without_quotes() {
        assert_eq!(parse_etag("\"7\""), Some(7));
        assert_eq!(parse_etag(" 7 "), Some(7));
        assert_eq!(parse_etag("W/\"7\""), None);
        assert_eq!(parse_etag(""), None);
    }
}
