use super::{Error, crc32c};

pub(crate) const HEADER_LEN: usize = 64;
/// The format version this build writes.
pub(crate) const FORMAT_VERSION: u16 = 3;
/// The oldest format version a build reads: from 0.1, every build reads every version from here up
/// to its own (ADR 0064, rule 11).
pub(crate) const MIN_FORMAT_VERSION: u16 = 3;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Header {
    /// `magic`, `format_version` and `header_len` keep their offsets in every version (ADR 0064,
    /// rule 13), so this decoder reads any version's; whether a build supports it is the caller's
    /// check.
    pub version: u16,
    pub log_id: [u8; 16],
    pub base_offset: u64,
    pub epoch: u64,
}

pub(crate) fn encode_header(header: &Header) -> [u8; HEADER_LEN] {
    let mut bytes = [0; HEADER_LEN];
    bytes[..4].copy_from_slice(b"SGLG");
    bytes[4..6].copy_from_slice(&header.version.to_le_bytes());
    bytes[6..8].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    bytes[8..24].copy_from_slice(&header.log_id);
    bytes[24..32].copy_from_slice(&header.base_offset.to_le_bytes());
    bytes[32..40].copy_from_slice(&header.epoch.to_le_bytes());
    let crc = crc32c(&bytes[..60]);
    bytes[60..].copy_from_slice(&crc.to_le_bytes());
    bytes
}

pub(crate) fn decode_header(bytes: &[u8]) -> Result<Header, Error> {
    if bytes.len() != HEADER_LEN {
        return Err(Error::Length);
    }
    if &bytes[..4] != b"SGLG" || bytes[6..8] != (HEADER_LEN as u16).to_le_bytes() {
        return Err(Error::Kind);
    }
    if bytes[40..60] != [0; 20] {
        return Err(Error::Reserved);
    }
    if crc32c(&bytes[..60]) != u32::from_le_bytes(bytes[60..].try_into().unwrap()) {
        return Err(Error::Crc);
    }
    Ok(Header {
        version: u16::from_le_bytes(bytes[4..6].try_into().unwrap()),
        log_id: bytes[8..24].try_into().unwrap(),
        base_offset: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
        epoch: u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
    })
}
