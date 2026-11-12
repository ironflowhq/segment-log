//! The v1 byte layout of [format.md](../../docs/architecture/format.md). Every width, limit and
//! check value comes from that file.

mod crc;
mod filter;
mod frame;
mod header;
mod index;

pub(crate) use crc::crc32c;
pub(crate) use filter::{KeyFilter, probes as filter_probes};
pub(crate) use frame::{
    Body, Entry, Footer, Frame, Group, StreamTail, decode, encode_footer, encode_group,
    encode_locator, encode_seal,
};
pub(crate) use header::{FORMAT_VERSION, HEADER_LEN, Header, decode_header, encode_header};
pub(crate) use index::{KeyEntry, KeyFrames, decode_page, encode_key_frames, page_count};

pub(crate) const FRAME_BODY_LIMIT: usize = 16 * 1024 * 1024;
pub(crate) const GROUP_BODY_LIMIT: usize = 8 * 1024 * 1024;
pub(crate) const STREAM_LEN_MAX: usize = 1024;
pub(crate) const KEY_LEN_MAX: usize = 256;
pub(crate) const ROLL_BYTES_LIMIT: u64 = 4000 * 1024 * 1024;
pub(crate) const PAGE_SIZE: usize = 4096;
pub(crate) const PAGE_ENTRIES: usize = 255;
pub(crate) const KEYED_RECORD_LIMIT: usize = 4095 * PAGE_ENTRIES;
pub(crate) const LOCATOR_LEN: usize = 40;

pub(crate) const ENVELOPE_LEN: usize = 8;
pub(crate) const BODY_HEADER_LEN: usize = 24;
pub(crate) const FOOTER_HEADER_LEN: usize = BODY_HEADER_LEN + 8;
pub(crate) const FOOTER_ENTRY_LEN: usize = 28;

const _: () = assert!(
    BODY_HEADER_LEN + PAGE_SIZE * KEYED_RECORD_LIMIT.div_ceil(PAGE_ENTRIES) <= FRAME_BODY_LIMIT
);
const _: () = assert!(
    BODY_HEADER_LEN + PAGE_SIZE * (KEYED_RECORD_LIMIT + 1).div_ceil(PAGE_ENTRIES)
        > FRAME_BODY_LIMIT
);

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
    Order,
    Fence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LogId {
    bytes: [u8; 16],
    crc: u32,
    keys: [u64; 2],
}

impl LogId {
    pub(crate) fn new(bytes: [u8; 16]) -> Self {
        let (k0, k1) = bytes.split_at(8);
        Self {
            bytes,
            crc: crc::update(!0, &bytes),
            keys: [
                u64::from_le_bytes(k0.try_into().unwrap()),
                u64::from_le_bytes(k1.try_into().unwrap()),
            ],
        }
    }

    pub(crate) fn bytes(&self) -> &[u8; 16] {
        &self.bytes
    }
}
