use super::{Error, crc32c};

pub(crate) const HEADER_LEN: usize = 64;
pub(crate) const FORMAT_VERSION: u16 = 1;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub log_id: [u8; 16],
    pub base_offset: u64,
    pub epoch: u64,
}

/// The magic, `format_version` and `header_len`.
fn prefix() -> [u8; 8] {
    let mut bytes = [0; 8];
    bytes[..4].copy_from_slice(b"SGLG");
    bytes[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes[6..8].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    bytes
}

pub(crate) fn encode_header(header: &Header) -> [u8; HEADER_LEN] {
    let mut bytes = [0; HEADER_LEN];
    bytes[..8].copy_from_slice(&prefix());
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
    if bytes[..8] != prefix() {
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
