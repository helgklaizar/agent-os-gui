//! Settings page calls: pages `GET/PATCH /v1/settings/{page}` with `If-Match`, secrets (write-only),
//! personas, accounts, modules, tunnels, files. Responses are returned as JSON, parsed by the controllers.

use serde_json::{Value, json};

use crate::client::HttpClient;
use crate::error::ClientError;
use crate::http::Request;
use crate::tools::parse_etag;

/// A settings page: the document without the service `rev`, and `rev` itself for the next `If-Match`.
#[derive(Debug, Clone, PartialEq)]
pub struct PageDoc {
    pub doc: Value,
    pub rev: i64,
}

/// Result of installing a module: `201` installed, `202` waiting for a decision (observer check).
#[derive(Debug, Clone, PartialEq)]
pub enum Installed {
    Done(Value),
    Pending(Value),
}

/// Editor file: text and `rev` from `ETag`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileText {
    pub text: String,
    pub rev: String,
}

fn protocol(what: &str) -> ClientError {
    ClientError::Protocol(format!("{what}: unexpected response"))
}

fn page_doc(resp: &crate::http::RawResponse, what: &str) -> Result<PageDoc, ClientError> {
    let mut doc: Value = resp.json()?;
    let from_body = doc.get("rev").and_then(Value::as_i64);
    let rev = from_body
        .or_else(|| resp.header("etag").and_then(parse_etag))
        .ok_or_else(|| protocol(what))?;
    if let Value::Object(o) = &mut doc {
        o.remove("rev");
    }
    Ok(PageDoc { doc, rev })
}

/// `If-Match` value in quotes, as `ETag` returns it.
fn if_match(rev: i64) -> String {
    format!("\"{rev}\"")
}

/// A path segment from a user value: only safe characters (page name, secret ref, module id).
fn segment(raw: &str, what: &str) -> Result<String, ClientError> {
    let ok = !raw.is_empty()
        && raw.len() <= 80
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok && raw != "." && raw != ".." {
        Ok(raw.to_string())
    } else {
        Err(ClientError::Protocol(format!("{what}: invalid name")))
    }
}

fn enc(raw: &str) -> String {
    let mut out = String::new();
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl HttpClient {
    /// `GET /v1/settings/{page}`.
    pub async fn settings_page(&self, page: &str) -> Result<PageDoc, ClientError> {
        let path = format!("/v1/settings/{}", segment(page, "page")?);
        page_doc(&self.call(Request::get(&path)).await?, "page")
    }

    /// `PATCH /v1/settings/{page}` (JSON merge patch) with `If-Match: <rev>`; `412` and `428` arrive as `ClientError::Api`.
    pub async fn patch_settings_page(
        &self,
        page: &str,
        rev: i64,
        patch: &Value,
    ) -> Result<PageDoc, ClientError> {
        let path = format!("/v1/settings/{}", segment(page, "page")?);
        let req = Request::patch(&path, patch.clone()).header("If-Match", &if_match(rev));
        page_doc(&self.call(req).await?, "page")
    }

    /// `PUT /v1/secrets/{ref}`: the value goes only into the request and is never read back (the response is `{ref, set, updated_at}`).
    pub async fn put_secret(&self, reference: &str, value: &str) -> Result<(), ClientError> {
        let path = format!("/v1/secrets/{}", segment(reference, "secret reference")?);
        self.call(Request::put(&path, json!({ "value": value })))
            .await?;
        Ok(())
    }

    /// `GET /v1/secrets`: `[{ref, set, updated_at}]`, no values.
    pub async fn list_secrets(&self) -> Result<Vec<Value>, ClientError> {
        self.call(Request::get("/v1/secrets")).await?.json()
    }

    pub async fn list_accounts(&self) -> Result<Vec<Value>, ClientError> {
        self.call(Request::get("/v1/accounts")).await?.json()
    }

    pub async fn create_account(&self, body: Value) -> Result<Value, ClientError> {
        self.call(Request::post("/v1/accounts", body)).await?.json()
    }

    pub async fn delete_account(&self, id: i64) -> Result<(), ClientError> {
        self.call(Request::delete(&format!("/v1/accounts/{id}")))
            .await?;
        Ok(())
    }

    /// `POST /v1/accounts/{id}/connect`: the connect flow (`ConnectFlow`).
    pub async fn connect_account(&self, id: i64) -> Result<Value, ClientError> {
        let path = format!("/v1/accounts/{id}/connect");
        self.call(Request::post(&path, json!({}))).await?.json()
    }

    pub async fn create_persona(&self, body: Value) -> Result<Value, ClientError> {
        self.call(Request::post("/v1/personas", body)).await?.json()
    }

    pub async fn patch_persona(&self, id: i64, body: Value) -> Result<Value, ClientError> {
        let path = format!("/v1/personas/{id}");
        self.call(Request::patch(&path, body)).await?.json()
    }

    pub async fn delete_persona(&self, id: i64) -> Result<(), ClientError> {
        self.call(Request::delete(&format!("/v1/personas/{id}")))
            .await?;
        Ok(())
    }

    pub async fn list_modules(&self) -> Result<Vec<Value>, ClientError> {
        self.call(Request::get("/v1/modules")).await?.json()
    }

    /// `POST /v1/modules`: `manifest`, `catalog_id` or `url`; `202` means the install waits for a decision.
    pub async fn install_module(&self, body: Value) -> Result<Installed, ClientError> {
        let resp = self.call(Request::post("/v1/modules", body)).await?;
        let value: Value = resp.json()?;
        Ok(if resp.status == 202 {
            Installed::Pending(value)
        } else {
            Installed::Done(value)
        })
    }

    pub async fn patch_module(&self, id: &str, body: Value) -> Result<Value, ClientError> {
        let path = format!("/v1/modules/{}", segment(id, "module")?);
        self.call(Request::patch(&path, body)).await?.json()
    }

    pub async fn delete_module(&self, id: &str) -> Result<(), ClientError> {
        let path = format!("/v1/modules/{}", segment(id, "module")?);
        self.call(Request::delete(&path)).await?;
        Ok(())
    }

    pub async fn list_tunnels(&self) -> Result<Vec<Value>, ClientError> {
        self.call(Request::get("/v1/tunnels")).await?.json()
    }

    pub async fn create_tunnel(&self, body: Value) -> Result<Value, ClientError> {
        self.call(Request::post("/v1/tunnels", body)).await?.json()
    }

    pub async fn patch_tunnel(&self, id: i64, body: Value) -> Result<Value, ClientError> {
        let path = format!("/v1/tunnels/{id}");
        self.call(Request::patch(&path, body)).await?.json()
    }

    pub async fn delete_tunnel(&self, id: i64) -> Result<(), ClientError> {
        self.call(Request::delete(&format!("/v1/tunnels/{id}")))
            .await?;
        Ok(())
    }

    /// `GET /v1/files`: `[{path, kind, size, rev}]`.
    pub async fn list_files(&self) -> Result<Vec<Value>, ClientError> {
        self.call(Request::get("/v1/files")).await?.json()
    }

    /// `GET /v1/files/content?path=`: text and `rev` from `ETag`.
    pub async fn file_content(&self, path: &str) -> Result<FileText, ClientError> {
        let url = format!("/v1/files/content?path={}", enc(path));
        let resp = self.call(Request::get(&url)).await?;
        let rev = resp
            .header("etag")
            .map(|e| e.trim().trim_matches('"').to_string())
            .ok_or_else(|| protocol("file"))?;
        let text = String::from_utf8(resp.body).map_err(|_| protocol("file"))?;
        Ok(FileText { text, rev })
    }

    /// `PUT /v1/files/content?path=` with `If-Match: "<rev>"`; the response is the new `rev` from `ETag`.
    pub async fn put_file(&self, path: &str, rev: &str, text: &str) -> Result<String, ClientError> {
        let url = format!("/v1/files/content?path={}", enc(path));
        let mut req = Request::put(&url, Value::Null).header("If-Match", &format!("\"{rev}\""));
        req.body = None;
        req.raw = Some(("text/plain; charset=utf-8".into(), text.as_bytes().to_vec()));
        let resp = self.call(req).await?;
        resp.header("etag")
            .map(|e| e.trim().trim_matches('"').to_string())
            .ok_or_else(|| protocol("file"))
    }
}

#[cfg(test)]
mod tests {
    use super::{enc, if_match, segment};

    #[test]
    fn segment_accepts_names_and_refuses_paths() {
        assert_eq!(segment("account.3", "x").ok().as_deref(), Some("account.3"));
        assert!(segment("../x", "x").is_err());
        assert!(segment("a/b", "x").is_err());
        assert!(segment("", "x").is_err());
        assert!(segment("..", "x").is_err());
        assert!(segment("a b", "x").is_err());
    }

    #[test]
    fn enc_escapes_query_text() {
        assert_eq!(enc("memory/a b.md"), "memory/a%20b.md");
        assert_eq!(enc("a&b=c#d"), "a%26b%3Dc%23d");
        assert_eq!(enc("é"), "%C3%A9");
    }

    #[test]
    fn if_match_is_quoted() {
        assert_eq!(if_match(7), "\"7\"");
    }
}
