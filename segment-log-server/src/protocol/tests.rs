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
    assert_eq!(frame_cap(49), 64 * 1024);
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
    // 49 is the smallest group byte cap (format.md, group record entry).
    for group_byte_cap in [49, 1000, 1024 * 1024, 8 * 1024 * 1024] {
        let cap = frame_cap(group_byte_cap);
        let records = (cap - 4) / (APPEND_ENTRY_LEN + 1) + 1;
        let wire_body = 4 + records * (APPEND_ENTRY_LEN + 1);
        assert!(wire_body > cap);
        let group_body = 32 + records * (16 + 1); // format.md, group record entry
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

#[test]
fn the_read_limit_never_asks_for_more_records_than_an_answer_holds() {
    assert_eq!(read_limit(10, 65536), 10);
    assert_eq!(read_limit(u32::MAX, 65536), 65536 / 25);
    let records: Vec<Record> = (0..read_limit(u32::MAX, 65536) + 10)
        .map(|_| Record {
            offset: 0,
            stream: b"s".to_vec(),
            revision: 0,
            key: None,
            payload: vec![],
        })
        .collect();
    assert!(fitting_records(&records, 65536) <= read_limit(u32::MAX, 65536));
}

/// The vectors in conformance/wire.txt, which the Go client checks too.
fn shared_vectors() -> Vec<(&'static str, Vec<u8>)> {
    include_str!("../../../conformance/wire.txt")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (name, hex) = line.split_once(':').expect("name: hex");
            let digits: Vec<u8> = hex.bytes().filter(|b| *b != b' ').collect();
            let bytes = digits
                .chunks(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect();
            (name, bytes)
        })
        .collect()
}

#[test]
fn the_shared_wire_vectors_match_the_encoder_and_decoder() {
    let committed = Committed {
        records: vec![Position {
            offset: 5,
            revision: 2,
        }],
        deduplicated: true,
    };
    let two = Committed {
        records: vec![
            Position {
                offset: 5,
                revision: 2,
            },
            Position {
                offset: 6,
                revision: 0,
            },
        ],
        deduplicated: false,
    };
    let read = [
        Record {
            offset: 1,
            stream: b"s".to_vec(),
            revision: 0,
            key: Some(b"k".to_vec()),
            payload: b"pp".to_vec(),
        },
        Record {
            offset: 2,
            stream: b"s".to_vec(),
            revision: 1,
            key: None,
            payload: b"q".to_vec(),
        },
    ];
    let head = StreamHead {
        revision: 3,
        offset: 4,
    };
    let failed = |kind, status| encode_answer(kind, &Answer::Failed(status));
    let append = Request::Append(Submit {
        records: vec![
            record(b"s", ExpectedRevision::Empty, None, b"p"),
            record(b"t", ExpectedRevision::At(7), Some(b"k"), b""),
        ],
    });
    let requests = [
        ("hello", Request::Hello { version: 1 }),
        ("append", append),
        ("read", Request::Read { from: 3, max: 9 }),
        ("watermark", Request::Watermark),
        ("stream-head", Request::StreamHead(b"orders".to_vec())),
        ("seal", Request::Seal),
    ];
    let answers = [
        (
            "hello-ok",
            encode_answer(HELLO, &Answer::Hello { frame_cap: 65536 }),
        ),
        (
            "append-ok",
            encode_answer(APPEND, &Answer::Append(&committed)),
        ),
        (
            "append-ok-two",
            encode_answer(APPEND, &Answer::Append(&two)),
        ),
        ("read-ok", encode_answer(READ, &Answer::Read(&read[..1]))),
        ("read-ok-two", encode_answer(READ, &Answer::Read(&read))),
        ("read-empty", encode_answer(READ, &Answer::Read(&[]))),
        (
            "watermark-ok",
            encode_answer(WATERMARK, &Answer::Watermark(9)),
        ),
        (
            "stream-head-present",
            encode_answer(STREAM_HEAD, &Answer::StreamHead(Some(&head))),
        ),
        (
            "stream-head-absent",
            encode_answer(STREAM_HEAD, &Answer::StreamHead(None)),
        ),
        ("seal-ok", encode_answer(SEAL, &Answer::Seal)),
        ("full", failed(APPEND, Status::Full)),
        ("too-large", failed(APPEND, Status::TooLarge)),
        ("invalid", failed(APPEND, Status::Invalid("ab"))),
        ("sealed", failed(APPEND, Status::Sealed)),
        ("poisoned", failed(APPEND, Status::Poisoned)),
        ("exhausted", failed(APPEND, Status::Exhausted)),
        (
            "wrong-revision",
            failed(
                APPEND,
                Status::WrongRevision {
                    index: 2,
                    actual: Some(6),
                },
            ),
        ),
        (
            "wrong-revision-none",
            failed(
                APPEND,
                Status::WrongRevision {
                    index: 0,
                    actual: None,
                },
            ),
        ),
        (
            "key-conflict",
            failed(APPEND, Status::KeyConflict { index: 3 }),
        ),
        ("partial-retry", failed(APPEND, Status::PartialRetry)),
        ("indeterminate", failed(APPEND, Status::Indeterminate)),
        (
            "poisoned-before-write",
            failed(APPEND, Status::PoisonedBeforeWrite),
        ),
        (
            "corrupt",
            failed(
                READ,
                Status::Corrupt {
                    segment: 1,
                    position: 64,
                },
            ),
        ),
        ("io", failed(READ, Status::Io(std::io::ErrorKind::Other))),
        (
            "unsupported-version",
            failed(HELLO, Status::UnsupportedVersion),
        ),
        ("malformed", failed(MALFORMED_KIND, Status::Malformed)),
    ];
    let vectors = shared_vectors();
    let mut checked = Vec::new();
    for (name, request) in &requests {
        let bytes = &vectors.iter().find(|v| v.0 == *name).expect(name).1;
        assert_eq!(&encode_request(request), bytes, "{name}");
        roundtrip(bytes);
        checked.push(*name);
    }
    for (name, encoded) in &answers {
        let bytes = &vectors.iter().find(|v| v.0 == *name).expect(name).1;
        assert_eq!(encoded, bytes, "{name}");
        checked.push(*name);
    }
    let names: Vec<&str> = vectors.iter().map(|v| v.0).collect();
    assert_eq!(
        checked, names,
        "every vector has exactly one check, in file order"
    );
    // A new status cannot arrive without a vector, so the Go client decodes every code too.
    let mut codes: Vec<u16> = vectors
        .iter()
        .filter(|(_, bytes)| bytes[4] & 0x80 != 0)
        .map(|(_, bytes)| u16::from_le_bytes([bytes[8], bytes[9]]))
        .filter(|code| *code != 0)
        .collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(
        codes,
        (1..=15).collect::<Vec<_>>(),
        "every status code has a vector"
    );
}
