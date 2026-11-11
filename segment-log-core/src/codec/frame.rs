use super::{
    BODY_HEADER_LEN, ENVELOPE_LEN, Error, FOOTER_ENTRY_LEN, FOOTER_HEADER_LEN, FRAME_BODY_LIMIT,
    GROUP_BODY_LIMIT, KEY_LEN_MAX, KEYED_RECORD_LIMIT, KeyFilter, LOCATOR_LEN, LogId, PAGE_SIZE,
    STREAM_LEN_MAX, crc, index,
};

#[derive(Clone, Copy)]
#[repr(u8)]
pub(super) enum Kind {
    Group = 1,
    Footer,
    Seal,
    Locator,
    KeyFilter,
    KeyIndex,
}

impl Kind {
    fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            1 => Self::Group,
            2 => Self::Footer,
            3 => Self::Seal,
            4 => Self::Locator,
            5 => Self::KeyFilter,
            6 => Self::KeyIndex,
            _ => return None,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Entry<'a> {
    pub stream: &'a [u8],
    pub key: Option<&'a [u8]>,
    pub payload: &'a [u8],
    pub revision: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Group<'a> {
    pub epoch: u64,
    pub base_offset: u64,
    pub entries: Vec<Entry<'a>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StreamTail<'a> {
    pub stream: &'a [u8],
    pub last_revision: u64,
    pub last_offset: u64,
    pub last_position: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Footer<'a> {
    /// `None` in the final segment of a sealed log, which has no key frames.
    pub key_filter_position: Option<std::num::NonZeroU64>,
    pub streams: Vec<StreamTail<'a>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Frame<'a> {
    pub epoch: u64,
    pub base_offset: u64,
    pub body: Body<'a>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Body<'a> {
    Group(Vec<Entry<'a>>),
    Footer(Footer<'a>),
    Seal,
    Locator { footer_position: u64 },
    KeyFilter(KeyFilter),
    KeyIndex { count: u32, pages: &'a [u8] },
}

impl<'a> Frame<'a> {
    pub(crate) fn into_group(self) -> Option<Group<'a>> {
        match self.body {
            Body::Group(entries) => Some(Group {
                epoch: self.epoch,
                base_offset: self.base_offset,
                entries,
            }),
            _ => None,
        }
    }
}

impl LogId {
    fn frame_crc(&self, frame: &[u8]) -> u32 {
        !crc::update(crc::update(self.crc, &frame[..4]), &frame[ENVELOPE_LEN..])
    }
}

pub(super) fn begin(
    kind: Kind,
    epoch: u64,
    base_offset: u64,
    count: u32,
    body_len: usize,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(ENVELOPE_LEN + body_len);
    out.extend_from_slice(&[0; ENVELOPE_LEN]);
    out.extend_from_slice(&[kind as u8, 0, 0, 0]);
    out.extend_from_slice(&epoch.to_le_bytes());
    out.extend_from_slice(&base_offset.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out
}

pub(super) fn finish(log: &LogId, mut frame: Vec<u8>) -> Vec<u8> {
    let body_len = (frame.len() - ENVELOPE_LEN) as u32;
    frame[..4].copy_from_slice(&body_len.to_le_bytes());
    let checksum = log.frame_crc(&frame);
    frame[4..ENVELOPE_LEN].copy_from_slice(&checksum.to_le_bytes());
    frame
}

fn check_stream(stream: &[u8]) -> Result<(), Error> {
    if stream.is_empty() || stream.len() > STREAM_LEN_MAX {
        return Err(Error::Stream);
    }
    Ok(())
}

fn check_entry(entry: &Entry<'_>) -> Result<(), Error> {
    check_stream(entry.stream)?;
    if entry
        .key
        .is_some_and(|k| k.is_empty() || k.len() > KEY_LEN_MAX)
    {
        return Err(Error::Key);
    }
    Ok(())
}

pub(crate) fn encode_group(log: &LogId, group: &Group<'_>) -> Result<Vec<u8>, Error> {
    if group.entries.is_empty() {
        return Err(Error::Count);
    }
    group
        .base_offset
        .checked_add(group.entries.len() as u64)
        .ok_or(Error::Offset)?;
    let mut size = BODY_HEADER_LEN;
    for entry in &group.entries {
        check_entry(entry)?;
        size = size
            .checked_add(16 + entry.stream.len() + entry.key.map_or(0, <[u8]>::len))
            .and_then(|n| n.checked_add(entry.payload.len()))
            .ok_or(Error::Length)?;
    }
    if size > GROUP_BODY_LIMIT {
        return Err(Error::Length);
    }
    let mut out = begin(
        Kind::Group,
        group.epoch,
        group.base_offset,
        group.entries.len() as u32,
        size,
    );
    for entry in &group.entries {
        out.extend_from_slice(&(entry.stream.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entry.key.map_or(0, <[u8]>::len) as u16).to_le_bytes());
        out.extend_from_slice(&(entry.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&entry.revision.to_le_bytes());
        out.extend_from_slice(entry.stream);
        out.extend_from_slice(entry.key.unwrap_or_default());
        out.extend_from_slice(entry.payload);
    }
    Ok(finish(log, out))
}

pub(crate) fn encode_footer(
    log: &LogId,
    epoch: u64,
    base_offset: u64,
    footer: &Footer<'_>,
) -> Result<Vec<u8>, Error> {
    let mut size = FOOTER_HEADER_LEN;
    for tail in &footer.streams {
        check_stream(tail.stream)?;
        size += FOOTER_ENTRY_LEN + tail.stream.len();
        if size > FRAME_BODY_LIMIT {
            return Err(Error::Length);
        }
    }
    let mut out = begin(
        Kind::Footer,
        epoch,
        base_offset,
        footer.streams.len() as u32,
        size,
    );
    out.extend_from_slice(
        &footer
            .key_filter_position
            .map_or(0, u64::from)
            .to_le_bytes(),
    );
    for tail in &footer.streams {
        out.extend_from_slice(&(tail.stream.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0; 2]);
        out.extend_from_slice(&tail.last_revision.to_le_bytes());
        out.extend_from_slice(&tail.last_offset.to_le_bytes());
        out.extend_from_slice(&tail.last_position.to_le_bytes());
        out.extend_from_slice(tail.stream);
    }
    Ok(finish(log, out))
}

pub(crate) fn encode_seal(log: &LogId, epoch: u64, base_offset: u64) -> Vec<u8> {
    finish(
        log,
        begin(Kind::Seal, epoch, base_offset, 0, BODY_HEADER_LEN),
    )
}

pub(crate) fn encode_locator(
    log: &LogId,
    epoch: u64,
    base_offset: u64,
    footer_position: u64,
) -> Vec<u8> {
    let mut out = begin(
        Kind::Locator,
        epoch,
        base_offset,
        0,
        LOCATOR_LEN - ENVELOPE_LEN,
    );
    out.extend_from_slice(&footer_position.to_le_bytes());
    finish(log, out)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self.position.checked_add(len).ok_or(Error::Length)?;
        let out = self.bytes.get(self.position..end).ok_or(Error::Length)?;
        self.position = end;
        Ok(out)
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

pub(crate) fn decode<'a>(log: &LogId, bytes: &'a [u8]) -> Result<Frame<'a>, Error> {
    let mut c = Cursor { bytes, position: 0 };
    let size = c.u32()? as usize;
    let stored = c.u32()?;
    if !(BODY_HEADER_LEN..=FRAME_BODY_LIMIT).contains(&size) || bytes.len() != size + ENVELOPE_LEN {
        return Err(Error::Length);
    }
    if log.frame_crc(bytes) != stored {
        return Err(Error::Crc);
    }
    let kind = Kind::from_byte(c.take(1)?[0]).ok_or(Error::Kind)?;
    if c.take(3)? != [0; 3] {
        return Err(Error::Reserved);
    }
    let epoch = c.u64()?;
    let base_offset = c.u64()?;
    let count = c.u32()?;
    let body = match kind {
        Kind::Group => Body::Group(decode_group(&mut c, size, base_offset, count)?),
        Kind::Footer => Body::Footer(decode_footer(&mut c, size, count)?),
        Kind::Seal => {
            check_fixed_body(&c, count, BODY_HEADER_LEN)?;
            Body::Seal
        }
        Kind::Locator => {
            check_fixed_body(&c, count, LOCATOR_LEN - ENVELOPE_LEN)?;
            Body::Locator {
                footer_position: c.u64()?,
            }
        }
        Kind::KeyFilter => {
            keyed(count)?;
            Body::KeyFilter(KeyFilter::decode(count, &bytes[c.position..])?)
        }
        Kind::KeyIndex => {
            keyed(count)?;
            let pages = &bytes[c.position..];
            if pages.len() != PAGE_SIZE * index::page_count(count) {
                return Err(Error::Length);
            }
            Body::KeyIndex { count, pages }
        }
    };
    Ok(Frame {
        epoch,
        base_offset,
        body,
    })
}

fn check_fixed_body(c: &Cursor<'_>, count: u32, body_len: usize) -> Result<(), Error> {
    if count != 0 {
        return Err(Error::Count);
    }
    if c.bytes.len() != ENVELOPE_LEN + body_len {
        return Err(Error::Length);
    }
    Ok(())
}

fn keyed(count: u32) -> Result<(), Error> {
    if count as usize > KEYED_RECORD_LIMIT {
        return Err(Error::Count);
    }
    Ok(())
}

fn decode_group<'a>(
    c: &mut Cursor<'a>,
    size: usize,
    base_offset: u64,
    count: u32,
) -> Result<Vec<Entry<'a>>, Error> {
    if size > GROUP_BODY_LIMIT {
        return Err(Error::Length);
    }
    let count = count as usize;
    if count == 0 || count > (size - BODY_HEADER_LEN) / 17 {
        return Err(Error::Count);
    }
    base_offset.checked_add(count as u64).ok_or(Error::Offset)?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let stream_len = c.u16()? as usize;
        let key_len = c.u16()? as usize;
        let payload_len = c.u32()? as usize;
        let revision = c.u64()?;
        if !(1..=STREAM_LEN_MAX).contains(&stream_len) {
            return Err(Error::Stream);
        }
        if key_len > KEY_LEN_MAX {
            return Err(Error::Key);
        }
        let stream = c.take(stream_len)?;
        let key = if key_len == 0 {
            None
        } else {
            Some(c.take(key_len)?)
        };
        let payload = c.take(payload_len)?;
        entries.push(Entry {
            stream,
            key,
            payload,
            revision,
        });
    }
    if c.position != c.bytes.len() {
        return Err(Error::Length);
    }
    Ok(entries)
}

fn decode_footer<'a>(c: &mut Cursor<'a>, size: usize, count: u32) -> Result<Footer<'a>, Error> {
    let key_filter_position = std::num::NonZeroU64::new(c.u64()?);
    let count = count as usize;
    if count > (size - FOOTER_HEADER_LEN) / (FOOTER_ENTRY_LEN + 1) {
        return Err(Error::Count);
    }
    let mut streams = Vec::with_capacity(count);
    for _ in 0..count {
        let stream_len = c.u16()? as usize;
        if c.take(2)? != [0; 2] {
            return Err(Error::Reserved);
        }
        let last_revision = c.u64()?;
        let last_offset = c.u64()?;
        let last_position = c.u64()?;
        if !(1..=STREAM_LEN_MAX).contains(&stream_len) {
            return Err(Error::Stream);
        }
        streams.push(StreamTail {
            stream: c.take(stream_len)?,
            last_revision,
            last_offset,
            last_position,
        });
    }
    if c.position != c.bytes.len() {
        return Err(Error::Length);
    }
    Ok(Footer {
        key_filter_position,
        streams,
    })
}
