//! HTTP client for providers: TLS via rustls only, timeouts for connect, first event and stream idle.

use std::collections::VecDeque;
use std::time::Duration;

use agentd_core::ProviderError;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{HeaderMap, Request};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use tokio::time::timeout;

use crate::sse::{SseEvent, SseParser};

/// Defaults: connect 10 s, first event 60 s, stream idle 120 s.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub connect: Duration,
    pub first_event: Duration,
    pub idle: Duration,
}

impl Default for Timeouts {
    fn default() -> Timeouts {
        Timeouts {
            connect: Duration::from_secs(10),
            first_event: Duration::from_secs(60),
            idle: Duration::from_secs(120),
        }
    }
}

type Conn = HttpsConnector<HttpConnector>;

/// Response event stream: waiting for the first event is bounded by `first_event`, afterwards idle time by `idle`.
pub struct SseStream {
    body: hyper::body::Incoming,
    parser: SseParser,
    queue: VecDeque<SseEvent>,
    first: bool,
    timeouts: Timeouts,
}

impl SseStream {
    /// Next event; `None` means the server finished the response.
    pub async fn next(&mut self) -> Result<Option<SseEvent>, ProviderError> {
        loop {
            if let Some(ev) = self.queue.pop_front() {
                return Ok(Some(ev));
            }
            let wait = if self.first {
                self.timeouts.first_event
            } else {
                self.timeouts.idle
            };
            match timeout(wait, self.body.frame()).await {
                Err(_) => return Err(timeout_err()),
                Ok(None) => return Ok(None),
                Ok(Some(Err(_))) => return Err(network_err()),
                Ok(Some(Ok(frame))) => {
                    if let Some(data) = frame.data_ref() {
                        self.first = false;
                        self.queue.extend(self.parser.feed(data));
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct HttpClient {
    timeouts: Timeouts,
    client: Client<Conn, Full<Bytes>>,
}

/// Response before reading the body: status and headers.
pub struct Reply {
    pub status: u16,
    pub headers: HeaderMap,
}

fn connector(connect: Duration) -> Conn {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    http.set_connect_timeout(Some(connect));
    match HttpsConnectorBuilder::new().with_native_roots() {
        Ok(b) => b.https_or_http().enable_http1().wrap_connector(http),
        Err(_) => {
            // No system roots: https will fail (network error); http to a local server works.
            tracing::warn!(
                scope = "providers",
                event = "no_native_roots",
                "system TLS roots not found"
            );
            let cfg = rustls::ClientConfig::builder()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth();
            HttpsConnectorBuilder::new()
                .with_tls_config(cfg)
                .https_or_http()
                .enable_http1()
                .wrap_connector(http)
        }
    }
}

fn timeout_err() -> ProviderError {
    ProviderError::new("timeout", true)
}

fn network_err() -> ProviderError {
    ProviderError::new("network", true)
}

impl HttpClient {
    #[must_use]
    pub fn new(timeouts: Timeouts) -> HttpClient {
        let client = Client::builder(TokioExecutor::new()).build(connector(timeouts.connect));
        HttpClient { timeouts, client }
    }

    #[must_use]
    pub fn timeouts(&self) -> Timeouts {
        self.timeouts
    }

    fn request(
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Vec<u8>,
    ) -> Result<Request<Full<Bytes>>, ProviderError> {
        let mut b = Request::builder().method(method).uri(url);
        for (k, v) in headers {
            b = b.header(*k, v);
        }
        b.body(Full::new(Bytes::from(body)))
            .map_err(|_| ProviderError::new("bad_response", false))
    }

    async fn send(
        &self,
        req: Request<Full<Bytes>>,
    ) -> Result<hyper::Response<hyper::body::Incoming>, ProviderError> {
        match timeout(self.timeouts.first_event, self.client.request(req)).await {
            Err(_) => Err(timeout_err()),
            Ok(Err(_)) => Err(network_err()),
            Ok(Ok(r)) => Ok(r),
        }
    }

    /// `POST` JSON: after the headers returns the response and the event stream. A non-2xx status is an error per the code table; the body
    /// is not included in messages. Cancellation is dropping the stream: the connection closes.
    pub async fn open_sse(
        &self,
        url: &str,
        headers: &[(&str, String)],
        body: Vec<u8>,
    ) -> Result<(Reply, SseStream), ProviderError> {
        let mut all: Vec<(&str, String)> = vec![
            ("content-type", "application/json".into()),
            ("accept", "text/event-stream".into()),
        ];
        all.extend(headers.iter().map(|(k, v)| (*k, v.clone())));
        let resp = self.send(Self::request("POST", url, &all, body)?).await?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(ProviderError::from_status(status));
        }
        let reply = Reply {
            status,
            headers: resp.headers().clone(),
        };
        let stream = SseStream {
            body: resp.into_body(),
            parser: SseParser::new(),
            queue: VecDeque::new(),
            first: true,
            timeouts: self.timeouts,
        };
        Ok((reply, stream))
    }

    /// Wrapper over `open_sse` with callbacks: `on_event` returns `false` to stop reading.
    pub async fn post_sse(
        &self,
        url: &str,
        headers: &[(&str, String)],
        body: Vec<u8>,
        on_reply: impl FnOnce(&Reply),
        mut on_event: impl FnMut(SseEvent) -> bool,
    ) -> Result<(), ProviderError> {
        let (reply, mut stream) = self.open_sse(url, headers, body).await?;
        on_reply(&reply);
        while let Some(ev) = stream.next().await? {
            if !on_event(ev) {
                break;
            }
        }
        Ok(())
    }

    /// `GET` without a body: the response status.
    pub async fn get_status(
        &self,
        url: &str,
        headers: &[(&str, String)],
    ) -> Result<u16, ProviderError> {
        let resp = self
            .send(Self::request("GET", url, headers, Vec::new())?)
            .await?;
        Ok(resp.status().as_u16())
    }

    /// `POST` JSON with a small reply: status and body (up to `MAX_JSON_REPLY` bytes). For channels (Telegram Bot API).
    pub async fn post_json(
        &self,
        url: &str,
        body: Vec<u8>,
    ) -> Result<(u16, Vec<u8>), ProviderError> {
        let headers = [("content-type", "application/json".to_string())];
        self.call_json("POST", url, &headers, body).await
    }

    /// Request with an arbitrary method and headers (`Authorization` for Whapi); the response is as for `post_json`.
    pub async fn call_json(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Vec<u8>,
    ) -> Result<(u16, Vec<u8>), ProviderError> {
        let resp = self
            .send(Self::request(method, url, headers, body)?)
            .await?;
        let status = resp.status().as_u16();
        let collected = timeout(
            self.timeouts.first_event,
            http_body_util::Limited::new(resp.into_body(), MAX_JSON_REPLY).collect(),
        )
        .await
        .map_err(|_| timeout_err())?
        .map_err(|_| ProviderError::new("bad_response", false))?;
        Ok((status, collected.to_bytes().to_vec()))
    }
}

/// `post_json` response limit: channel replies are small, anything larger signals a fault.
const MAX_JSON_REPLY: usize = 1024 * 1024;
