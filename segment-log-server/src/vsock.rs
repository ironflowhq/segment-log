//! The vsock listener (ADR 0058). Linux only, and through `libc`, because `std` has no `AF_VSOCK`.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

pub struct Listener(OwnedFd);

fn owned(fd: libc::c_int) -> io::Result<OwnedFd> {
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a non-negative result of socket or accept4 is a new descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn check(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

impl Listener {
    /// Listens on `port` for any CID, so the host loopback (CID 1) and a guest's peer both reach it.
    pub fn bind(port: u32, backlog: libc::c_int) -> io::Result<Listener> {
        // SAFETY: socket has no memory preconditions.
        let fd = owned(unsafe {
            libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0)
        })?;
        // SAFETY: sockaddr_vm is plain data, and all zeroes is a valid value for it.
        let mut addr: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_cid = libc::VMADDR_CID_ANY;
        addr.svm_port = port;
        let len = size_of::<libc::sockaddr_vm>() as libc::socklen_t;
        // SAFETY: addr is a live sockaddr_vm and len is its size.
        check(unsafe { libc::bind(fd.as_raw_fd(), (&raw const addr).cast(), len) })?;
        // SAFETY: fd is a bound socket that this function owns.
        check(unsafe { libc::listen(fd.as_raw_fd(), backlog) })?;
        Ok(Listener(fd))
    }

    pub fn accept(&self) -> io::Result<UnixStream> {
        // SAFETY: null address pointers ask accept4 not to write the peer address.
        let fd = owned(unsafe {
            libc::accept4(
                self.0.as_raw_fd(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_CLOEXEC,
            )
        })?;
        // std has no vsock stream type. UnixStream's read, write, shutdown and try_clone are
        // plain descriptor calls that work on any connected stream socket, and the server never
        // asks it for an address.
        Ok(UnixStream::from(fd))
    }
}
