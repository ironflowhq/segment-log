#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Length,
    Crc,
    Reserved,
    Kind,
    Count,
    Stream,
    Key,
    Offset,
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
// Slicing-by-8: eight table lookups consume eight input bytes per step.
const TABLES: [[u32; 256]; 8] = {
    let mut tables = [[0; 256]; 8];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc >> 1) ^ (0x82f63b78 & 0u32.wrapping_sub(crc & 1));
            bit += 1;
        }
        tables[0][index] = crc;
        index += 1;
    }
    let mut table = 1;
    while table < 8 {
        let mut index = 0;
        while index < 256 {
            let previous = tables[table - 1][index];
            tables[table][index] = (previous >> 8) ^ tables[0][(previous & 0xff) as usize];
            index += 1;
        }
        table += 1;
    }
    tables
};
fn crc_update(mut crc: u32, bytes: &[u8]) -> u32 {
    let (chunks, rest) = bytes.as_chunks::<8>();
    for chunk in chunks {
        let word = u64::from_le_bytes(*chunk) ^ u64::from(crc);
        crc = TABLES[7][(word & 0xff) as usize]
            ^ TABLES[6][((word >> 8) & 0xff) as usize]
            ^ TABLES[5][((word >> 16) & 0xff) as usize]
            ^ TABLES[4][((word >> 24) & 0xff) as usize]
            ^ TABLES[3][((word >> 32) & 0xff) as usize]
            ^ TABLES[2][((word >> 40) & 0xff) as usize]
            ^ TABLES[1][((word >> 48) & 0xff) as usize]
            ^ TABLES[0][(word >> 56) as usize];
    }
    for &byte in rest {
        crc = (crc >> 8) ^ TABLES[0][((crc ^ u32::from(byte)) & 0xff) as usize];
    }
    crc
}
pub(crate) fn crc32c(bytes: &[u8]) -> u32 {
    !crc_update(!0, bytes)
}
fn fields(entry: &Entry<'_>) -> Result<(), Error> {
    if entry.stream.is_empty() || entry.stream.len() > 1024 {
        return Err(Error::Stream);
    }
    if entry.key.is_some_and(|k| k.is_empty() || k.len() > 256) {
        return Err(Error::Key);
    }
    Ok(())
}
pub(crate) fn encode(group: &Group<'_>) -> Result<Vec<u8>, Error> {
    if group.entries.is_empty() {
        return Err(Error::Count);
    }
    group
        .base_offset
        .checked_add(group.entries.len() as u64)
        .ok_or(Error::Offset)?;
    let mut size = 24usize;
    for entry in &group.entries {
        fields(entry)?;
        size = size
            .checked_add(16 + entry.stream.len() + entry.key.map_or(0, <[u8]>::len))
            .and_then(|n| n.checked_add(entry.payload.len()))
            .ok_or(Error::Length)?;
    }
    if size > 8 * 1024 * 1024 {
        return Err(Error::Length);
    }
    let mut out = Vec::with_capacity(size + 8);
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&[1, 0, 0, 0]);
    out.extend_from_slice(&group.epoch.to_le_bytes());
    out.extend_from_slice(&group.base_offset.to_le_bytes());
    out.extend_from_slice(&(group.entries.len() as u32).to_le_bytes());
    for entry in &group.entries {
        out.extend_from_slice(&(entry.stream.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entry.key.map_or(0, <[u8]>::len) as u16).to_le_bytes());
        out.extend_from_slice(&(entry.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&entry.revision.to_le_bytes());
        out.extend_from_slice(entry.stream);
        out.extend_from_slice(entry.key.unwrap_or_default());
        out.extend_from_slice(entry.payload);
    }
    let crc = frame_crc(&out);
    out[4..8].copy_from_slice(&crc.to_le_bytes());
    Ok(out)
}
// The checksum skips its own field, bytes 4..8.
fn frame_crc(bytes: &[u8]) -> u32 {
    !crc_update(crc_update(!0, &bytes[..4]), &bytes[8..])
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
pub(crate) fn decode(bytes: &[u8]) -> Result<Group<'_>, Error> {
    let mut c = Cursor { bytes, position: 0 };
    let size = c.u32()? as usize;
    let crc = c.u32()?;
    if !(24..=8 * 1024 * 1024).contains(&size) || bytes.len() != size + 8 {
        return Err(Error::Length);
    }
    if frame_crc(bytes) != crc {
        return Err(Error::Crc);
    }
    if c.take(1)? != [1] {
        return Err(Error::Kind);
    }
    if c.take(3)? != [0; 3] {
        return Err(Error::Reserved);
    }
    let epoch = c.u64()?;
    let base_offset = c.u64()?;
    let count = c.u32()? as usize;
    if count == 0 || count > (size - 24) / 17 {
        return Err(Error::Count);
    }
    base_offset.checked_add(count as u64).ok_or(Error::Offset)?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let stream_len = c.u16()? as usize;
        let key_len = c.u16()? as usize;
        let payload_len = c.u32()? as usize;
        let revision = c.u64()?;
        if !(1..=1024).contains(&stream_len) {
            return Err(Error::Stream);
        }
        if key_len > 256 {
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
    if c.position != bytes.len() {
        return Err(Error::Length);
    }
    Ok(Group {
        epoch,
        base_offset,
        entries,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub log_id: [u8; 16],
    pub base_offset: u64,
    pub epoch: u64,
}
pub(crate) fn encode_header(header: &Header) -> [u8; 64] {
    let mut bytes = [0; 64];
    bytes[..8].copy_from_slice(b"SGLG\0\0\x40\0");
    bytes[8..24].copy_from_slice(&header.log_id);
    bytes[24..32].copy_from_slice(&header.base_offset.to_le_bytes());
    bytes[32..40].copy_from_slice(&header.epoch.to_le_bytes());
    let crc = crc32c(&bytes[..60]);
    bytes[60..].copy_from_slice(&crc.to_le_bytes());
    bytes
}
pub(crate) fn decode_header(bytes: &[u8]) -> Result<Header, Error> {
    if bytes.len() != 64 {
        return Err(Error::Length);
    }
    if bytes[..8] != *b"SGLG\0\0\x40\0" {
        return Err(Error::Kind);
    }
    if bytes[40..60] != [0; 20] {
        return Err(Error::Reserved);
    }
    if crc32c(&bytes[..60]) != u32::from_le_bytes(bytes[60..].try_into().unwrap()) {
        return Err(Error::Crc);
    }
    Ok(Header {
        log_id: bytes[8..24].try_into().unwrap(),
        base_offset: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
        epoch: u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
    })
}
