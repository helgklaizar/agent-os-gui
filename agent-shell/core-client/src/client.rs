//! Core HTTP client: token, `X-Request-Id`, error parsing and typed calls.

use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use serde_json::json;

use crate::api::{
    EchoRequest, EchoResponse, Health, PersonasPage, Theme, ThemePage, TokenCreate, TokenInfo,
    TokenIssued,
};
use crate::error::ClientError;
use crate::http::{RawResponse, Request};
use crate::token::Token;
use crate::transport::Endpoint;

/// Response body limit, 4 MiB.
const MAX_BODY: usize = 4 << 20;

fn wire(_: hyper::Error) -> ClientError {
    ClientError::Transport("HTTP exchange interrupted".into())
}

/// Request id for the log: only if it is a UUID (foreign text does not reach the log).
fn log_request_id(req: &hyper::Request<Full<Bytes>>) -> String {
    req.headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| uuid::Uuid::parse_str(v).is_ok())
        .map_or_else(|| "-".to_string(), str::to_string)
}

#[derive(Debug, Clone)]
pub struct HttpClient {
    endpoint: Endpoint,
    token: Option<Token>,
    timeout: Duration,
}

impl HttpClient {
    pub fn new(endpoint: Endpoint, token: Option<Token>) -> HttpClient {
        HttpClient {
            endpoint,
            token,
            timeout: Duration::from_secs(10),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> HttpClient {
        self.timeout = timeout;
        self
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// One request: a new connection, `Authorization`, `X-Request-Id` (UUID v7). A status >= 400 is not an error.
    pub async fn send(&self, req: Request) -> Result<RawResponse, ClientError> {
        tokio::time::timeout(self.timeout, self.exchange(req))
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    async fn exchange(&self, req: Request) -> Result<RawResponse, ClientError> {
        let conn = self.endpoint.connect().await?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(conn))
            .await
            .map_err(wire)?;
        let driver = tokio::spawn(async move {
            let _ = connection.await;
        });
        let result = self.round_trip(&mut sender, req).await;
        driver.abort();
        result
    }

    fn build(&self, req: &Request) -> Result<hyper::Request<Full<Bytes>>, ClientError> {
        let has = |name: &str| {
            req.headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case(name))
        };
        let mut b = hyper::Request::builder()
            .method(req.method.as_str())
            .uri(req.path.as_str());
        if !has("host") {
            b = b.header("host", self.endpoint.host_header());
        }
        if !has("x-request-id") {
            b = b.header("x-request-id", uuid::Uuid::now_v7().to_string());
        }
        b = b.header("accept", "application/json");
        if req.authorized
            && let Some(t) = &self.token
        {
            b = b.header("authorization", format!("Bearer {}", t.expose()));
        }
        if let Some((ctype, _)) = &req.raw {
            b = b.header("content-type", ctype.as_str());
        } else if req.body.is_some() {
            b = b.header("content-type", "application/json");
        }
        for (k, v) in &req.headers {
            b = b.header(k.as_str(), v.as_str());
        }
        let body = match (&req.raw, &req.body) {
            (Some((_, raw)), _) => Bytes::from(raw.clone()),
            (None, Some(v)) => Bytes::from(v.to_string()),
            (None, None) => Bytes::new(),
        };
        b.body(Full::new(body))
            .map_err(|_| ClientError::Protocol("request not built".into()))
    }

    async fn round_trip(
        &self,
        sender: &mut hyper::client::conn::http1::SendRequest<Full<Bytes>>,
        req: Request,
    ) -> Result<RawResponse, ClientError> {
        let request = self.build(&req)?;
        let rid = log_request_id(&request);
        let started = std::time::Instant::now();
        let resp = sender.send_request(request).await.map_err(wire)?;
        let status = resp.status().as_u16();
        let dur_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        // Only method, status and duration: the path and bodies may carry user names and texts.
        if status >= 400 {
            tracing::warn!(scope = "client", event = "http.error", request_id = %rid, method = %req.method, status, dur_ms, "request failed");
        } else {
            tracing::debug!(scope = "client", event = "http.request", request_id = %rid, method = %req.method, status, dur_ms, "request completed");
        }
        let headers = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
            .collect();
        let body = Limited::new(resp.into_body(), MAX_BODY)
            .collect()
            .await
            .map_err(|_| ClientError::Protocol("response body not read".into()))?;
        Ok(RawResponse {
            status,
            headers,
            body: body.to_bytes().to_vec(),
        })
    }

    /// Like [`send`](Self::send), but a status >= 400 becomes [`ClientError::Api`].
    pub async fn call(&self, req: Request) -> Result<RawResponse, ClientError> {
        let resp = self.send(req).await?;
        if resp.status >= 400 {
            return Err(ClientError::Api(Box::new(resp.api_error())));
        }
        Ok(resp)
    }

    /// `GET /v1/health`, anonymous.
    pub async fn health(&self) -> Result<Health, ClientError> {
        self.call(Request::get("/v1/health").anonymous())
            .await?
            .json()
    }

    pub async fn theme(&self) -> Result<ThemePage, ClientError> {
        self.call(Request::get("/v1/settings/theme")).await?.json()
    }

    /// `PATCH` with `If-Match: <rev>`; `412` and `428` arrive as [`ClientError::Api`].
    pub async fn set_theme(&self, rev: i64, theme: Theme) -> Result<ThemePage, ClientError> {
        let req = Request::patch("/v1/settings/theme", json!({ "theme": theme }))
            .header("If-Match", &rev.to_string());
        self.call(req).await?.json()
    }

    /// `POST /v1/logout` (admin, local only): ends the user's session.
    pub async fn logout(&self) -> Result<(), ClientError> {
        self.call(Request::post("/v1/logout", json!({}))).await?;
        Ok(())
    }

    /// `GET /v1/settings/personas`.
    pub async fn personas(&self) -> Result<PersonasPage, ClientError> {
        self.call(Request::get("/v1/settings/personas"))
            .await?
            .json()
    }

    pub async fn echo(&self, client_id: &str, text: &str) -> Result<EchoResponse, ClientError> {
        let body = serde_json::to_value(EchoRequest {
            client_id: client_id.to_string(),
            text: text.to_string(),
        })
        .map_err(|_| ClientError::Protocol("request body not built".into()))?;
        self.call(Request::post("/v1/_test/echo", body))
            .await?
            .json()
    }

    pub async fn issue_token(&self, create: &TokenCreate) -> Result<TokenIssued, ClientError> {
        let body = serde_json::to_value(create)
            .map_err(|_| ClientError::Protocol("request body not built".into()))?;
        self.call(Request::post("/v1/tokens", body)).await?.json()
    }

    pub async fn list_tokens(&self) -> Result<Vec<TokenInfo>, ClientError> {
        self.call(Request::get("/v1/tokens")).await?.json()
    }

    pub async fn revoke_token(&self, id: i64) -> Result<(), ClientError> {
        self.call(Request::delete(&format!("/v1/tokens/{id}")))
            .await?;
        Ok(())
    }
}
