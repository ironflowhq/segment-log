use super::{CHECK_ID, log};
use crate::acceptance::support::{bounded, raw};
use crate::codec::{self, Entry, Group, LogId};

fn body_header(kind: u8, epoch: u64, base_offset: u64, count: u32) -> Vec<u8> {
    let mut body = vec![kind, 0, 0, 0];
    body.extend(epoch.to_le_bytes());
    body.extend(base_offset.to_le_bytes());
    body.extend(count.to_le_bytes());
    body
}

fn frame_of(body: &[u8]) -> Vec<u8> {
    raw::frame(&CHECK_ID, body)
}

const SALTS: [u32; 11] = [
    0x47b6137b, 0x44974d91, 0x8824ad5b, 0xa2b7289d, 0x705495c7, 0x2df1424b, 0x9efc4947, 0x5c6bfb31,
    0x1d0f3e9b, 0x6ac1e2f5, 0xb3a8c64d,
];

fn probes(hash: u64, blocks: usize) -> (usize, Vec<usize>) {
    let block = (((hash >> 32) * blocks as u64) >> 32) as usize;
    let low = (hash & 0xffff_ffff) as u32;
    (
        block,
        SALTS
            .iter()
            .map(|salt| (low.wrapping_mul(*salt) >> 23) as usize)
            .collect(),
    )
}

fn filter_blocks(hashes: &[u64]) -> Vec<u8> {
    let blocks = hashes.len().div_ceil(32);
    let mut out = vec![0; blocks * 64];
    for &hash in hashes {
        let (block, bits) = probes(hash, blocks);
        for bit in bits {
            out[block * 64 + bit / 8] |= 1 << (bit % 8);
        }
    }
    out
}

fn entry(hash: u64, position: u32, record: u32) -> codec::KeyEntry {
    codec::KeyEntry {
        hash,
        position,
        record,
    }
}

fn key_filter_of(frame: &[u8]) -> codec::KeyFilter {
    match codec::decode(&log(), frame).unwrap().body {
        codec::Body::KeyFilter(filter) => filter,
        other => panic!("not a key filter: {other:?}"),
    }
}

fn index_pages_of(frame: &[u8]) -> Vec<Vec<u8>> {
    match codec::decode(&log(), frame).unwrap().body {
        codec::Body::KeyIndex { pages, .. } => pages.chunks(4096).map(<[u8]>::to_vec).collect(),
        other => panic!("not a key index: {other:?}"),
    }
}

fn tail(stream: &[u8]) -> codec::StreamTail<'_> {
    codec::StreamTail {
        stream,
        last_revision: 0,
        last_offset: 0,
        last_position: 64,
    }
}

fn footer(streams: Vec<codec::StreamTail<'_>>) -> codec::Footer<'_> {
    codec::Footer {
        key_filter_position: 0,
        streams,
    }
}

/// Given the log id `00 01 02 … 0f` and a seal frame with epoch 1 and base offset 0
/// When the frame is encoded and decoded
/// Then its bytes are the check value of format.md, with crc 0xfd78783f, and the decoder reads a seal
///
/// Cases: CODEC-GROUP
#[test]
fn the_seal_frame_matches_the_frame_crc_check_value() {
    bounded(|| {
        let mut expected = vec![24, 0, 0, 0, 0x3f, 0x78, 0x78, 0xfd, 3, 0, 0, 0, 1];
        expected.extend([0; 19]);
        let bytes = codec::encode_seal(&log(), 1, 0);
        assert_eq!(bytes, expected);
        assert_eq!(
            codec::decode(&log(), &bytes),
            Ok(codec::Frame {
                epoch: 1,
                base_offset: 0,
                body: codec::Body::Seal
            })
        );
    });
}

/// Given a group frame and a frame of every closing kind, each written for one log
/// When a decoder with another log id, or one whose checksum has no log id, reads them
/// Then it rejects each with a checksum error, and the log's own decoder accepts them
///
/// Cases: CODEC-GROUP
#[test]
fn a_frame_validates_only_under_its_own_log_id() {
    bounded(|| {
        use codec::Error;
        let group = Group {
            epoch: 1,
            base_offset: 0,
            entries: vec![Entry {
                stream: b"s",
                key: None,
                payload: b"p",
                revision: 0,
            }],
        };
        let keys = codec::encode_key_frames(&log(), 1, 1, &mut [entry(1, 64, 0)]).unwrap();
        let frames = [
            codec::encode_group(&log(), &group).unwrap(),
            codec::encode_footer(&log(), 1, 1, &footer(vec![])).unwrap(),
            codec::encode_seal(&log(), 1, 1),
            codec::encode_locator(&log(), 1, 1, 64),
            keys.filter,
            keys.index,
        ];
        let other = LogId::new([9; 16]);
        for frame in &frames {
            assert!(codec::decode(&log(), frame).is_ok());
            assert_eq!(codec::decode(&other, frame), Err(Error::Crc));
            let mut without_log_id = frame.clone();
            let checksum = raw::crc(&[&frame[..4], &frame[8..]].concat());
            without_log_id[4..8].copy_from_slice(&checksum.to_le_bytes());
            assert_eq!(codec::decode(&log(), &without_log_id), Err(Error::Crc));
        }
    });
}

/// Given footers, a seal and a locator with extreme field values, one footer with no streams
/// When each is encoded, compared with bytes assembled from format.md, and decoded
/// Then the bytes match and every field round trips, and the locator frame is 40 bytes
///
/// Cases: CODEC-GROUP
#[test]
fn footer_seal_and_locator_frames_round_trip() {
    bounded(|| {
        let long = [255; 1024];
        for streams in [
            vec![],
            vec![codec::StreamTail {
                stream: b"s",
                last_revision: 4,
                last_offset: 5,
                last_position: 64,
            }],
            vec![
                codec::StreamTail {
                    stream: &long,
                    last_revision: u64::MAX,
                    last_offset: u64::MAX - 1,
                    last_position: u64::MAX - 2,
                },
                tail(b"t"),
            ],
        ] {
            let footer = codec::Footer {
                key_filter_position: 0x0102_0304_0506,
                streams,
            };
            let mut body = body_header(2, 7, 9, footer.streams.len() as u32);
            body.extend(footer.key_filter_position.to_le_bytes());
            for tail in &footer.streams {
                body.extend((tail.stream.len() as u16).to_le_bytes());
                body.extend([0, 0]);
                body.extend(tail.last_revision.to_le_bytes());
                body.extend(tail.last_offset.to_le_bytes());
                body.extend(tail.last_position.to_le_bytes());
                body.extend(tail.stream);
            }
            let bytes = codec::encode_footer(&log(), 7, 9, &footer).unwrap();
            assert_eq!(bytes, frame_of(&body));
            assert_eq!(
                codec::decode(&log(), &bytes),
                Ok(codec::Frame {
                    epoch: 7,
                    base_offset: 9,
                    body: codec::Body::Footer(footer)
                })
            );
        }
        let seal = codec::encode_seal(&log(), u64::MAX, u64::MAX);
        assert_eq!(seal, frame_of(&body_header(3, u64::MAX, u64::MAX, 0)));
        assert_eq!(
            codec::decode(&log(), &seal).unwrap().body,
            codec::Body::Seal
        );
        let mut body = body_header(4, 7, 9, 0);
        body.extend(0x1122_3344_5566_7788u64.to_le_bytes());
        let locator = codec::encode_locator(&log(), 7, 9, 0x1122_3344_5566_7788);
        assert_eq!(locator, frame_of(&body));
        assert_eq!(locator.len(), codec::LOCATOR_LEN);
        assert_eq!(
            codec::decode(&log(), &locator),
            Ok(codec::Frame {
                epoch: 7,
                base_offset: 9,
                body: codec::Body::Locator {
                    footer_position: 0x1122_3344_5566_7788
                }
            })
        );
    });
}

/// Given the log id `00 01 02 … 0f`
/// When the empty key and the 15-byte key `00 01 … 0e` are hashed
/// Then the hashes are the two check values of format.md, and another log id hashes differently
///
/// Cases: CODEC-GROUP
#[test]
fn key_hashes_match_the_siphash_check_values() {
    bounded(|| {
        let key: Vec<u8> = (0..15).collect();
        assert_eq!(log().key_hash(b""), 0x726f_db47_dd0e_0e31);
        assert_eq!(log().key_hash(&key), 0xa129_ca61_49be_45e5);
        assert_ne!(LogId::new([9; 16]).key_hash(&key), log().key_hash(&key));
    });
}

/// Given 65 hashes in three filter blocks, one of them 0xa129ca6149be45e5 and the rest in blocks 0 and 2
/// When the key filter frame is built
/// Then block 1 holds exactly the bits 27, 32, 93, 97, 144, 171, 364, 386, 393, 426 and 434
/// And a filter with those bits answers maybe for the hash, and no after any one bit clears
///
/// Cases: CODEC-GROUP
#[test]
fn the_key_filter_sets_the_bits_of_the_format_check_value() {
    bounded(|| {
        const BITS: [usize; 11] = [27, 32, 93, 97, 144, 171, 364, 386, 393, 426, 434];
        let hash = 0xa129_ca61_49be_45e5;
        let mut entries = vec![entry(hash, 64, 0)];
        entries.extend((0..32).map(|i| entry(i, 64, i as u32 + 1)));
        entries.extend((0..32).map(|i| entry(0xffff_ffff_0000_0000 | i, 64, i as u32 + 40)));
        let frames = codec::encode_key_frames(&log(), 1, 1, &mut entries).unwrap();
        assert_eq!(frames.filter.len(), 8 + 24 + 8 + 3 * 64);
        let mut expected = [0u8; 64];
        for bit in BITS {
            expected[bit / 8] |= 1 << (bit % 8);
        }
        let block = 8 + 24 + 8 + 64;
        assert_eq!(frames.filter[block..block + 64], expected);

        let mut body = body_header(5, 1, 1, 65);
        body.extend(0u64.to_le_bytes());
        body.extend([0; 3 * 64]);
        let block = 32 + 64;
        body[block..block + 64].copy_from_slice(&expected);
        assert!(key_filter_of(&frame_of(&body)).may_contain(hash));
        for bit in BITS {
            let mut fewer = body.clone();
            fewer[block + bit / 8] &= !(1 << (bit % 8));
            let filter = key_filter_of(&frame_of(&fewer));
            assert!(!filter.may_contain(hash), "bit {bit}");
        }
        for wrong in [32, 32 + 128] {
            let mut moved = body.clone();
            moved[block..block + 64].fill(0);
            moved[wrong..wrong + 64].copy_from_slice(&expected);
            assert!(!key_filter_of(&frame_of(&moved)).may_contain(hash));
        }
    });
}

/// Given keyed records at known positions, given in any order
/// When both key frames are built
/// Then they equal frames assembled from format.md with the records sorted by hash, position and record
///
/// Cases: CODEC-GROUP
#[test]
fn key_frames_follow_the_format_and_round_trip() {
    bounded(|| {
        let mut entries = [
            entry(u64::MAX, 200, 3),
            entry(5, 100, 1),
            entry(5, 100, 0),
            entry(5, 64, 9),
        ];
        let frames = codec::encode_key_frames(&log(), 3, 12, &mut entries).unwrap();
        let sorted = [
            entry(5, 64, 9),
            entry(5, 100, 0),
            entry(5, 100, 1),
            entry(u64::MAX, 200, 3),
        ];
        assert_eq!(entries, sorted);

        let mut filter = body_header(5, 3, 12, 4);
        filter.extend(5u64.to_le_bytes());
        filter.extend(filter_blocks(&sorted.map(|e| e.hash)));
        assert_eq!(frames.filter, frame_of(&filter));

        let mut index = body_header(6, 3, 12, 4);
        for e in sorted {
            index.extend(e.hash.to_le_bytes());
            index.extend(e.position.to_le_bytes());
            index.extend(e.record.to_le_bytes());
        }
        index.resize(24 + 4080, 0);
        index.extend(4u32.to_le_bytes());
        index.extend([0; 8]);
        let checksum = raw::crc(&index[24..]);
        index.extend(checksum.to_le_bytes());
        assert_eq!(frames.index, frame_of(&index));

        assert_eq!(key_filter_of(&frames.filter).count(), 4);
        let page = &index_pages_of(&frames.index)[0];
        assert_eq!(codec::decode_page(page, 0, 4, 5).unwrap(), sorted);
    });
}

/// Given 1,000 keyed records, and a segment with none
/// When the key frames are built and decoded
/// Then the filter answers maybe for every inserted hash and rarely for others, and no for any hash when it has no blocks
///
/// Cases: CODEC-GROUP
#[test]
fn a_key_filter_answers_maybe_for_every_inserted_hash() {
    bounded(|| {
        let hash = |i: u64| i.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (i << 40);
        let mut entries: Vec<_> = (0..1000).map(|i| entry(hash(i), i as u32, 0)).collect();
        let frames = codec::encode_key_frames(&log(), 1, 1000, &mut entries).unwrap();
        assert_eq!(frames.filter.len(), 8 + 24 + 8 * 4 + 64 * 32);
        assert_eq!(frames.index.len(), 8 + 24 + 4096 * 4);
        let filter = key_filter_of(&frames.filter);
        assert_eq!(filter.count(), 1000);
        assert_eq!(filter.memory_bytes(), 8 * 4 + 64 * 32);
        for i in 0..1000 {
            assert!(filter.may_contain(hash(i)), "hash {i}");
        }
        let hits = (1000..11_000)
            .filter(|&i| filter.may_contain(hash(i)))
            .count();
        assert!(hits < 100, "{hits} of 10000 absent hashes answered maybe");

        let frames = codec::encode_key_frames(&log(), 1, 0, &mut []).unwrap();
        assert_eq!(frames.filter, frame_of(&body_header(5, 1, 0, 0)));
        assert_eq!(frames.index, frame_of(&body_header(6, 1, 0, 0)));
        let filter = key_filter_of(&frames.filter);
        assert_eq!(filter.memory_bytes(), 0);
        for hash in [0, 1, 0xa129_ca61_49be_45e5, u64::MAX] {
            assert!(!filter.may_contain(hash));
            assert_eq!(filter.index_pages(hash), 0..0);
        }
    });
}

/// Given a key index of two pages, with 255 entries and 45
/// When each page is decoded with its fence and place, and then with each page rule broken in turn
/// Then valid pages return their entries, and each broken rule is rejected with its own error
///
/// Cases: CODEC-GROUP
#[test]
fn key_index_page_rules_are_enforced() {
    bounded(|| {
        use codec::Error;
        let mut entries: Vec<_> = (0..300).map(|i| entry(i * 3 + 1, i as u32, 0)).collect();
        let frames = codec::encode_key_frames(&log(), 1, 300, &mut entries).unwrap();
        let pages = index_pages_of(&frames.index);
        assert_eq!(pages.len(), 2);
        let fences = [1, 766];
        assert_eq!(
            codec::decode_page(&pages[0], 0, 300, fences[0]).unwrap(),
            entries[..255]
        );
        assert_eq!(
            codec::decode_page(&pages[1], 1, 300, fences[1]).unwrap(),
            entries[255..]
        );
        let seal = |page: &mut Vec<u8>| {
            let checksum = raw::crc(&page[..4092]);
            page[4092..].copy_from_slice(&checksum.to_le_bytes());
        };
        let check = |number: usize, expected: Error, change: &dyn Fn(&mut Vec<u8>)| {
            let mut page = pages[number].clone();
            change(&mut page);
            assert_eq!(
                codec::decode_page(&page, number, 300, fences[number]),
                Err(expected)
            );
        };
        check(0, Error::Crc, &|p| p[100] ^= 1);
        check(0, Error::Crc, &|p| p[4092] ^= 1);
        check(0, Error::Length, &|p| {
            p.pop();
        });
        check(0, Error::Length, &|p| p.push(0));
        for (number, count) in [(0, 254u32), (0, 0), (0, 256), (1, 255), (1, 44), (1, 46)] {
            check(number, Error::Count, &|p| {
                p[4080..4084].copy_from_slice(&count.to_le_bytes());
                seal(p);
            });
        }
        for at in [4084, 4091] {
            check(0, Error::Reserved, &|p| {
                p[at] = 1;
                seal(p);
            });
        }
        for at in [45 * 16, 45 * 16 + 3, 4079] {
            check(1, Error::Reserved, &|p| {
                p[at] = 1;
                seal(p);
            });
        }
        check(0, Error::Order, &|p| {
            let (first, second) = p.split_at_mut(16);
            first.swap_with_slice(&mut second[..16]);
            seal(p);
        });
        check(0, Error::Order, &|p| {
            p.copy_within(16..32, 32);
            seal(p);
        });
        assert_eq!(
            codec::decode_page(&pages[0], 0, 300, fences[0] + 1),
            Err(Error::Fence)
        );
        assert_eq!(
            codec::decode_page(&pages[0], 2, 300, fences[0]),
            Err(Error::Count)
        );
    });
}

/// Given the entries of one hash that span a page boundary, and of another that span three pages
/// When the key filter names the pages to read and each is decoded
/// Then the pages hold all of the hash's entries, and a hash below every fence starts at page 0
///
/// Cases: CODEC-GROUP
#[test]
fn the_fence_lookup_finds_a_hash_across_page_boundaries() {
    bounded(|| {
        for (low, same, high) in [(250, 20, 330), (10, 520, 70)] {
            let hash = 1_000_000;
            let mut entries: Vec<_> = (0..low).map(|i| entry(i + 1, i as u32, 0)).collect();
            entries.extend((0..same).map(|i| entry(hash, 100 + i as u32, 0)));
            entries.extend((0..high).map(|i| entry(hash + 1 + i, i as u32, 0)));
            let count = entries.len() as u32;
            let frames = codec::encode_key_frames(&log(), 1, 1, &mut entries.clone()).unwrap();
            let filter = key_filter_of(&frames.filter);
            let pages = index_pages_of(&frames.index);
            let range = filter.index_pages(hash);
            let mut found = vec![];
            for number in range.clone() {
                let fence = entries[number * 255].hash;
                found.extend(
                    codec::decode_page(&pages[number], number, count, fence)
                        .unwrap()
                        .into_iter()
                        .filter(|e| e.hash == hash),
                );
            }
            assert_eq!(found.len(), same as usize, "{low} {same} {high}");
            let first = entries.iter().position(|e| e.hash == hash).unwrap();
            let last = first + same as usize - 1;
            assert_eq!(range, first / 255..last / 255 + 1);
            assert!(filter.may_contain(hash));
            assert_eq!(filter.index_pages(0), 0..1);
            assert_eq!(filter.index_pages(u64::MAX).end, pages.len());
        }
    });
}

/// Given entries that repeat, more keyed records than the limit, a footer past the frame body limit,
/// and streams outside the stream id limits
/// When the frames are encoded
/// Then each encode is rejected with its own error and no frame
///
/// Cases: CODEC-GROUP
#[test]
fn encoding_rejects_repeats_and_counts_past_the_limits() {
    bounded(|| {
        use codec::Error;
        let mut repeat = [entry(1, 64, 0), entry(2, 64, 0), entry(1, 64, 0)];
        assert!(matches!(
            codec::encode_key_frames(&log(), 1, 1, &mut repeat),
            Err(Error::Order)
        ));
        let mut same_hash = [entry(1, 64, 0), entry(1, 64, 1), entry(1, 65, 0)];
        assert!(codec::encode_key_frames(&log(), 1, 1, &mut same_hash).is_ok());

        let limit = codec::KEYED_RECORD_LIMIT;
        assert_eq!(limit, 1_044_225);
        let mut too_many = vec![entry(0, 0, 0); limit + 1];
        assert!(matches!(
            codec::encode_key_frames(&log(), 1, 1, &mut too_many),
            Err(Error::Count)
        ));

        let stream = [1; 1024];
        let fits = (codec::FRAME_BODY_LIMIT - 32) / (28 + 1024);
        let streams: Vec<_> = (0..fits).map(|_| tail(&stream)).collect();
        assert!(codec::encode_footer(&log(), 1, 1, &footer(streams)).is_ok());
        let streams: Vec<_> = (0..fits + 1).map(|_| tail(&stream)).collect();
        assert!(matches!(
            codec::encode_footer(&log(), 1, 1, &footer(streams)),
            Err(Error::Length)
        ));
        for bad in [&[][..], &[0; 1025][..]] {
            assert!(matches!(
                codec::encode_footer(&log(), 1, 1, &footer(vec![tail(bad)])),
                Err(Error::Stream)
            ));
        }
    });
}

/// Given a valid frame of each closing kind
/// When one field at a time is broken and the checksum is made right again
/// Then the decoder rejects an unknown kind, flags, reserved fields, counts and sizes that its bytes disprove
///
/// Cases: CODEC-GROUP
#[test]
fn malformed_closing_frames_are_rejected() {
    bounded(|| {
        use codec::Error;
        let streams = vec![tail(b"s"), tail(b"t")];
        let mut entries: Vec<_> = (0..300).map(|i| entry(i * 3 + 1, 64, i as u32)).collect();
        let keys = codec::encode_key_frames(&log(), 1, 300, &mut entries).unwrap();
        let valid = [
            codec::encode_footer(&log(), 1, 300, &footer(streams)).unwrap(),
            codec::encode_seal(&log(), 1, 300),
            codec::encode_locator(&log(), 1, 300, 64),
            keys.filter,
            keys.index,
        ];
        let [footer, seal, locator, filter, index] = &valid;
        let resize = |frame: &[u8], len: usize| {
            let mut frame = frame.to_vec();
            frame.resize(len, 0);
            frame[..4].copy_from_slice(&((len - 8) as u32).to_le_bytes());
            frame
        };
        let with = |frame: &[u8], change: &dyn Fn(&mut Vec<u8>)| {
            let mut frame = frame.to_vec();
            change(&mut frame);
            raw::rechecksum(&CHECK_ID, &mut frame);
            codec::decode(&log(), &frame).map(|_| ())
        };
        for frame in &valid {
            assert!(codec::decode(&log(), frame).is_ok());
            assert_eq!(with(frame, &|f| f[8] = 0), Err(Error::Kind));
            assert_eq!(with(frame, &|f| f[8] = 7), Err(Error::Kind));
            for at in [9, 10, 11] {
                assert_eq!(with(frame, &|f| f[at] = 1), Err(Error::Reserved));
            }
        }
        assert_eq!(with(seal, &|f| f[28] = 1), Err(Error::Count));
        assert_eq!(with(locator, &|f| f[28] = 1), Err(Error::Count));
        assert_eq!(with(seal, &|f| *f = resize(f, 33)), Err(Error::Length));
        assert_eq!(with(locator, &|f| *f = resize(f, 41)), Err(Error::Length));
        assert_eq!(with(locator, &|f| *f = resize(f, 32)), Err(Error::Length));
        assert_eq!(with(seal, &|f| f[8] = 4), Err(Error::Length));
        assert_eq!(with(locator, &|f| f[8] = 3), Err(Error::Length));

        assert_eq!(with(footer, &|f| f[28] = 3), Err(Error::Count));
        assert_eq!(with(footer, &|f| f[28] = 1), Err(Error::Length));
        assert_eq!(with(footer, &|f| f[40] = 0), Err(Error::Stream));
        assert_eq!(with(footer, &|f| f[41] = 4), Err(Error::Stream));
        assert_eq!(with(footer, &|f| f[42] = 1), Err(Error::Reserved));
        assert_eq!(with(footer, &|f| f[43] = 1), Err(Error::Reserved));
        let longer = footer.len() + 1;
        assert_eq!(
            with(footer, &|f| *f = resize(f, longer)),
            Err(Error::Length)
        );
        let shorter = footer.len() - 1;
        assert_eq!(
            with(footer, &|f| *f = resize(f, shorter)),
            Err(Error::Count)
        );

        let too_many = (codec::KEYED_RECORD_LIMIT + 1) as u32;
        for frame in [filter, index] {
            for (count, expected) in [
                (too_many, Error::Count),
                (0, Error::Length),
                (255, Error::Length),
            ] {
                assert_eq!(
                    with(frame, &|f| f[28..32].copy_from_slice(&count.to_le_bytes())),
                    Err(expected)
                );
            }
        }
        let longer = filter.len() + 64;
        assert_eq!(
            with(filter, &|f| *f = resize(f, longer)),
            Err(Error::Length)
        );
        let shorter = filter.len() - 1;
        assert_eq!(
            with(filter, &|f| *f = resize(f, shorter)),
            Err(Error::Length)
        );
        let longer = index.len() + 1;
        assert_eq!(with(index, &|f| *f = resize(f, longer)), Err(Error::Length));
        let shorter = index.len() - 4096;
        assert_eq!(
            with(index, &|f| *f = resize(f, shorter)),
            Err(Error::Length)
        );
        assert_eq!(with(filter, &|f| f[32 + 7] = 1), Err(Error::Order));
    });
}
