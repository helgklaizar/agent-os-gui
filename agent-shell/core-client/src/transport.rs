//! Core address and transport: unix socket (primary) or TCP (from outside, through a tunnel).

use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpStream, UnixStream};

use crate::error::ClientError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Unix(PathBuf),
    /// `host:port`.
    Tcp(String),
}

impl Endpoint {
    /// `AGENTD_SOCKET` takes precedence over `AGENTD_ADDR` (`tcp:127.0.0.1:port`); otherwise `$XDG_RUNTIME_DIR/agentd.sock`.
    /// An empty or unrecognised value gives `None`; there is no fallback to another source.
    pub fn from_env_values(
        socket: Option<&str>,
        addr: Option<&str>,
        xdg_runtime_dir: Option<&str>,
    ) -> Option<Endpoint> {
        if let Some(path) = socket {
            return (!path.is_empty()).then(|| Endpoint::Unix(PathBuf::from(path)));
        }
        if let Some(addr) = addr {
            let hostport = addr.strip_prefix("tcp:")?;
            let (host, port) = hostport.rsplit_once(':')?;
            return (!host.is_empty() && port.parse::<u16>().is_ok())
                .then(|| Endpoint::Tcp(hostport.to_string()));
        }
        xdg_runtime_dir
            .filter(|d| !d.is_empty())
            .map(|d| Endpoint::Unix(PathBuf::from(d).join("agentd.sock")))
    }

    /// The same from process environment variables: `AGENTD_SOCKET`, `AGENTD_ADDR`, `XDG_RUNTIME_DIR`.
    pub fn from_env() -> Option<Endpoint> {
        let get = |k: &str| std::env::var(k).ok();
        Endpoint::from_env_values(
            get("AGENTD_SOCKET").as_deref(),
            get("AGENTD_ADDR").as_deref(),
            get("XDG_RUNTIME_DIR").as_deref(),
        )
    }

    /// Value of the `Host` header.
    pub fn host_header(&self) -> String {
        match self {
            Endpoint::Unix(_) => "localhost".to_string(),
            Endpoint::Tcp(addr) => addr.clone(),
        }
    }

    pub(crate) async fn connect(&self) -> Result<Conn, ClientError> {
        let fail = |e: io::Error| {
            ClientError::Transport(format!("connection not opened ({:?})", e.kind()))
        };
        match self {
            Endpoint::Unix(path) => Ok(Conn::Unix(UnixStream::connect(path).await.map_err(fail)?)),
            Endpoint::Tcp(addr) => {
                let s = TcpStream::connect(addr).await.map_err(fail)?;
                s.set_nodelay(true).map_err(fail)?;
                Ok(Conn::Tcp(s))
            }
        }
    }
}

/// A connection to the core: one type over a socket and TCP.
pub enum Conn {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl AsyncRead for Conn {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Unix(s) => Pin::new(s).poll_read(cx, buf),
            Conn::Tcp(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Conn::Unix(s) => Pin::new(s).poll_write(cx, buf),
            Conn::Tcp(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Unix(s) => Pin::new(s).poll_flush(cx),
            Conn::Tcp(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Unix(s) => Pin::new(s).poll_shutdown(cx),
            Conn::Tcp(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}
