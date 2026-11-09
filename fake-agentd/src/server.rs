//! Mock HTTP server: unix socket and TCP, `Origin`/`Host` checks, token, routes, upgrade to WS.

use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;

use hyper::body::{Body as HyperBody, Incoming};
use hyper::header::{CONNECTION, UPGRADE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{HeaderMap, Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;

use crate::auth::{AuthFail, authenticate, source_key};
use crate::config::CoreMode;
use crate::handlers::{Ctx, dispatch};
use crate::reply::{ApiErr, Body, VERSION, build, from_err, from_reply};
use crate::routes::{authorize, find};
use crate::state::Shared;
use crate::{RequestRecord, Transport, ws};

const BODY_LIMIT: usize = 1 << 20;

#[derive(Debug, Clone, Copy)]
pub struct Peer {
    pub transport: Transport,
    pub uid: Option<u32>,
    pub ip: Option<IpAddr>,
}

pub async fn serve<S>(shared: Arc<Shared>, io: S, peer: Peer)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let svc = service_fn(move |req| handle(shared.clone(), peer, req));
    let _ = http1::Builder::new()
        .serve_connection(TokioIo::new(io), svc)
        .with_upgrades()
        .await;
}

fn request_id(headers: &HeaderMap) -> String {
    let given = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| uuid::Uuid::parse_str(v).is_ok());
    given.map_or_else(|| uuid::Uuid::now_v7().to_string(), str::to_string)
}

fn origin_allowed(shared: &Shared, headers: &HeaderMap) -> bool {
    let port = shared.tcp_port();
    let mut origins = vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ];
    origins.extend(shared.allowed_origins.iter().cloned());
    let mut hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    hosts.extend(
        shared
            .allowed_origins
            .iter()
            .filter_map(|o| o.split_once("://").map(|(_, h)| h.to_string())),
    );
    let host_ok = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|h| hosts.iter().any(|a| a == h));
    let origin_ok = headers.get("origin").is_none_or(|v| {
        v.to_str()
            .ok()
            .is_some_and(|o| origins.iter().any(|a| a == o))
    });
    host_ok && origin_ok
}

fn health(shared: &Shared) -> Value {
    let (status, migration) = match shared.mode() {
        CoreMode::Ok => ("ok", Value::Null),
        CoreMode::Down => ("starting", Value::Null),
        CoreMode::Migrating => (
            "migrating",
            json!({ "status": "running", "from": 0, "to": 1, "step": "0001_init", "progress": 0.5 }),
        ),
    };
    // `user_version` is the real `PRAGMA user_version` of the mock DB; `null` if it could not be read (as in the core)
    let user_version = shared
        .db
        .reader()
        .and_then(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?))
        .ok();
    json!({
        "status": status, "version": VERSION, "api": { "major": shared.api_major, "minor": 0 },
        "min_client": "0.0.0", "min_gui": "0.0.0", "max_gui": "0.x", "user_version": user_version,
        "capabilities": ["if-match", "ws-seq"], "migration": migration,
    })
}

fn mode_error(mode: CoreMode) -> Option<ApiErr> {
    match mode {
        CoreMode::Ok => None,
        CoreMode::Down => Some(ApiErr::new(503, "core_down", "Core is starting").retry_after(1)),
        CoreMode::Migrating => Some(
            ApiErr::new(503, "migrating", "Core is applying migrations")
                .retry_after(1)
                .details(json!({ "migration": { "status": "running", "from": 0, "to": 1, "progress": 0.5 } })),
        ),
    }
}

async fn read_body(mut body: Incoming) -> Result<Vec<u8>, ApiErr> {
    let mut out = Vec::new();
    loop {
        match std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await {
            None => return Ok(out),
            Some(Err(_)) => {
                return Err(ApiErr::new(
                    400,
                    "bad_request",
                    "Request body could not be read",
                ));
            }
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    out.extend_from_slice(&data);
                    if out.len() > BODY_LIMIT {
                        return Err(ApiErr::new(
                            413,
                            "payload_too_large",
                            "Body is larger than 1 MiB",
                        ));
                    }
                }
            }
        }
    }
}

async fn handle(
    shared: Arc<Shared>,
    peer: Peer,
    req: Request<Incoming>,
) -> Result<Response<Body>, io::Error> {
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let rid = request_id(req.headers());
    let resp = route(&shared, peer, &method, &path, req, &rid).await?;
    let record = RequestRecord {
        method,
        path,
        status: resp.status().as_u16(),
        request_id: rid,
        transport: peer.transport,
        peer_uid: peer.uid,
    };
    shared.lock().requests.push(record);
    Ok(resp)
}

fn is_ws_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

async fn route(
    shared: &Arc<Shared>,
    peer: Peer,
    method: &str,
    path: &str,
    req: Request<Incoming>,
    rid: &str,
) -> Result<Response<Body>, io::Error> {
    let tcp = peer.transport == Transport::Tcp;
    let headers = req.headers().clone();
    let query = req.uri().query().map(str::to_string);
    if tcp && !origin_allowed(shared, &headers) {
        return Ok(from_err(
            &ApiErr::new(
                403,
                "forbidden_origin",
                "Origin or Host is not on the allowed list",
            ),
            rid,
        ));
    }
    if method == "GET" && path == "/v1/health" {
        return Ok(build(200, rid, Some(&health(shared)), &[]));
    }
    if let Some(e) = mode_error(shared.mode()) {
        return Ok(from_err(&e, rid));
    }
    let ws = method == "GET" && path == "/v1/ws" && is_ws_upgrade(&headers);
    if ws && shared.hub.is_dropped() {
        return Err(io::Error::other("WS disabled by the drop_ws command"));
    }
    let key = source_key(peer.ip, &headers);
    let auth = authenticate(shared, &headers, query.as_deref(), &key);
    if ws {
        return upgrade(shared, req, &headers, auth, rid);
    }
    let who = match auth {
        Ok(w) => w,
        Err(f) => return Ok(from_err(&f.into_api(), rid)),
    };
    let Some((route, id)) = path.strip_prefix("/v1").and_then(|p| find(method, p)) else {
        return Ok(from_err(
            &ApiErr::new(404, "not_found", "Route not found"),
            rid,
        ));
    };
    if let Err(e) = authorize(route, &who, tcp) {
        return Ok(from_err(&e, rid));
    }
    let body = match read_body(req.into_body()).await {
        Ok(b) => b,
        Err(e) => return Ok(from_err(&e, rid)),
    };
    let ctx = Ctx {
        shared,
        who: &who,
        path,
        headers: &headers,
        body: &body,
        query: query.as_deref(),
    };
    Ok(match dispatch(route, id, &ctx) {
        Ok(r) => from_reply(&r, rid),
        Err(e) => from_err(&e, rid),
    })
}

fn upgrade(
    shared: &Arc<Shared>,
    req: Request<Incoming>,
    headers: &HeaderMap,
    auth: Result<crate::auth::Authed, AuthFail>,
    rid: &str,
) -> Result<Response<Body>, io::Error> {
    let who = match auth {
        Ok(a) => Some(a.id),
        Err(AuthFail::Missing) => return Ok(from_err(&AuthFail::Missing.into_api(), rid)),
        Err(AuthFail::RateLimited(s)) => {
            return Ok(from_err(&AuthFail::RateLimited(s).into_api(), rid));
        }
        Err(e @ AuthFail::Store(_)) => return Ok(from_err(&e.into_api(), rid)),
        Err(_) => None,
    };
    let Some(key) = headers.get("sec-websocket-key") else {
        return Ok(from_err(
            &ApiErr::new(400, "bad_request", "No Sec-WebSocket-Key"),
            rid,
        ));
    };
    let accept = derive_accept_key(key.as_bytes());
    let shared = shared.clone();
    tokio::spawn(async move {
        if let Ok(up) = hyper::upgrade::on(req).await {
            let stream =
                WebSocketStream::from_raw_socket(TokioIo::new(up), Role::Server, None).await;
            ws::run(&shared, stream, who).await;
        }
    });
    Response::builder()
        .status(101)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header("sec-websocket-accept", accept)
        .header("x-request-id", rid)
        .header("x-agentd-version", VERSION)
        .body(Body::empty())
        .map_err(|e| io::Error::other(e.to_string()))
}
