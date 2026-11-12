use super::{Error, KeyEntry, LogId, PAGE_ENTRIES, index::page_count};
use std::ops::Range;

const BLOCK_BYTES: usize = 64;
const KEYS_PER_BLOCK: usize = 32;
const SALTS: [u32; 11] = [
    0x47b6137b, 0x44974d91, 0x8824ad5b, 0xa2b7289d, 0x705495c7, 0x2df1424b, 0x9efc4947, 0x5c6bfb31,
    0x1d0f3e9b, 0x6ac1e2f5, 0xb3a8c64d,
];

fn round(v: &mut [u64; 4]) {
    v[0] = v[0].wrapping_add(v[1]);
    v[1] = v[1].rotate_left(13) ^ v[0];
    v[0] = v[0].rotate_left(32);
    v[2] = v[2].wrapping_add(v[3]);
    v[3] = v[3].rotate_left(16) ^ v[2];
    v[0] = v[0].wrapping_add(v[3]);
    v[3] = v[3].rotate_left(21) ^ v[0];
    v[2] = v[2].wrapping_add(v[1]);
    v[1] = v[1].rotate_left(17) ^ v[2];
    v[2] = v[2].rotate_left(32);
}

fn siphash24([k0, k1]: [u64; 2], bytes: &[u8]) -> u64 {
    let mut v = [
        k0 ^ 0x736f6d6570736575,
        k1 ^ 0x646f72616e646f6d,
        k0 ^ 0x6c7967656e657261,
        k1 ^ 0x7465646279746573,
    ];
    let (words, rest) = bytes.as_chunks::<8>();
    for word in words {
        let word = u64::from_le_bytes(*word);
        v[3] ^= word;
        round(&mut v);
        round(&mut v);
        v[0] ^= word;
    }
    let mut last = u64::from(bytes.len() as u8) << 56;
    for (i, &byte) in rest.iter().enumerate() {
        last |= u64::from(byte) << (8 * i);
    }
    v[3] ^= last;
    round(&mut v);
    round(&mut v);
    v[0] ^= last;
    v[2] ^= 0xff;
    for _ in 0..4 {
        round(&mut v);
    }
    v[0] ^ v[1] ^ v[2] ^ v[3]
}

impl LogId {
    pub(crate) fn key_hash(&self, key: &[u8]) -> u64 {
        siphash24(self.keys, key)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct KeyFilter {
    count: u32,
    fences: Vec<u64>,
    blocks: Vec<[u8; BLOCK_BYTES]>,
}

impl KeyFilter {
    pub(super) fn build(entries: &[KeyEntry]) -> Self {
        let mut filter = Self {
            count: entries.len() as u32,
            fences: entries
                .iter()
                .step_by(PAGE_ENTRIES)
                .map(|entry| entry.hash)
                .collect(),
            blocks: vec![[0; BLOCK_BYTES]; entries.len().div_ceil(KEYS_PER_BLOCK)],
        };
        for entry in entries {
            filter.insert(entry.hash);
        }
        filter
    }

    pub(super) fn decode(count: u32, body: &[u8]) -> Result<Self, Error> {
        let pages = page_count(count);
        let blocks = (count as usize).div_ceil(KEYS_PER_BLOCK);
        if body.len() != 8 * pages + BLOCK_BYTES * blocks {
            return Err(Error::Length);
        }
        let (fences, blocks) = body.split_at(8 * pages);
        let fences: Vec<u64> = fences
            .as_chunks()
            .0
            .iter()
            .map(|fence| u64::from_le_bytes(*fence))
            .collect();
        if fences.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err(Error::Order);
        }
        Ok(Self {
            count,
            fences,
            blocks: blocks.as_chunks().0.to_vec(),
        })
    }

    pub(super) fn write_body(&self, out: &mut Vec<u8>) {
        for fence in &self.fences {
            out.extend_from_slice(&fence.to_le_bytes());
        }
        for block in &self.blocks {
            out.extend_from_slice(block);
        }
    }

    fn probes(&self, hash: u64) -> (usize, [usize; SALTS.len()]) {
        let block = (((hash >> 32) * self.blocks.len() as u64) >> 32) as usize;
        let low = hash as u32;
        (
            block,
            SALTS.map(|salt| (low.wrapping_mul(salt) >> 23) as usize),
        )
    }

    fn insert(&mut self, hash: u64) {
        let (block, bits) = self.probes(hash);
        for bit in bits {
            self.blocks[block][bit / 8] |= 1 << (bit % 8);
        }
    }

    pub(crate) fn may_contain(&self, hash: u64) -> bool {
        if self.blocks.is_empty() {
            return false;
        }
        let (block, bits) = self.probes(hash);
        bits.into_iter()
            .all(|bit| self.blocks[block][bit / 8] & (1 << (bit % 8)) != 0)
    }

    pub(crate) fn index_pages(&self, hash: u64) -> Range<usize> {
        if self.fences.is_empty() {
            return 0..0;
        }
        let below = self.fences.partition_point(|&fence| fence < hash);
        let through = self.fences.partition_point(|&fence| fence <= hash);
        let start = below.saturating_sub(1);
        start..through.max(start + 1)
    }

    pub(crate) fn count(&self) -> u32 {
        self.count
    }

    pub(crate) fn memory_bytes(&self) -> usize {
        self.fences.len() * size_of::<u64>() + self.blocks.len() * BLOCK_BYTES
    }
}
