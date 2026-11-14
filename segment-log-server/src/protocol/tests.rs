//! Byte-level checks against protocol.md. The golden byte strings are written out from its tables,
//! so a change to a width or an offset fails here.

use super::*;
use segment_log_core::Position;

fn header(body_len: u32, kind: u8) -> [u8; HEADER_LEN] {
    let mut h = [0; HEADER_LEN];
    h[..4].copy_from_slice(&body_len.to_le_bytes());
    h[4] = kind;
    h
}

fn roundtrip(message: &[u8]) -> Request {
    let h = decode_header(message[..HEADER_LEN].try_into().unwrap()).unwrap();
    assert_eq!(h.body_len, message.len() - HEADER_LEN);
    let request = decode_request(h.kind, &message[HEADER_LEN..]).unwrap();
    assert_eq!(encode_request(&request), message);
    request
}

fn record(
    stream: &[u8],
    expected: ExpectedRevision,
    key: Option<&[u8]>,
    payload: &[u8],
) -> NewRecord {
    NewRecord {
        stream: stream.to_vec(),
        expected,
        key: key.map(<[u8]>::to_vec),
        payload: payload.to_vec(),
    }
}

#[test]
fn hello_request_matches_the_layout() {
    let bytes = [
        8, 0, 0, 0, HELLO, 0, 0, 0, b'S', b'G', b'L', b'P', 1, 0, 0, 0,
    ];
    assert_eq!(encode_request(&Request::Hello { version: 1 }), bytes);
    assert!(matches!(roundtrip(&bytes), Request::Hello { version: 1 }));
}

#[test]
fn append_request_matches_the_layout() {
    let submit = Submit {
        records: vec![
            record(b"s", ExpectedRevision::Empty, None, b"p"),
            record(b"t", ExpectedRevision::At(7), Some(b"k"), b""),
        ],
    };
    let mut body = vec![2, 0, 0, 0];
    body.extend([
        1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b's', b'p',
    ]);
    body.extend([
        1, 0, 1, 0, 0, 0, 0, 0, 1, 7, 0, 0, 0, 0, 0, 0, 0, b't', b'k',
    ]);
    let mut bytes = header(body.len() as u32, APPEND).to_vec();
    bytes.extend(body);
    assert_eq!(encode_request(&Request::Append(submit)), bytes);
    roundtrip(&bytes);
}

#[test]
fn fixed_requests_round_trip() {
    for request in [
        Request::Read { from: 3, max: 9 },
        Request::Watermark,
        Request::StreamHead(b"orders".to_vec()),
        Request::Seal,
        Request::Append(Submit { records: vec![] }),
    ] {
        roundtrip(&encode_request(&request));
    }
    let read = encode_request(&Request::Read { from: 3, max: 9 });
    assert_eq!(read[HEADER_LEN..], [3, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0]);
}

#[test]
fn non_canonical_requests_are_malformed() {
    let mut nonzero_reserved = header(0, WATERMARK);
    nonzero_reserved[7] = 1;
    assert_eq!(decode_header(nonzero_reserved), Err(Malformed));

    let cases: [(u8, &[u8]); 9] = [
        (HELLO, b"SGLQ\x01\x00\x00\x00"), // wrong magic
        (HELLO, b"SGLP\x01\x00\x01\x00"), // nonzero reserved
        (HELLO, b"SGLP\x01\x00"),         // short
        (READ, &[0; 13]),                 // long
        (WATERMARK, &[0]),                // a body where none belongs
        (SEAL, &[0]),
        (APPEND, &[1, 0, 0, 0]),    // count names a missing entry
        (APPEND, &[0, 0, 0, 0, 9]), // trailing bytes
        (9, &[]),                   // unknown kind
    ];
    for (kind, body) in cases {
        assert!(
            decode_request(kind, body).is_err(),
            "kind {kind} body {body:?}"
        );
    }

    let mut empty_with_revision = vec![1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0];
    empty_with_revision.extend([1, 0, 0, 0, 0, 0, 0, 0, b's']);
    assert!(decode_request(APPEND, &empty_with_revision).is_err());
    let mut bad_expected = vec![1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2];
    bad_expected.extend([0, 0, 0, 0, 0, 0, 0, 0, b's']);
    assert!(decode_request(APPEND, &bad_expected).is_err());
}

#[test]
fn a_huge_count_reserves_only_what_the_body_holds() {
    let body = [0xff, 0xff, 0xff, 0xff];
    assert!(decode_request(APPEND, &body).is_err());
}

#[test]
fn bodies_above_the_cap_are_discarded_or_malformed_before_reading() {
    let cap = frame_cap(1024 * 1024);
    assert_eq!(cap, 2 * 1024 * 1024);
    assert_eq!(frame_cap(41), 64 * 1024);
    let at = |len: usize, kind| Header {
        body_len: len,
        kind,
    };
    assert_eq!(admit(at(cap, APPEND), cap), Ok(Admit::Read));
    assert_eq!(admit(at(cap + 1, APPEND), cap), Ok(Admit::Discard));
    assert_eq!(admit(at(cap + 1, STREAM_HEAD), cap), Err(Malformed));
    assert_eq!(admit(at(u32::MAX as usize, READ), cap), Err(Malformed));
}

/// protocol.md argues that an append above the cap is one the core refuses with `TooLarge`. The
/// worst case packs the most records into the body: one-byte stream ids, no keys, no payloads.
#[test]
fn an_append_above_the_cap_has_a_group_body_above_the_group_cap() {
    for group_byte_cap in [41, 1000, 1024 * 1024, 8 * 1024 * 1024] {
        let cap = frame_cap(group_byte_cap);
        let records = (cap - 4) / (APPEND_ENTRY_LEN + 1) + 1;
        let wire_body = 4 + records * (APPEND_ENTRY_LEN + 1);
        assert!(wire_body > cap);
        let group_body = 24 + records * (16 + 1); // format.md, group record entry
        assert!(group_body > group_byte_cap, "cap {group_byte_cap}");
    }
}

#[test]
fn success_answers_match_the_layout() {
    let committed = Committed {
        records: vec![Position {
            offset: 5,
            revision: 2,
        }],
        deduplicated: true,
    };
    let bytes = encode_answer(APPEND, &Answer::Append(&committed));
    let mut expected = header(26, APPEND | 0x80).to_vec();
    expected.extend([0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
    expected.extend([5, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(bytes, expected);

    let hello = encode_answer(HELLO, &Answer::Hello { frame_cap: 65536 });
    assert_eq!(hello[4], 0x81);
    assert_eq!(hello[HEADER_LEN..], [0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);

    let head = StreamHead {
        revision: 3,
        offset: 4,
    };
    let present = encode_answer(STREAM_HEAD, &Answer::StreamHead(Some(&head)));
    assert_eq!(present.len(), HEADER_LEN + 26);
    assert_eq!(present[HEADER_LEN + 2], 1);
    assert_eq!(present[HEADER_LEN + 10], 3);
    assert_eq!(present[HEADER_LEN + 18], 4);
    let absent = encode_answer(STREAM_HEAD, &Answer::StreamHead(None));
    assert_eq!(absent[HEADER_LEN..], [0; 26]);

    let record = Record {
        offset: 1,
        stream: b"s".to_vec(),
        revision: 0,
        key: Some(b"k".to_vec()),
        payload: b"pp".to_vec(),
    };
    let read = encode_answer(READ, &Answer::Read(std::slice::from_ref(&record)));
    let mut expected = vec![0, 0, 1, 0, 0, 0];
    expected.extend([1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    expected.extend([1, 0, 1, 0, 2, 0, 0, 0, b's', b'k', b'p', b'p']);
    assert_eq!(read[HEADER_LEN..], expected);
    assert_eq!(
        encode_answer(WATERMARK, &Answer::Watermark(9))[HEADER_LEN..],
        [0, 0, 9, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        encode_answer(SEAL, &Answer::Seal),
        [2, 0, 0, 0, 0x86, 0, 0, 0, 0, 0]
    );
}

#[test]
fn every_core_outcome_keeps_its_own_code() {
    let statuses = [
        Status::from(AppendError::Full),
        Status::from(AppendError::TooLarge),
        Status::from(AppendError::Invalid("empty submit")),
        Status::from(AppendError::Sealed),
        Status::from(AppendError::Poisoned),
        Status::from(CommitError::Exhausted),
        Status::from(CommitError::WrongRevision {
            index: 1,
            actual: None,
        }),
        Status::from(CommitError::KeyConflict { index: 0 }),
        Status::from(CommitError::PartialRetry),
        Status::from(CommitError::Indeterminate),
        Status::from(CommitError::Poisoned),
        Status::from(ReadError::Corrupt {
            segment: 0,
            position: 64,
        }),
        Status::from(ReadError::Io(std::io::ErrorKind::UnexpectedEof.into())),
        Status::Malformed,
        Status::UnsupportedVersion,
    ];
    let codes: Vec<u16> = statuses.iter().map(Status::code).collect();
    assert_eq!(codes, (1..=15).collect::<Vec<_>>());
    assert_eq!(Status::from(SealError::Poisoned), Status::Poisoned);
}

#[test]
fn failure_answers_carry_their_details() {
    let body = |kind, status| encode_answer(kind, &Answer::Failed(status))[HEADER_LEN..].to_vec();
    assert_eq!(
        body(
            APPEND,
            Status::WrongRevision {
                index: 2,
                actual: Some(6)
            }
        ),
        [7, 0, 2, 0, 0, 0, 1, 6, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        body(
            APPEND,
            Status::WrongRevision {
                index: 0,
                actual: None
            }
        ),
        [7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        body(APPEND, Status::KeyConflict { index: 3 }),
        [8, 0, 3, 0, 0, 0]
    );
    assert_eq!(
        body(APPEND, Status::Invalid("ab")),
        [3, 0, 2, 0, b'a', b'b']
    );
    assert_eq!(
        body(READ, Status::Io(std::io::ErrorKind::Other)),
        [13, 0, 5, 0, b'O', b't', b'h', b'e', b'r']
    );
    assert_eq!(body(HELLO, Status::UnsupportedVersion), [15, 0, 1, 0]);
    let malformed = encode_answer(MALFORMED_KIND, &Answer::Failed(Status::Malformed));
    assert_eq!(malformed, [2, 0, 0, 0, 0x80, 0, 0, 0, 14, 0]);
}

#[test]
fn a_read_answer_holds_the_longest_prefix_within_the_cap_and_at_least_one_record() {
    let record = |len| Record {
        offset: 0,
        stream: b"s".to_vec(),
        revision: 0,
        key: None,
        payload: vec![0; len],
    };
    let entry = 24 + 1 + 100;
    let records = [record(100), record(100), record(100)];
    assert_eq!(fitting_records(&records, 6 + 2 * entry), 2);
    assert_eq!(fitting_records(&records, 6 + 2 * entry - 1), 1);
    assert_eq!(fitting_records(&[record(1000)], 10), 1);
    assert_eq!(fitting_records(&[], 10), 0);
    let fit = fitting_records(&records, 6 + 2 * entry);
    assert_eq!(
        encode_answer(READ, &Answer::Read(&records[..fit])).len(),
        HEADER_LEN + 6 + 2 * entry
    );
}
