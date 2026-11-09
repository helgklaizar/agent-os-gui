//! WS connection to the core: handshake with the token and `X-Request-Id`, reading frames.

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{WebSocketStream, client_async};

use crate::api::{ApiError, ClientOp};
use crate::error::ClientError;
use crate::token::Token;
use crate::transport::{Conn, Endpoint};

/// What the connection consumer sees: a text frame or a close with a code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsIn {
    Text(String),
    Close(Option<u16>),
}

pub struct WsConn {
    ws: WebSocketStream<Conn>,
}

fn protocol(what: &str) -> ClientError {
    ClientError::Protocol(what.to_string())
}

impl WsConn {
    /// `path_and_query` is usually `/v1/ws`. A handshake response other than 101 gives [`ClientError::Api`].
    pub async fn connect(
        endpoint: &Endpoint,
        token: Option<&Token>,
        path_and_query: &str,
    ) -> Result<WsConn, ClientError> {
        let conn = endpoint.connect().await?;
        let mut request = format!("ws://{}{}", endpoint.host_header(), path_and_query)
            .into_client_request()
            .map_err(|_| protocol("WS address not built"))?;
        let h = request.headers_mut();
        h.insert(
            "x-request-id",
            uuid::Uuid::now_v7()
                .to_string()
                .parse()
                .map_err(|_| protocol("X-Request-Id"))?,
        );
        if let Some(t) = token {
            h.insert(
                "authorization",
                format!("Bearer {}", t.expose())
                    .parse()
                    .map_err(|_| protocol("token not valid for a header"))?,
            );
        }
        match client_async(request, conn).await {
            Ok((ws, _)) => Ok(WsConn { ws }),
            Err(WsError::Http(resp)) => {
                let find = |n: &str| {
                    resp.headers()
                        .get(n)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string)
                };
                let body = resp.body().as_deref().unwrap_or_default();
                let err = ApiError::from_response(
                    resp.status().as_u16(),
                    body,
                    find("x-request-id").as_deref(),
                    find("retry-after").as_deref(),
                );
                Err(ClientError::Api(Box::new(err)))
            }
            Err(WsError::Io(_) | WsError::ConnectionClosed | WsError::AlreadyClosed) => {
                Err(ClientError::Transport("WS handshake interrupted".into()))
            }
            Err(_) => Err(protocol("WS handshake not accepted")),
        }
    }

    pub async fn send_op(&mut self, op: &ClientOp) -> Result<(), ClientError> {
        self.ws
            .send(Message::text(op.to_json()))
            .await
            .map_err(|_| ClientError::Transport("WS write failed".into()))
    }

    /// Next message; `None` means the stream ended without a close frame. WS-level ping/pong is skipped.
    pub async fn next(&mut self) -> Option<Result<WsIn, ClientError>> {
        loop {
            return match self.ws.next().await? {
                Ok(Message::Text(t)) => Some(Ok(WsIn::Text(t.as_str().to_string()))),
                Ok(Message::Close(frame)) => {
                    Some(Ok(WsIn::Close(frame.map(|f| u16::from(f.code)))))
                }
                Ok(_) => continue,
                Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => None,
                Err(WsError::Io(_)) => {
                    Some(Err(ClientError::Transport("WS read interrupted".into())))
                }
                Err(_) => Some(Err(protocol("WS frame not accepted"))),
            };
        }
    }

    /// Polite close; errors are ignored: the connection is going away anyway.
    pub async fn close(&mut self) {
        let _ = self.ws.close(None).await;
    }
}
