//! The socket server for the segment log (M6, ADR 0058). Not a stable library API: the binary and
//! the fuzz target are its only users.

mod args;
mod connection;
pub mod protocol;

use segment_log_core::SegmentLog;
use std::fs;
use std::io;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Exit status when the log does not open or a flag is invalid (ADR 0058).
const EXIT_START: u8 = 1;

pub fn main_with(args: impl Iterator<Item = String>) -> ExitCode {
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
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let Some(slot) = Slot::take(&open, config.max_connections) else {
            continue; // dropping the stream closes it
        };
        let Ok(writer) = stream.try_clone() else {
            continue;
        };
        connection::serve(stream, writer, Arc::clone(&log), frame_cap, slot);
    }
    ExitCode::SUCCESS
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

/// One of `--max-connections` connection slots, released on drop.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(open: &Arc<AtomicUsize>, max: usize) -> Option<Slot> {
        open.try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < max).then_some(n + 1)
        })
        .ok()
        .map(|_| Slot(Arc::clone(open)))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
