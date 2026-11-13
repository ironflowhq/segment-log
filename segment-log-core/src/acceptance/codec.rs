mod closing;

use super::support::{bounded, raw};
use crate::codec::{self, Entry, Group, LogId};
const CHECK_ID: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
fn log() -> LogId {
    LogId::new(CHECK_ID)
}
fn raw_log() -> LogId {
    LogId::new(raw::LOG_ID)
}
fn group_of(frame: codec::Frame<'_>) -> Group<'_> {
    frame.into_group().expect("a group frame")
}
/// Given the Castagnoli check string and a keyed binary group
/// When they are encoded and decoded
/// Then the check value and hand-built bytes match and all fields round trip
/// Cases: CODEC-GROUP
#[test]
fn group_bytes_follow_the_provisional_format() {
    bounded(|| {
        assert_eq!(codec::crc32c(b"123456789"), 0xe3069283);
        let group = Group {
            epoch: 2,
            base_offset: 3,
            entries: vec![Entry {
                stream: b"s",
                key: Some(b"k"),
                payload: &[0, 255],
                revision: 4,
            }],
        };
        let bytes = codec::encode_group(&log(), &group).unwrap();
        let expected = [
            44, 0, 0, 0, 0xf0, 0x85, 0xc8, 0x87, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0,
            0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 0, 2, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, b's', b'k', 0,
            255,
        ];
        assert_eq!(bytes, expected);
        assert_eq!(group_of(codec::decode(&log(), &bytes).unwrap()), group);
    });
}
fn checksum(bytes: &mut [u8]) {
    raw::rechecksum(&CHECK_ID, bytes);
}
/// Given a valid group frame and malformed variants of each envelope and entry field
/// When the plain byte decoder reads them
/// Then it rejects bad checksums, lengths, kinds, flags, reserved fields, counts and bounds
/// Cases: CODEC-GROUP
#[test]
fn malformed_groups_are_rejected_without_panicking() {
    bounded(|| {
        use codec::Error;
        let group = Group {
            epoch: 0,
            base_offset: 0,
            entries: vec![Entry {
                stream: b"s",
                key: None,
                payload: b"p",
                revision: 0,
            }],
        };
        let valid = codec::encode_group(&log(), &group).unwrap();
        for end in 0..valid.len() {
            assert!(codec::decode(&log(), &valid[..end]).is_err());
        }
        for (pos, value, error) in [
            (8, 0, Error::Kind),
            (8, 7, Error::Kind),
            (9, 1, Error::Reserved),
            (10, 1, Error::Reserved),
            (11, 1, Error::Reserved),
            (28, 0, Error::Count),
            (28, 255, Error::Count),
            (32, 0, Error::Stream),
            (33, 5, Error::Stream),
            (35, 2, Error::Key),
            (36, 255, Error::Length),
        ] {
            let mut bytes = valid.clone();
            bytes[pos] = value;
            checksum(&mut bytes);
            assert_eq!(codec::decode(&log(), &bytes), Err(error), "byte {pos}");
        }
        let mut bytes = valid.clone();
        bytes[20..28].fill(255);
        checksum(&mut bytes);
        assert_eq!(codec::decode(&log(), &bytes), Err(Error::Offset));
        let mut bytes = valid.clone();
        bytes[12] ^= 1;
        assert_eq!(codec::decode(&log(), &bytes), Err(Error::Crc));
        for len in [
            0u32,
            23,
            8 * 1024 * 1024 + 1,
            16 * 1024 * 1024 + 1,
            u32::MAX,
        ] {
            let mut bytes = valid.clone();
            bytes[..4].copy_from_slice(&len.to_le_bytes());
            assert_eq!(codec::decode(&log(), &bytes), Err(Error::Length));
        }
        let mut extra = valid.clone();
        extra.push(0);
        let len = (extra.len() - 8) as u32;
        extra[..4].copy_from_slice(&len.to_le_bytes());
        checksum(&mut extra);
        assert_eq!(codec::decode(&log(), &extra), Err(Error::Length));
    });
}
/// Given binary records with absent and maximum length keys and stream ids
/// When many groups round trip and invalid groups are encoded
/// Then valid bytes retain every field and invalid inputs return errors
/// Cases: CODEC-GROUP
#[test]
fn binary_groups_round_trip_and_encoding_enforces_limits() {
    bounded(|| {
        use codec::Error;
        let stream = vec![255; 1024];
        let key = vec![0; 256];
        for size in 0..64 {
            let payload = vec![size as u8; size * 17];
            let g = Group {
                epoch: u64::MAX,
                base_offset: 5,
                entries: vec![
                    Entry {
                        stream: &stream,
                        key: Some(&key),
                        payload: &payload,
                        revision: u64::MAX,
                    },
                    Entry {
                        stream: b"x",
                        key: None,
                        payload: &[],
                        revision: 0,
                    },
                ],
            };
            let bytes = codec::encode_group(&log(), &g).unwrap();
            assert_eq!(group_of(codec::decode(&log(), &bytes).unwrap()), g);
        }
        for (stream, key, error) in [
            (&[][..], None, Error::Stream),
            (&vec![0; 1025], None, Error::Stream),
            (b"s", Some(&[][..]), Error::Key),
            (b"s", Some(&vec![0; 257][..]), Error::Key),
        ] {
            let g = Group {
                epoch: 0,
                base_offset: 0,
                entries: vec![Entry {
                    stream,
                    key,
                    payload: &[],
                    revision: 0,
                }],
            };
            assert_eq!(codec::encode_group(&log(), &g), Err(error));
        }
        let mut g = Group {
            epoch: 0,
            base_offset: 0,
            entries: vec![],
        };
        assert_eq!(codec::encode_group(&log(), &g), Err(Error::Count));
        let payload = vec![0; 8 * 1024 * 1024 - 41];
        g.entries.push(Entry {
            stream: b"s",
            key: None,
            payload: &payload,
            revision: 0,
        });
        assert_eq!(
            codec::encode_group(&log(), &g).unwrap().len(),
            8 * 1024 * 1024 + 8
        );
        g.entries.push(Entry {
            stream: b"x",
            key: None,
            payload: &[],
            revision: 0,
        });
        assert_eq!(codec::encode_group(&log(), &g), Err(Error::Length));
    });
}

/// Given the specified header fields and an independently checksummed byte fixture
/// When a header is encoded and decoded
/// Then every byte matches the fixture and malformed headers are rejected
/// Cases: CODEC-GROUP
#[test]
fn header_bytes_and_validation_follow_the_format() {
    bounded(|| {
        let mut expected = [0; 64];
        expected[..8].copy_from_slice(b"SGLG\x02\0\x40\0");
        expected[8..24].copy_from_slice(&raw::LOG_ID);
        expected[24] = 3;
        expected[32] = 2;
        fn crc(bytes: &[u8]) -> u32 {
            let mut n = 0xffff_ffffu32;
            for byte in bytes {
                for bit in 0..8 {
                    let low = (n ^ u32::from(byte >> bit)) & 1;
                    n >>= 1;
                    if low != 0 {
                        n ^= 0x82f6_3b78;
                    }
                }
            }
            n ^ 0xffff_ffff
        }
        let sum = crc(&expected[..60]);
        expected[60..].copy_from_slice(&sum.to_le_bytes());
        let header = codec::Header {
            log_id: raw::LOG_ID,
            base_offset: 3,
            epoch: 2,
        };
        assert_eq!(codec::encode_header(&header), expected);
        assert_eq!(codec::decode_header(&expected), Ok(header));
        for end in 0..64 {
            assert!(codec::decode_header(&expected[..end]).is_err());
        }
        for pos in [0, 4, 6, 40, 59, 60] {
            let mut bad = expected;
            bad[pos] ^= 1;
            if pos < 60 {
                let sum = crc(&bad[..60]);
                bad[60..].copy_from_slice(&sum.to_le_bytes());
            }
            assert!(codec::decode_header(&bad).is_err(), "byte {pos}");
        }
    });
}

/// Given all two-byte values and groups with empty and variable binary payloads
/// When CRC32C and complete frames are compared with the independent bitwise fixture
/// Then checksums and every durable byte remain identical
/// Cases: CODEC-GROUP
#[test]
fn checksums_and_frames_match_the_bitwise_fixture() {
    assert_eq!(codec::crc32c(&[]), 0);
    for word in 0..=u16::MAX {
        let bytes = word.to_le_bytes();
        assert_eq!(codec::crc32c(&bytes), raw::crc(&bytes));
    }
    let payload: Vec<u8> = (0..8193)
        .map(|n| ((n * 97 + n / 251) % 256) as u8)
        .collect();
    for len in [0, 1, 7, 8, 255, 256, 257, 4095, 4096, 8193] {
        let bytes = &payload[..len];
        assert_eq!(codec::crc32c(bytes), raw::crc(bytes));
        let group = Group {
            epoch: 3,
            base_offset: 5,
            entries: vec![Entry {
                stream: b"s",
                key: None,
                payload: bytes,
                revision: 5,
            }],
        };
        let expected = raw::group(3, 5, &[bytes]);
        assert_eq!(codec::encode_group(&raw_log(), &group).unwrap(), expected);
        assert_eq!(
            group_of(codec::decode(&raw_log(), &expected).unwrap()),
            group
        );
    }
}

/// Given byte strings and group frames of every length across four sixteen-byte checksum steps
/// When CRC32C and complete frames are compared with the independent bitwise fixture
/// Then every checksum and every durable byte match, whatever remainder follows the last step
/// Cases: CODEC-GROUP
#[test]
fn checksums_match_the_bitwise_fixture_at_every_step_remainder() {
    let bytes: Vec<u8> = (0..64).map(|n| ((n * 151 + 13) % 256) as u8).collect();
    for len in 0..=bytes.len() {
        let payload = &bytes[..len];
        assert_eq!(codec::crc32c(payload), raw::crc(payload), "length {len}");
        let group = Group {
            epoch: 3,
            base_offset: 5,
            entries: vec![Entry {
                stream: b"s",
                key: None,
                payload,
                revision: 5,
            }],
        };
        let expected = raw::group(3, 5, &[payload]);
        assert_eq!(
            codec::encode_group(&raw_log(), &group).unwrap(),
            expected,
            "length {len}"
        );
        assert_eq!(
            group_of(codec::decode(&raw_log(), &expected).unwrap()),
            group,
            "length {len}"
        );
    }
}

/// Given byte strings and group frames that end on, before and after each 4 KiB checksum round
/// When CRC32C and complete frames are compared with the independent bitwise fixture
/// Then every checksum and every durable byte match, however many rounds the bytes fill
/// Cases: CODEC-GROUP
#[test]
fn checksums_match_the_bitwise_fixture_across_checksum_rounds() {
    let bytes: Vec<u8> = (0..4 * 4096 + 1040)
        .map(|n| ((n * 151 + n / 4093 + 13) % 256) as u8)
        .collect();
    for rounds in 0..=4 {
        for remainder in [0, 1, 15, 16, 17, 1023, 1024, 1025, 4095] {
            let len = (rounds * 4096 + remainder).min(bytes.len());
            let payload = &bytes[..len];
            assert_eq!(codec::crc32c(payload), raw::crc(payload), "length {len}");
            let expected = raw::group(3, 5, &[payload]);
            assert_eq!(
                group_of(codec::decode(&raw_log(), &expected).unwrap()).entries[0].payload,
                payload,
                "length {len}"
            );
        }
    }
}

/// Given valid group frames whose lengths take every remainder of an eight-byte word
/// When any one bit of a frame changes
/// Then the decoder rejects the frame: a size change by its length, any other change by its checksum
/// Cases: CODEC-GROUP
#[test]
fn every_single_bit_change_in_a_frame_is_rejected() {
    use codec::Error;
    for len in 0..8 {
        let frame = raw::group(3, 5, &[&[0xa5; 8][..len]]);
        assert!(codec::decode(&raw_log(), &frame).is_ok());
        for bit in 0..frame.len() * 8 {
            let mut changed = frame.clone();
            changed[bit / 8] ^= 1 << (bit % 8);
            let expected = if bit < 32 { Error::Length } else { Error::Crc };
            assert_eq!(
                codec::decode(&raw_log(), &changed),
                Err(expected),
                "length {len} bit {bit}"
            );
        }
    }
}
