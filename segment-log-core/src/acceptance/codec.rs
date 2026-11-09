use super::support::bounded;
use crate::codec::{self, Entry, Group};
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
        let bytes = codec::encode(&group).unwrap();
        // CRC bytes were computed independently using the reflected polynomial division.
        let expected = [
            44, 0, 0, 0, 0x48, 0x3b, 0x11, 0xa2, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0,
            0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 0, 2, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, b's', b'k', 0,
            255,
        ];
        assert_eq!(bytes, expected);
        assert_eq!(codec::decode(&bytes).unwrap(), group);
    });
}
fn checksum(bytes: &mut [u8]) {
    let input: Vec<_> = bytes[..4].iter().chain(&bytes[8..]).copied().collect();
    bytes[4..8].copy_from_slice(&codec::crc32c(&input).to_le_bytes());
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
        let valid = codec::encode(&group).unwrap();
        for end in 0..valid.len() {
            assert!(codec::decode(&valid[..end]).is_err());
        }
        for (pos, value, error) in [
            (8, 2, Error::Kind),
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
            assert_eq!(codec::decode(&bytes), Err(error), "byte {pos}");
        }
        let mut bytes = valid.clone();
        bytes[20..28].fill(255);
        checksum(&mut bytes);
        assert_eq!(codec::decode(&bytes), Err(Error::Offset));
        let mut bytes = valid.clone();
        bytes[12] ^= 1;
        assert_eq!(codec::decode(&bytes), Err(Error::Crc));
        for len in [
            0u32,
            23,
            8 * 1024 * 1024 + 1,
            16 * 1024 * 1024 + 1,
            u32::MAX,
        ] {
            let mut bytes = valid.clone();
            bytes[..4].copy_from_slice(&len.to_le_bytes());
            assert_eq!(codec::decode(&bytes), Err(Error::Length));
        }
        let mut extra = valid.clone();
        extra.push(0);
        let len = (extra.len() - 8) as u32;
        extra[..4].copy_from_slice(&len.to_le_bytes());
        checksum(&mut extra);
        assert_eq!(codec::decode(&extra), Err(Error::Length));
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
            assert_eq!(codec::decode(&codec::encode(&g).unwrap()).unwrap(), g);
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
            assert_eq!(codec::encode(&g), Err(error));
        }
        let mut g = Group {
            epoch: 0,
            base_offset: 0,
            entries: vec![],
        };
        assert_eq!(codec::encode(&g), Err(Error::Count));
        let payload = vec![0; 8 * 1024 * 1024 - 41];
        g.entries.push(Entry {
            stream: b"s",
            key: None,
            payload: &payload,
            revision: 0,
        });
        assert_eq!(codec::encode(&g).unwrap().len(), 8 * 1024 * 1024 + 8);
        g.entries.push(Entry {
            stream: b"x",
            key: None,
            payload: &[],
            revision: 0,
        });
        assert_eq!(codec::encode(&g), Err(Error::Length));
    });
}
