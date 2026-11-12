use super::{
    BODY_HEADER_LEN, Error, KEYED_RECORD_LIMIT, KeyFilter, LogId, PAGE_ENTRIES, PAGE_SIZE, crc32c,
    frame::{Kind, begin, finish},
};

const ENTRY_LEN: usize = 16;
const PAGE_CRC_AT: usize = PAGE_SIZE - 4;
const PAGE_COUNT_AT: usize = PAGE_ENTRIES * ENTRY_LEN;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct KeyEntry {
    pub hash: u64,
    pub position: u32,
    pub record: u32,
}

pub(crate) struct KeyFrames {
    pub filter: Vec<u8>,
    pub index: Vec<u8>,
}

pub(crate) fn page_count(count: u32) -> usize {
    (count as usize).div_ceil(PAGE_ENTRIES)
}

fn write_page(out: &mut Vec<u8>, entries: &[KeyEntry]) {
    let start = out.len();
    for entry in entries {
        out.extend_from_slice(&entry.hash.to_le_bytes());
        out.extend_from_slice(&entry.position.to_le_bytes());
        out.extend_from_slice(&entry.record.to_le_bytes());
    }
    out.resize(start + PAGE_COUNT_AT, 0);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    let crc = crc32c(&out[start..]);
    out.extend_from_slice(&crc.to_le_bytes());
}

pub(crate) fn encode_key_frames(
    log: &LogId,
    epoch: u64,
    base_offset: u64,
    entries: &mut [KeyEntry],
) -> Result<KeyFrames, Error> {
    if entries.len() > KEYED_RECORD_LIMIT {
        return Err(Error::Count);
    }
    entries.sort_unstable();
    if entries.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(Error::Order);
    }
    let count = entries.len() as u32;
    let filter = KeyFilter::build(entries);
    let mut out = begin(
        Kind::KeyFilter,
        epoch,
        base_offset,
        count,
        BODY_HEADER_LEN + filter.memory_bytes(),
    );
    filter.write_body(&mut out);
    let filter = finish(log, out);
    let mut out = begin(
        Kind::KeyIndex,
        epoch,
        base_offset,
        count,
        BODY_HEADER_LEN + PAGE_SIZE * page_count(count),
    );
    for page in entries.chunks(PAGE_ENTRIES) {
        write_page(&mut out, page);
    }
    Ok(KeyFrames {
        filter,
        index: finish(log, out),
    })
}

pub(crate) fn decode_page(
    page: &[u8],
    number: usize,
    count: u32,
    fence: u64,
) -> Result<Vec<KeyEntry>, Error> {
    if page.len() != PAGE_SIZE {
        return Err(Error::Length);
    }
    if crc32c(&page[..PAGE_CRC_AT]) != u32::from_le_bytes(page[PAGE_CRC_AT..].try_into().unwrap()) {
        return Err(Error::Crc);
    }
    let pages = page_count(count);
    if number >= pages {
        return Err(Error::Count);
    }
    let expected = if number + 1 == pages {
        count as usize - number * PAGE_ENTRIES
    } else {
        PAGE_ENTRIES
    };
    let stored = u32::from_le_bytes(page[PAGE_COUNT_AT..PAGE_COUNT_AT + 4].try_into().unwrap());
    if stored as usize != expected {
        return Err(Error::Count);
    }
    let unused = &page[expected * ENTRY_LEN..PAGE_COUNT_AT];
    let reserved = &page[PAGE_COUNT_AT + 4..PAGE_CRC_AT];
    if unused.iter().chain(reserved).any(|&byte| byte != 0) {
        return Err(Error::Reserved);
    }
    let entries: Vec<KeyEntry> = page[..expected * ENTRY_LEN]
        .as_chunks::<ENTRY_LEN>()
        .0
        .iter()
        .map(|entry| KeyEntry {
            hash: u64::from_le_bytes(entry[..8].try_into().unwrap()),
            position: u32::from_le_bytes(entry[8..12].try_into().unwrap()),
            record: u32::from_le_bytes(entry[12..].try_into().unwrap()),
        })
        .collect();
    if entries.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(Error::Order);
    }
    if entries[0].hash != fence {
        return Err(Error::Fence);
    }
    Ok(entries)
}
