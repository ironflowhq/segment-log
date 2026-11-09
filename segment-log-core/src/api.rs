//! Public contracts are defined in [api.md](../../docs/architecture/api.md).
use std::time::Duration;
pub type LogOffset = u64;
pub type Revision = u64;
pub struct Options {
    pub linger: Duration,
    pub group_byte_cap: usize,
    pub roll_bytes: u64,
    pub queue_capacity: usize,
    pub pending_byte_budget: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            linger: Duration::from_millis(1),
            group_byte_cap: 1024 * 1024,
            roll_bytes: 64 * 1024 * 1024,
            queue_capacity: 4096,
            pending_byte_budget: 16 * 1024 * 1024,
        }
    }
}
#[derive(Debug)]
pub struct Submit {
    pub records: Vec<NewRecord>,
}

#[derive(Debug)]
pub struct NewRecord {
    pub stream: Vec<u8>,
    pub expected: ExpectedRevision,
    pub key: Option<Vec<u8>>,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub enum ExpectedRevision {
    Empty,
    At(Revision),
}

#[derive(Debug)]
pub struct Committed {
    pub records: Vec<Position>,
    pub deduplicated: bool,
}

#[derive(Debug)]
pub struct Position {
    pub offset: LogOffset,
    pub revision: Revision,
}

#[derive(Debug)]
pub struct Record {
    pub offset: LogOffset,
    pub stream: Vec<u8>,
    pub revision: Revision,
    pub key: Option<Vec<u8>>,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub struct StreamHead {
    pub revision: Revision,
    pub offset: LogOffset,
}

#[derive(Debug)]
pub enum AppendError {
    Full,
    TooLarge,
    Invalid(&'static str),
    Sealed,
    Poisoned,
}

#[derive(Debug)]
pub enum CommitError {
    Exhausted,
    WrongRevision {
        index: usize,
        actual: Option<Revision>,
    },
    KeyConflict {
        index: usize,
    },
    PartialRetry,
    Indeterminate,
    Poisoned,
}

#[derive(Debug)]
pub enum OpenError {
    Exhausted,
    Locked,
    InvalidOptions(&'static str),
    Corrupt { segment: LogOffset, position: u64 },
    LogIdMismatch { segment: LogOffset },
    Gap { segment: LogOffset },
    NotSealed,
    UnsupportedVersion(u16),
    Io(std::io::Error),
}

#[derive(Debug)]
pub enum ReadError {
    Corrupt { segment: LogOffset, position: u64 },
    Io(std::io::Error),
}
