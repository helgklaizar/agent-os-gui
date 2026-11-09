//! Listeners with peer information: `SO_PEERCRED` on the unix socket (uid), address and port on TCP.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use axum::serve::{IncomingStream, Listener};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Unix,
    Tcp,
}

/// What is known about the connection; put into the request as `ConnectInfo<PeerInfo>`.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub transport: Transport,
    /// Peer uid from `SO_PEERCRED` (unix socket only).
    pub uid: Option<u32>,
    /// Port on which the TCP connection was accepted (for the `Host` check).
    pub local_port: Option<u16>,
    pub remote: Option<SocketAddr>,
    /// Becomes `true` when the listener closes: open WS are closed with code `1001`.
    pub closed: watch::Receiver<bool>,
}

pub(crate) struct UnixPeerListener {
    pub inner: UnixListener,
    pub expected_uid: u32,
    pub closed: watch::Receiver<bool>,
}

pub(crate) struct TcpPeerListener {
    pub inner: TcpListener,
    pub closed: watch::Receiver<bool>,
}

async fn accept_pause(e: &io::Error) {
    tracing::warn!(event = "accept_error", kind = ?e.kind(), "accept error");
    tokio::time::sleep(Duration::from_millis(50)).await;
}

impl Listener for UnixPeerListener {
    type Io = UnixStream;
    type Addr = PeerInfo;

    async fn accept(&mut self) -> (UnixStream, PeerInfo) {
        loop {
            let stream = match self.inner.accept().await {
                Ok((s, _)) => s,
                Err(e) => {
                    accept_pause(&e).await;
                    continue;
                }
            };
            match stream.peer_cred() {
                Ok(cred) if cred.uid() == self.expected_uid => {
                    let info = PeerInfo {
                        transport: Transport::Unix,
                        uid: Some(cred.uid()),
                        local_port: None,
                        remote: None,
                        closed: self.closed.clone(),
                    };
                    return (stream, info);
                }
                // Foreign uid or unreadable credentials: the connection is closed without a single response byte.
                Ok(_) => tracing::warn!(
                    event = "peer_uid_rejected",
                    "connection of a foreign user closed"
                ),
                Err(e) => accept_pause(&e).await,
            }
        }
    }

    fn local_addr(&self) -> io::Result<PeerInfo> {
        Ok(PeerInfo {
            transport: Transport::Unix,
            uid: None,
            local_port: None,
            remote: None,
            closed: self.closed.clone(),
        })
    }
}

impl Listener for TcpPeerListener {
    type Io = TcpStream;
    type Addr = PeerInfo;

    async fn accept(&mut self) -> (TcpStream, PeerInfo) {
        loop {
            match self.inner.accept().await {
                Ok((s, remote)) => {
                    let port = self.inner.local_addr().ok().map(|a| a.port());
                    let info = PeerInfo {
                        transport: Transport::Tcp,
                        uid: None,
                        local_port: port,
                        remote: Some(remote),
                        closed: self.closed.clone(),
                    };
                    return (s, info);
                }
                Err(e) => accept_pause(&e).await,
            }
        }
    }

    fn local_addr(&self) -> io::Result<PeerInfo> {
        Ok(PeerInfo {
            transport: Transport::Tcp,
            uid: None,
            local_port: self.inner.local_addr().ok().map(|a| a.port()),
            remote: None,
            closed: self.closed.clone(),
        })
    }
}

impl axum::extract::connect_info::Connected<IncomingStream<'_, UnixPeerListener>> for PeerInfo {
    fn connect_info(stream: IncomingStream<'_, UnixPeerListener>) -> Self {
        stream.remote_addr().clone()
    }
}

impl axum::extract::connect_info::Connected<IncomingStream<'_, TcpPeerListener>> for PeerInfo {
    fn connect_info(stream: IncomingStream<'_, TcpPeerListener>) -> Self {
        stream.remote_addr().clone()
    }
}
