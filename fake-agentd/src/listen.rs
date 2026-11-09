//! Accepting connections (unix socket with uid check, TCP) and the GUI token file.

use std::path::Path;
use std::sync::Arc;

use tokio::net::{TcpListener, UnixListener};

use crate::server::{self, Peer};
use crate::state::Shared;
use crate::types::{FakeError, Transport, io};

/// Writes the GUI token to a `0600` file (tests and manual work only: `AGENTD_TOKEN_FILE`).
pub(crate) fn write_token_file(path: &Path, token: &str) -> Result<(), FakeError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(io)?;
    f.write_all(token.as_bytes()).map_err(io)
}

pub(crate) async fn accept_unix(shared: Arc<Shared>, listener: UnixListener, owner_uid: u32) {
    while let Ok((stream, _)) = listener.accept().await {
        let uid = stream.peer_cred().ok().map(|c| c.uid());
        if uid != Some(owner_uid) {
            continue;
        }
        let peer = Peer {
            transport: Transport::Unix,
            uid,
            ip: None,
        };
        tokio::spawn(server::serve(shared.clone(), stream, peer));
    }
}

pub(crate) async fn accept_tcp(shared: Arc<Shared>, listener: TcpListener) {
    while let Ok((stream, addr)) = listener.accept().await {
        let peer = Peer {
            transport: Transport::Tcp,
            uid: None,
            ip: Some(addr.ip()),
        };
        tokio::spawn(server::serve(shared.clone(), stream, peer));
    }
}
