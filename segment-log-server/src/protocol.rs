//! The wire protocol. [protocol.md](../../docs/architecture/protocol.md) is the only source for its
//! bytes; this module follows it field by field.
#![forbid(unsafe_code)]

use segment_log_core::{
    AppendError, CommitError, Committed, ExpectedRevision, NewRecord, ReadError, Record, SealError,
    StreamHead, Submit,
};

pub const HEADER_LEN: usize = 8;
pub const MAGIC: [u8; 4] = *b"SGLP";
/// The protocol versions this server speaks. From 0.1 the range only grows at the top: a server
/// speaks every version from 2 up to its own, and no 0.x release raises `MIN_VERSION` (ADR 0064,
/// rule 5).
pub const MIN_VERSION: u16 = 2;
pub const MAX_VERSION: u16 = 2;

pub const HELLO: u8 = 1;
pub const APPEND: u8 = 2;
pub const READ: u8 = 3;
pub const WATERMARK: u8 = 4;
pub const STREAM_HEAD: u8 = 5;
pub const SEAL: u8 = 6;
/// The answer kind for a request that could not be decoded.
pub const MALFORMED_KIND: u8 = 0;

const ANSWER_BIT: u8 = 0x80;
const FRAME_CAP_FLOOR: usize = 64 * 1024;
const APPEND_ENTRY_LEN: usize = 17;
const READ_ENTRY_LEN: usize = 24;

/// The peer sent bytes that are not a request. The server answers `Malformed` and closes.
#[derive(Debug, PartialEq, Eq)]
pub struct Malformed;

/// The highest version in both ranges, or `None` when they share none. A `min` above its `max`
/// is an empty range. So a version 1 `Hello` (`version` 1, then a zero reserved field, read as
/// `min` 1 and `max` 0) gets `UnsupportedVersion`, never a misread (ADR 0064, rule 4).
pub fn negotiate(client: (u16, u16), server: (u16, u16)) -> Option<u16> {
    let high = client.1.min(server.1);
    (high >= client.0.max(server.0)).then_some(high)
}

/// The largest message body either side sends or buffers.
pub fn frame_cap(group_byte_cap: usize) -> usize {
    group_byte_cap.saturating_mul(2).max(FRAME_CAP_FLOOR)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub body_len: usize,
    pub kind: u8,
}

pub fn decode_header(bytes: [u8; HEADER_LEN]) -> Result<Header, Malformed> {
    if bytes[5..] != [0; 3] {
        return Err(Malformed);
    }
    let body_len = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    Ok(Header {
        body_len: body_len as usize,
        kind: bytes[4],
    })
}

/// What to do with a body, decided from its header before any of it is read.
#[derive(Debug, PartialEq, Eq)]
pub enum Admit {
    Read,
    /// An append above the cap: skip its bytes and answer `TooLarge`, as the core would.
    Discard,
}

pub fn admit(header: Header, frame_cap: usize) -> Result<Admit, Malformed> {
    if header.body_len <= frame_cap {
        Ok(Admit::Read)
    } else if header.kind == APPEND {
        Ok(Admit::Discard)
    } else {
        Err(Malformed)
    }
}

#[derive(Debug)]
pub enum Request {
    Hello { min: u16, max: u16 },
    Append(Submit),
    Read { from: u64, max: u32 },
    Watermark,
    StreamHead(Vec<u8>),
    Seal,
}

/// Rejects every non-canonical form, so a decoded request encodes back to the same bytes.
pub fn decode_request(kind: u8, body: &[u8]) -> Result<Request, Malformed> {
    let mut r = Reader(body);
    let request = match kind {
        HELLO => {
            if r.take(4)? != MAGIC {
                return Err(Malformed);
            }
            Request::Hello {
                min: r.u16()?,
                max: r.u16()?,
            }
        }
        APPEND => Request::Append(decode_submit(&mut r)?),
        READ => Request::Read {
            from: r.u64()?,
            max: r.u32()?,
        },
        WATERMARK => Request::Watermark,
        STREAM_HEAD => Request::StreamHead(r.take(body.len())?.to_vec()),
        SEAL => Request::Seal,
        _ => return Err(Malformed),
    };
    if !r.0.is_empty() {
        return Err(Malformed);
    }
    Ok(request)
}

fn decode_submit(r: &mut Reader) -> Result<Submit, Malformed> {
    let count = r.u32()? as usize;
    // The count comes from the peer: reserve no more entries than the body can hold.
    let mut records = Vec::with_capacity(count.min(r.0.len() / APPEND_ENTRY_LEN));
    for _ in 0..count {
        let stream_len = usize::from(r.u16()?);
        let key_len = usize::from(r.u16()?);
        let payload_len = r.u32()? as usize;
        let expected = match (r.u8()?, r.u64()?) {
            (0, 0) => ExpectedRevision::Empty,
            (1, revision) => ExpectedRevision::At(revision),
            _ => return Err(Malformed),
        };
        let stream = r.take(stream_len)?.to_vec();
        let key = match key_len {
            0 => None,
            n => Some(r.take(n)?.to_vec()),
        };
        let payload = r.take(payload_len)?.to_vec();
        records.push(NewRecord {
            stream,
            expected,
            key,
            payload,
        });
    }
    Ok(Submit { records })
}

/// The whole message, header included. The server never sends requests; tests and the fuzz target do.
pub fn encode_request(request: &Request) -> Vec<u8> {
    let mut w = Writer::new();
    let kind = match request {
        Request::Hello { min, max } => {
            w.bytes(&MAGIC);
            w.u16(*min);
            w.u16(*max);
            HELLO
        }
        Request::Append(submit) => {
            w.u32(len32(submit.records.len()));
            for record in &submit.records {
                let key = record.key.as_deref().unwrap_or_default();
                w.u16(len16(record.stream.len()));
                w.u16(len16(key.len()));
                w.u32(len32(record.payload.len()));
                match record.expected {
                    ExpectedRevision::Empty => {
                        w.u8(0);
                        w.u64(0);
                    }
                    ExpectedRevision::At(revision) => {
                        w.u8(1);
                        w.u64(revision);
                    }
                }
                w.bytes(&record.stream);
                w.bytes(key);
                w.bytes(&record.payload);
            }
            APPEND
        }
        Request::Read { from, max } => {
            w.u64(*from);
            w.u32(*max);
            READ
        }
        Request::Watermark => WATERMARK,
        Request::StreamHead(stream) => {
            w.bytes(stream);
            STREAM_HEAD
        }
        Request::Seal => SEAL,
    };
    w.finish(kind)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Full,
    TooLarge,
    Invalid(&'static str),
    Sealed,
    Poisoned,
    Exhausted,
    WrongRevision { index: u32, actual: Option<u64> },
    KeyConflict { index: u32 },
    PartialRetry,
    Indeterminate,
    PoisonedBeforeWrite,
    Corrupt { segment: u64, position: u64 },
    Io(std::io::ErrorKind),
    Malformed,
    UnsupportedVersion,
}

impl Status {
    pub fn code(&self) -> u16 {
        match self {
            Status::Full => 1,
            Status::TooLarge => 2,
            Status::Invalid(_) => 3,
            Status::Sealed => 4,
            Status::Poisoned => 5,
            Status::Exhausted => 6,
            Status::WrongRevision { .. } => 7,
            Status::KeyConflict { .. } => 8,
            Status::PartialRetry => 9,
            Status::Indeterminate => 10,
            Status::PoisonedBeforeWrite => 11,
            Status::Corrupt { .. } => 12,
            Status::Io(_) => 13,
            Status::Malformed => 14,
            Status::UnsupportedVersion => 15,
        }
    }
}

/// Record indexes fit in 32 bits because a wire submit counts its records in 32 bits.
fn index32(index: usize) -> u32 {
    u32::try_from(index).expect("a record index of a wire submit fits in 32 bits")
}

impl From<AppendError> for Status {
    fn from(error: AppendError) -> Self {
        match error {
            AppendError::Full => Status::Full,
            AppendError::TooLarge => Status::TooLarge,
            AppendError::Invalid(reason) => Status::Invalid(reason),
            AppendError::Sealed => Status::Sealed,
            AppendError::Poisoned => Status::Poisoned,
        }
    }
}

impl From<CommitError> for Status {
    fn from(error: CommitError) -> Self {
        match error {
            CommitError::Exhausted => Status::Exhausted,
            CommitError::WrongRevision { index, actual } => Status::WrongRevision {
                index: index32(index),
                actual,
            },
            CommitError::KeyConflict { index } => Status::KeyConflict {
                index: index32(index),
            },
            CommitError::PartialRetry => Status::PartialRetry,
            CommitError::Indeterminate => Status::Indeterminate,
            CommitError::Poisoned => Status::PoisonedBeforeWrite,
        }
    }
}

impl From<ReadError> for Status {
    fn from(error: ReadError) -> Self {
        match error {
            ReadError::Corrupt { segment, position } => Status::Corrupt { segment, position },
            ReadError::Io(error) => Status::Io(error.kind()),
        }
    }
}

impl From<SealError> for Status {
    fn from(error: SealError) -> Self {
        match error {
            SealError::Poisoned => Status::Poisoned,
        }
    }
}

pub enum Answer<'a> {
    Hello {
        version: u16,
        frame_cap: usize,
    },
    Append(&'a Committed),
    /// Already cut to the cap with [`fitting_records`].
    Read(&'a [Record]),
    Watermark(u64),
    StreamHead(Option<&'a StreamHead>),
    Seal,
    Failed(Status),
}

/// The whole answer message to a request of `kind` (`MALFORMED_KIND` when it could not be decoded).
pub fn encode_answer(kind: u8, answer: &Answer) -> Vec<u8> {
    let mut w = Writer::new();
    let Answer::Failed(status) = answer else {
        w.u16(0);
        match answer {
            Answer::Hello { version, frame_cap } => {
                w.u16(*version);
                w.u32(len32(*frame_cap));
                w.u16(0);
            }
            Answer::Append(committed) => {
                w.u8(u8::from(committed.deduplicated));
                w.bytes(&[0; 3]);
                w.u32(len32(committed.records.len()));
                for position in &committed.records {
                    w.u64(position.offset);
                    w.u64(position.revision);
                }
            }
            Answer::Read(records) => {
                w.u32(len32(records.len()));
                for record in *records {
                    let key = record.key.as_deref().unwrap_or_default();
                    w.u64(record.offset);
                    w.u64(record.revision);
                    w.u16(len16(record.stream.len()));
                    w.u16(len16(key.len()));
                    w.u32(len32(record.payload.len()));
                    w.bytes(&record.stream);
                    w.bytes(key);
                    w.bytes(&record.payload);
                }
            }
            Answer::Watermark(watermark) => w.u64(*watermark),
            Answer::StreamHead(head) => {
                w.u8(u8::from(head.is_some()));
                w.bytes(&[0; 7]);
                w.u64(head.map_or(0, |h| h.revision));
                w.u64(head.map_or(0, |h| h.offset));
            }
            Answer::Seal => {}
            Answer::Failed(_) => unreachable!(),
        }
        return w.finish(kind | ANSWER_BIT);
    };
    w.u16(status.code());
    match status {
        Status::Invalid(reason) => {
            w.u16(len16(reason.len()));
            w.bytes(reason.as_bytes());
        }
        Status::WrongRevision { index, actual } => {
            w.u32(*index);
            w.u8(u8::from(actual.is_some()));
            w.u64(actual.unwrap_or(0));
        }
        Status::KeyConflict { index } => w.u32(*index),
        Status::Corrupt { segment, position } => {
            w.u64(*segment);
            w.u64(*position);
        }
        Status::Io(kind) => {
            let name = format!("{kind:?}");
            w.u16(len16(name.len()));
            w.bytes(name.as_bytes());
        }
        Status::UnsupportedVersion => {
            w.u16(MIN_VERSION);
            w.u16(MAX_VERSION);
        }
        _ => {}
    }
    w.finish(kind | ANSWER_BIT)
}

/// The `max` to pass to the core's `read`. No answer within `frame_cap` holds more records than
/// this, because every entry has at least a one-byte stream id; a larger `max` would only make the
/// core collect records the answer then drops.
pub fn read_limit(max: u32, frame_cap: usize) -> usize {
    (max as usize).min(frame_cap / (READ_ENTRY_LEN + 1))
}

/// How many of `records` a `Read` answer holds: the longest prefix within `frame_cap`, and at least
/// one record when there is any.
pub fn fitting_records(records: &[Record], frame_cap: usize) -> usize {
    let mut len = 2 + 4;
    let fit = records
        .iter()
        .take_while(|r| {
            len += READ_ENTRY_LEN
                + r.stream.len()
                + r.key.as_ref().map_or(0, Vec::len)
                + r.payload.len();
            len <= frame_cap
        })
        .count();
    fit.max(records.len().min(1))
}

// Field lengths come from limits far below these widths (format.md), so a failure is a bug.
fn len16(len: usize) -> u16 {
    u16::try_from(len).expect("a length within its 16-bit field")
}

fn len32(len: usize) -> u32 {
    u32::try_from(len).expect("a length within its 32-bit field")
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        let (head, rest) = self.0.split_at_checked(n).ok_or(Malformed)?;
        self.0 = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, Malformed> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Malformed> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, Malformed> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, Malformed> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

struct Writer(Vec<u8>);

impl Writer {
    fn new() -> Self {
        Writer(vec![0; HEADER_LEN])
    }
    fn bytes(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn finish(mut self, kind: u8) -> Vec<u8> {
        let body_len = len32(self.0.len() - HEADER_LEN);
        self.0[..4].copy_from_slice(&body_len.to_le_bytes());
        self.0[4] = kind;
        self.0
    }
}

#[cfg(test)]
mod tests;
