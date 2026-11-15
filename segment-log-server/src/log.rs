//! The log a server serves: the writer, or with `--read-only` a view of a sealed log that writes
//! nothing to the volume (ADR 0060).

use segment_log_core::{
    AppendError, LogOffset, LogReader, Pending, ReadError, Record, SealError, SegmentLog,
    StreamHead, Submit,
};

pub enum Log {
    Writer(SegmentLog),
    Reader(LogReader),
}

impl Log {
    /// A reader answers like a sealed log.
    pub fn append(&self, submit: Submit) -> Result<Pending, AppendError> {
        match self {
            Log::Writer(log) => log.append(submit),
            Log::Reader(_) => Err(AppendError::Sealed),
        }
    }

    pub fn read(&self, from: LogOffset, max: usize) -> Result<Vec<Record>, ReadError> {
        match self {
            Log::Writer(log) => log.read(from, max),
            Log::Reader(log) => log.read(from, max),
        }
    }

    pub fn watermark(&self) -> LogOffset {
        match self {
            Log::Writer(log) => log.watermark(),
            Log::Reader(log) => log.watermark(),
        }
    }

    pub fn stream_head(&self, stream: &[u8]) -> Option<StreamHead> {
        match self {
            Log::Writer(log) => log.stream_head(stream),
            Log::Reader(log) => log.stream_head(stream),
        }
    }

    /// Returns whether the server should stop. A writer's work ends with its seal (ADR 0058). A
    /// reader's log is already sealed, as a second seal call finds it, so the reader keeps
    /// serving: no client can stop it.
    pub fn seal(&self) -> Result<bool, SealError> {
        match self {
            Log::Writer(log) => log.seal().map(|()| true),
            Log::Reader(_) => Ok(false),
        }
    }
}
