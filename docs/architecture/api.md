# API sketch

Status: provisional. This is the M1–M4 target for `segment-log-core`. Names can change during implementation. A change in behavior needs an ADR. This file is the only source for the public signatures and the default values.

```rust
use std::path::Path;
use std::time::Duration;

pub type LogOffset = u64;
pub type Revision = u64;

pub struct Options {
    /// How long the writer waits for more submits after the first one.
    pub linger: Duration,          // default 1 ms
    /// Largest group body in bytes. A submit that cannot fit alone gets TooLarge.
    pub group_byte_cap: usize,     // default 1 MiB; at most the group body limit in format.md
    /// Roll the active segment when it reaches this size.
    pub roll_bytes: u64,           // default 64 MiB; at most the roll size limit in format.md
    /// Submits the accept queue holds before append returns Full.
    pub queue_capacity: usize,     // default 4096
}

impl Default for Options { /* the defaults above */ }

/// One log directory, open for writing or read-only after seal.
/// Send + Sync: share it with Arc and call append from many threads.
pub struct SegmentLog { /* … */ }

impl SegmentLog {
    /// Creates the directory or the log if needed, takes the lock, recovers,
    /// makes the kept prefix durable, and starts the writer thread.
    pub fn open(dir: impl AsRef<Path>, options: Options) -> Result<SegmentLog, OpenError>;

    /// Puts the submit in the accept queue and returns at once.
    /// Never blocks on a full queue.
    pub fn append(&self, submit: Submit) -> Result<Pending, AppendError>;

    /// The offset just past the last durable record.
    pub fn watermark(&self) -> LogOffset;

    /// Records in log order from `from`, up to `max` records, never past the watermark.
    pub fn read(&self, from: LogOffset, max: usize) -> Result<Vec<Record>, ReadError>;

    /// The latest durable revision and offset of one stream, from the tail map.
    pub fn stream_head(&self, stream: &[u8]) -> Option<StreamHead>;

    /// Stops admission, commits the queued submits, writes the footer, the
    /// seal frame and the locator, and makes them durable. Takes &self because other threads
    /// can hold the same Arc: from the moment seal starts, their append gets
    /// Sealed. read, watermark and stream_head keep working on the sealed log.
    /// A second call returns Ok(()).
    pub fn seal(&self) -> Result<(), SealError>;
}

/// Dropping the last handle stops admission, lets the writer commit what is
/// already queued, joins the writer thread, and releases the lock. If seal was
/// not called, the log is not sealed.
impl Drop for SegmentLog { /* … */ }

/// A read-only view of a sealed log (ADR 0016). Takes no lock and writes
/// nothing, so it works on a read-only volume and many can open at once.
/// Send + Sync. M4.
pub struct LogReader { /* … */ }

impl LogReader {
    /// Checks the headers and reads the segments without changing any file.
    /// Fails with NotSealed if the log has no seal frame.
    pub fn open(dir: impl AsRef<Path>) -> Result<LogReader, OpenError>;
    pub fn watermark(&self) -> LogOffset;
    pub fn read(&self, from: LogOffset, max: usize) -> Result<Vec<Record>, ReadError>;
    pub fn stream_head(&self, stream: &[u8]) -> Option<StreamHead>;
}

pub struct Submit {
    /// At least one record. The log commits all of them or none.
    pub records: Vec<NewRecord>,
}

pub struct NewRecord {
    pub stream: Vec<u8>,                 // within the stream id limit in format.md
    pub expected: ExpectedRevision,
    pub key: Option<Vec<u8>>,            // within the key limit in format.md when present
    pub payload: Vec<u8>,
}

/// Provisional vocabulary (ADR 0008). There is no Any.
pub enum ExpectedRevision {
    /// The stream has no records. A successful append gets revision 0.
    Empty,
    /// The stream's current last revision is exactly this value.
    At(Revision),
}

/// The handle for one submit in the queue.
pub struct Pending { /* … */ }

impl Pending {
    /// Blocks until the writer answers this submit. If the writer thread stops
    /// without an answer (a panic), returns Indeterminate.
    pub fn wait(self) -> Result<Committed, CommitError>;
}

pub struct Committed {
    /// One entry for each record, in submit order.
    pub records: Vec<Position>,
    /// True when every key matched durable records and nothing was appended.
    pub deduplicated: bool,
}

pub struct Position {
    pub offset: LogOffset,
    pub revision: Revision,
}

pub struct Record {
    pub offset: LogOffset,
    pub stream: Vec<u8>,
    pub revision: Revision,
    pub key: Option<Vec<u8>>,
    pub payload: Vec<u8>,
}

pub struct StreamHead {
    pub revision: Revision,
    pub offset: LogOffset,
}

/// Returned at once by append. Nothing was queued.
pub enum AppendError {
    Full,                  // the accept queue is full
    TooLarge,              // the submit cannot fit in one group
    Invalid(&'static str), // empty submit, or a field outside its limits
    Sealed,                // seal has started or finished
    Poisoned,              // an earlier I/O error; reopen the log
}

/// Returned by Pending::wait.
pub enum CommitError {
    /// Record `index` had the wrong expected revision. `actual` is None when
    /// the stream has no records. Nothing was appended. No offset was used.
    WrongRevision { index: usize, actual: Option<Revision> },
    /// Record `index` has a key that matches a durable record with different bytes.
    KeyConflict { index: usize },
    /// Some keys in the submit matched durable records and others did not.
    PartialRetry,
    /// An I/O error happened while this submit was in flight. It might be
    /// durable. Reopen the log and retry with the same keys.
    Indeterminate,
    /// The writer was poisoned before it wrote this submit. Nothing was
    /// written. Reopen the log and retry.
    Poisoned,
}

pub enum OpenError {
    Locked,                                        // another process holds LOCK
    InvalidOptions(&'static str),                  // an option outside its limits
    Corrupt { segment: LogOffset, position: u64 }, // bad frame in a sealed segment
    LogIdMismatch { segment: LogOffset },
    Gap { segment: LogOffset },                    // does not start where the one before it ends
    NotSealed,                                     // LogReader::open on a log with no seal frame
    UnsupportedVersion(u16),
    Io(std::io::Error),
}

pub enum ReadError { Corrupt { segment: LogOffset, position: u64 }, Io(std::io::Error) }
pub enum SealError { Poisoned, Io(std::io::Error) }
```

## Behavior notes

- **Same bytes.** For a key match, "same bytes" means the same stream id and the same payload. The expected revision is not compared, because a retry usually carries a stale one. The writer reads the original record from its segment to compare. It does not trust a hash.
- **Retry of a whole submit.** If every record has a key, and every key matches a durable record with the same bytes, the submit is a retry. The answer is the original positions with `deduplicated: true`. If some keys match and others do not, or a record without a key is next to a matching key, the answer is `PartialRetry`. A record without a key is never deduplicated.
- **Milestones.** M1 needs only the full-retry match (`deduplicated: true`). `KeyConflict` and `PartialRetry` are M2.
- **Order of checks.** Keys first, for the whole submit. Then expected revisions, record by record. Records in the same submit and earlier pending submits count, in order of admission (R12). For a second record to the same stream in one submit, use `At(revision of the first)`.
- **A sealed log** opens read-only. `append` returns `Sealed`.
- **A reader after the guest is gone** uses `LogReader`, not `SegmentLog::open`, so that it changes nothing on the volume. It opens only a sealed log. To read an unsealed log, open it with `SegmentLog::open` on a writable volume and call `seal`.
- **Corrupt is for a human to resolve.** `OpenError::Corrupt { segment, position }` names the file and the byte where open stopped. Open never repairs it, because it cannot tell a torn tail from a bad frame in a sealed segment, and truncating a sealed segment drops acknowledged records. Repair is manual for now. Copy the directory, inspect the segment at that position, and truncate the copy only if the bytes there are a torn tail. A repair tool is future work ([issue #1](https://github.com/ironflowhq/segment-log/issues/1)).
- **Options.** `open` checks each option against its limit and returns `InvalidOptions`. `queue_capacity` must be at least 1, because a zero-capacity `sync_channel` refuses almost every `try_send`.
- **Thread model.** One writer thread, owned by `SegmentLog`. Callers never write files.

## Implementation notes

- The core crate uses only `std` in M1. A new dependency needs an ADR ([ADR 0012](../decisions/0012-no-log-crate-dependency.md)). Dev-dependencies for tests and benches do not.
- Accept queue: `std::sync::mpsc::sync_channel` with `try_send` gives "refuse when full" without a dependency.
- Lock: `std::fs::File::try_lock` (stable since Rust 1.89, an advisory lock).
- Flush: `File::sync_data` for segments, and `sync_all` on an opened directory handle for directory entries. [ADR 0005](../decisions/0005-flush-calls-and-platforms.md) gives the mapping to system calls and what M1 must still verify.
- All file I/O goes through a small storage trait, so that the tests can use simulated storage ([testing.md](testing.md)).
- Log id: random bytes from the storage trait, as many as the `log_id` width in format.md. `std` has no public random source, and the simulated storage must make them deterministic from its seed.
- CRC-32C: a software table is enough for M1. A faster version waits for a measurement.
