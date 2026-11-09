//! Descriptor passing over a unix socket (`SCM_RIGHTS`) and `memfd` via the safe `rustix` API, without `unsafe`.

use std::io::{self, IoSlice, IoSliceMut};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
};

/// Anonymous in-memory file with the given contents; nothing is on disk. The read position is at the start.
#[cfg(target_os = "linux")]
pub fn memfd_with(name: &str, contents: &[u8]) -> io::Result<OwnedFd> {
    use rustix::fs::{MemfdFlags, memfd_create};
    use std::io::Write;
    let fd = memfd_create(name, MemfdFlags::CLOEXEC)?;
    let mut f = std::fs::File::from(fd);
    f.write_all(contents)?;
    f.sync_all().ok();
    // The descriptor is read via `/proc/self/fd/N` (a fresh open, position 0) or directly: rewind to the start.
    std::io::Seek::rewind(&mut f)?;
    Ok(OwnedFd::from(f))
}

#[cfg(not(target_os = "linux"))]
pub fn memfd_with(_name: &str, _contents: &[u8]) -> io::Result<OwnedFd> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "memfd exists only on Linux",
    ))
}

/// Descriptor holding a secret to hand to another process: on Linux a `memfd` (re-readable), elsewhere a
/// pipe with the write end closed (readable once, nothing on disk). The secret must fit the pipe buffer (64 KiB).
pub fn secret_fd(name: &str, contents: &[u8]) -> io::Result<OwnedFd> {
    #[cfg(target_os = "linux")]
    {
        memfd_with(name, contents)
    }
    #[cfg(not(target_os = "linux"))]
    {
        use std::io::Write;
        let _ = name;
        if contents.len() > 16 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "secret does not fit in the pipe",
            ));
        }
        let (rd, mut wr) = std::io::pipe()?;
        wr.write_all(contents)?;
        drop(wr);
        Ok(OwnedFd::from(rd))
    }
}

/// Sends `msg` together with a descriptor.
pub fn send_fd(sock: &impl AsFd, msg: &[u8], fd: BorrowedFd<'_>) -> io::Result<()> {
    let fds = [fd];
    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    if !ancillary.push(SendAncillaryMessage::ScmRights(&fds)) {
        return Err(io::Error::other("ancillary data buffer too small"));
    }
    sendmsg(
        sock.as_fd(),
        &[IoSlice::new(msg)],
        &mut ancillary,
        SendFlags::empty(),
    )?;
    Ok(())
}

/// Reads a message and, if one arrived, a descriptor. The received fd gets `CLOEXEC`; for it to survive `exec`,
/// the caller clears the flag itself (`make_inheritable`).
pub fn recv_fd(sock: &impl AsFd, buf: &mut [u8]) -> io::Result<(usize, Option<OwnedFd>)> {
    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let msg = recvmsg(
        sock.as_fd(),
        &mut [IoSliceMut::new(buf)],
        &mut ancillary,
        RecvFlags::empty(),
    )?;
    let mut fd = None;
    for m in ancillary.drain() {
        if let RecvAncillaryMessage::ScmRights(mut it) = m {
            fd = fd.or(it.next());
        }
    }
    if let Some(f) = &fd {
        rustix::io::fcntl_setfd(f, rustix::io::FdFlags::CLOEXEC)?;
    }
    Ok((msg.bytes, fd))
}

/// Clears `CLOEXEC`: the descriptor survives `exec`.
pub fn make_inheritable(fd: &OwnedFd) -> io::Result<()> {
    rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::empty())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::io::Read;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    #[cfg(target_os = "linux")]
    #[test]
    fn fd_travels_with_contents_and_msg() {
        let (a, b) = UnixStream::pair().unwrap();
        let fd = memfd_with("t", b"secret-value").unwrap();
        send_fd(&a, b"OK", fd.as_fd()).unwrap();
        drop(fd);
        let mut buf = [0u8; 16];
        let (n, got) = recv_fd(&b, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"OK");
        let mut s = String::new();
        std::fs::File::from(got.expect("descriptor arrived"))
            .read_to_string(&mut s)
            .unwrap();
        assert_eq!(s, "secret-value");
    }

    #[test]
    fn message_without_fd_gives_none() {
        let (mut a, b) = UnixStream::pair().unwrap();
        a.write_all(b"DENY parent").unwrap();
        let mut buf = [0u8; 32];
        let (n, got) = recv_fd(&b, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"DENY parent");
        assert!(got.is_none());
    }
}
