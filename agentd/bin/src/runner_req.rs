//! Reads the runner request line together with the agent token descriptor (A7-02).

use std::os::fd::OwnedFd;
use std::time::Duration;

use tokio::net::UnixStream;

const MAX_REQUEST: usize = 64 * 1024;

/// Reads the request line and the descriptor that arrived with it (the agent token). Length limit is `MAX_REQUEST`, wait is 5 s.
pub(crate) async fn read_request(stream: &UnixStream) -> Option<(String, Option<OwnedFd>)> {
    let mut buf = vec![0u8; MAX_REQUEST];
    let mut filled = 0usize;
    let mut fd: Option<OwnedFd> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let (n, got) = tokio::time::timeout_at(deadline, async {
            loop {
                stream.readable().await?;
                let r = stream.try_io(tokio::io::Interest::READABLE, || {
                    agentd_sys::fdpass::recv_fd(stream, &mut buf[filled..])
                });
                match r {
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    other => return other,
                }
            }
        })
        .await
        .ok()?
        .ok()?;
        if n == 0 {
            return None;
        }
        fd = fd.or(got);
        let start = filled;
        filled += n;
        if let Some(pos) = buf[start..filled].iter().position(|b| *b == b'\n') {
            return Some((
                String::from_utf8_lossy(&buf[..start + pos]).into_owned(),
                fd,
            ));
        }
        if filled >= MAX_REQUEST {
            return None;
        }
    }
}
