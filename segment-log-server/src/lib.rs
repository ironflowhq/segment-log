//! The socket server for the segment log (M6, ADR 0058). Not a stable library API: the binary and
//! the fuzz target are its only users.

mod args;
mod connection;
pub mod protocol;
mod server;

use segment_log_core::SegmentLog;
use server::Server;
use std::fs;
use std::io;
use std::net::Shutdown;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Weak};
use std::thread;
use std::time::Duration;

/// Exit status when the log does not open or a flag is invalid (ADR 0058).
const EXIT_START: u8 = 1;

pub fn main_with(args: impl Iterator<Item = String>) -> ExitCode {
    // Before any thread starts, so every thread inherits the mask and only the waiter below
    // receives these signals.
    let stop_signals = block_stop_signals();
    let config = match args::parse(args) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("segment-log-server: {error}\n{}", args::USAGE);
            return ExitCode::from(EXIT_START);
        }
    };
    let frame_cap = protocol::frame_cap(config.options.group_byte_cap);
    // The log first: its LOCK makes this the only server for the directory, so removing a stale
    // socket below cannot take the socket of a live server.
    let log = match SegmentLog::open(&config.dir, config.options) {
        Ok(log) => Arc::new(log),
        Err(error) => {
            eprintln!(
                "segment-log-server: open {}: {error:?}",
                config.dir.display()
            );
            return ExitCode::from(EXIT_START);
        }
    };
    let listener = match bind_unix(&config.unix) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!(
                "segment-log-server: bind {}: {error}",
                config.unix.display()
            );
            return ExitCode::from(EXIT_START);
        }
    };
    eprintln!("segment-log-server: listening on {}", config.unix.display());
    let server = Server::new(config.max_connections);
    {
        let server = Arc::clone(&server);
        thread::spawn(move || {
            let mut signal = 0;
            // SAFETY: the set is initialized and blocked in this thread; sigwait only writes signal.
            unsafe { libc::sigwait(&stop_signals, &mut signal) };
            server.stop(server::EXIT_STOPPED);
        });
    }
    {
        let (server, log) = (Arc::clone(&server), Arc::downgrade(&log));
        thread::spawn(move || accept(listener, &server, &log, frame_cap));
    }
    let code = server.wait_for_stop();
    server.wait_until_idle(DRAIN_LIMIT);
    // Dropping the last handle commits what is queued, joins the writer and releases the lock,
    // without sealing (ADR 0058). A connection still open past the drain limit keeps a handle;
    // then the process exit releases the lock.
    if let Ok(log) = Arc::try_unwrap(log) {
        drop(log);
    }
    ExitCode::from(code)
}

fn block_stop_signals() -> libc::sigset_t {
    // SAFETY: sigemptyset initializes the set before sigaddset and pthread_sigmask read it, and
    // these calls fail only for an invalid signal number or how argument, which these are not.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    }
}

/// How long a stopping server waits for its connections to send the answers they owe.
const DRAIN_LIMIT: Duration = Duration::from_secs(5);

/// Holds the log weakly, so that after a stop the main thread's handle is the last one.
fn accept(listener: UnixListener, server: &Arc<Server>, log: &Weak<SegmentLog>, frame_cap: usize) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let (Ok(writer), Ok(closer)) = (stream.try_clone(), stream.try_clone()) else {
            continue;
        };
        let close_reads = Box::new(move || {
            let _ = closer.shutdown(Shutdown::Read);
        });
        // Without a slot, dropping the stream closes it.
        if let (Some(slot), Some(log)) = (server.admit(close_reads), log.upgrade()) {
            connection::serve(stream, writer, log, frame_cap, slot);
        }
    }
}

fn bind_unix(path: &Path) -> io::Result<UnixListener> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket()) {
        fs::remove_file(path)?;
    }
    // A umask, not a chmod after bind, so the socket never exists with wider permissions. The
    // umask is process-wide; the writer thread creates files only for appends, and none can
    // arrive before the socket exists.
    // SAFETY: umask has no preconditions and cannot fail.
    let old = unsafe { libc::umask(0o177) };
    let listener = UnixListener::bind(path);
    // SAFETY: as above.
    unsafe { libc::umask(old) };
    listener
}
