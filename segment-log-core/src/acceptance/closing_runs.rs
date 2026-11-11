//! Open and a roll's closing run: complete, cut short by a crash, damaged, or only imitated by the
//! bytes of a record's payload. These scenarios set the default roll size, because they assert
//! which file the next append lands in.
use super::support::{bounded, fixtures::*, raw};
use crate::{codec, storage::sim::Crash, *};
use std::{path::Path, sync::Arc};

const KEYS: usize = 300;

fn unrolled() -> Options {
    Options {
        roll_bytes: Options::default().roll_bytes,
        ..options()
    }
}

fn try_open(sim: &Sim) -> Result<SegmentLog, OpenError> {
    SegmentLog::with_storage(Path::new("/log"), unrolled(), Arc::new(sim.clone()))
}

fn key(n: usize) -> Vec<u8> {
    format!("key-{n}").into_bytes()
}

fn keyed(n: usize) -> NewRecord {
    NewRecord {
        stream: format!("s-{n}").into_bytes(),
        key: Some(key(n)),
        payload: key(n),
        expected: ExpectedRevision::Empty,
    }
}

/// A header, then one group of `KEYS` keyed records on their own streams, so that the key index of
/// its closing run has two pages.
fn keyed_segment(epoch: u64) -> Vec<u8> {
    let records: Vec<_> = (0..KEYS).map(keyed).collect();
    let group = codec::encode_group(
        &codec::LogId::new(raw::LOG_ID),
        &codec::Group {
            epoch,
            base_offset: 0,
            entries: records
                .iter()
                .map(|r| codec::Entry {
                    stream: &r.stream,
                    key: r.key.as_deref(),
                    payload: &r.payload,
                    revision: 0,
                })
                .collect(),
        },
    )
    .unwrap();
    [raw::header(raw::VERSION, 0, epoch), group].concat()
}

/// The position and length of each frame of a closing run that starts at file position `start`.
fn frames_at(run: &[u8], start: usize) -> Vec<(usize, usize)> {
    let mut at = 0;
    let mut out = vec![];
    while at < run.len() {
        let length = u32::from_le_bytes(run[at..at + 4].try_into().unwrap()) as usize + 8;
        out.push((start + at, length));
        at += length;
    }
    out
}

/// A complete roll's closing run under `log_id`, for a segment with one keyed record at position 64
/// and next unused offset `next`, whose key filter sits at file position `filter_at`.
fn closing_run_at(log_id: &codec::LogId, filter_at: usize, next: u64) -> Vec<u8> {
    let mut entries = [codec::KeyEntry {
        hash: log_id.key_hash(b"k"),
        position: 64,
        record: 0,
    }];
    let keys = codec::encode_key_frames(log_id, 1, next, &mut entries).unwrap();
    let footer = codec::encode_footer(
        log_id,
        1,
        next,
        &codec::Footer {
            key_filter_position: std::num::NonZeroU64::new(filter_at as u64),
            streams: vec![codec::StreamTail {
                stream: b"s",
                last_revision: 0,
                last_offset: next - 1,
                last_position: 64,
            }],
        },
    )
    .unwrap();
    let footer_at = filter_at + keys.filter.len() + keys.index.len();
    let locator = codec::encode_locator(log_id, 1, next, footer_at as u64);
    [keys.filter, keys.index, footer, locator].concat()
}

/// Every record of `keyed_segment` is readable, and a retry of one of its keys returns the original
/// position without an append.
fn assert_keyed(log: &SegmentLog) {
    let records = log.read(0, KEYS).unwrap();
    assert_eq!(records.len(), KEYS);
    for (n, record) in records.iter().enumerate() {
        assert_eq!(record.offset, n as u64);
        assert_eq!(record.key, Some(key(n)));
    }
    let watermark = log.watermark();
    for n in [0, KEYS - 1] {
        let answer = log
            .append(Submit {
                records: vec![keyed(n)],
            })
            .unwrap()
            .wait()
            .unwrap();
        assert!(answer.deduplicated);
        assert_eq!(answer.records[0].offset, n as u64);
    }
    assert_eq!(log.watermark(), watermark);
}

fn append_new(log: &SegmentLog) -> u64 {
    log.append(Submit {
        records: vec![keyed(KEYS)],
    })
    .unwrap()
    .wait()
    .unwrap()
    .records[0]
        .offset
}

/// Opens a log whose only segment is `bytes`, and checks that open cut it to `groups`, kept it as
/// the segment that takes the next append, and that a power loss after open changes nothing.
fn assert_cut_to(bytes: Vec<u8>, groups: &[u8], label: &str) {
    let sim = Sim::new(330);
    raw::install_segments(&sim, &[(0, bytes)]);
    let log = open_with(&sim, unrolled());
    assert_eq!(raw::segments(&sim), [(0, groups.to_vec())], "{label}");
    assert_keyed(&log);
    drop(log);
    sim.crash(Crash::PowerLoss, 330);
    let log = open_with(&sim, unrolled());
    assert_keyed(&log);
    assert_eq!(append_new(&log), KEYS as u64, "{label}");
    let segments = raw::segments(&sim);
    assert_eq!(segments.len(), 1, "{label}: the segment stays active");
    assert!(segments[0].1.starts_with(groups), "{label}");
}

/// Given a segment of keyed groups followed by the first one, two or three frames of a roll's
/// closing run, with no locator after them
/// When the log opens
/// Then open cuts the segment back to its groups, because the flush that covered those frames
/// never returned
/// And every record is readable, and a retry of a key returns its original position
/// And a power loss after open gives the same log, and the next append lands in that segment
///
/// Cases: RECOVERY-FIXTURES, ORACLE-4, ORACLE-5
#[test]
fn closing_frames_without_a_locator_are_truncated() {
    bounded(|| {
        let groups = keyed_segment(1);
        let run = raw::roll_run(&groups, 1);
        for (kept, &(end, _)) in frames_at(&run, 0).iter().enumerate().skip(1) {
            assert_cut_to(
                [groups.clone(), run[..end].to_vec()].concat(),
                &groups,
                &format!("{kept} closing frames"),
            );
        }
    });
}

/// Given a segment whose last record's payload ends with a complete roll's closing run under the
/// log's own id, whose positions name the frames inside that payload
/// When the log opens
/// Then the segment stays active, because the forward scan steps over the whole group frame
/// And the record is readable with its payload, and the next append lands after it in that segment
///
/// Cases: RECOVERY-FIXTURES
#[test]
fn a_payload_that_ends_in_a_closing_run_leaves_the_segment_active() {
    bounded(|| {
        let log_id = codec::LogId::new(raw::LOG_ID);
        let prefix = vec![b'p'; 100];
        let payload_at = 64 + 8 + 24 + 16 + b"s".len() + b"k".len();
        let run = closing_run_at(&log_id, payload_at + prefix.len(), 1);
        let payload = [prefix, run].concat();
        let group = codec::encode_group(
            &log_id,
            &codec::Group {
                epoch: 1,
                base_offset: 0,
                entries: vec![codec::Entry {
                    stream: b"s",
                    key: Some(b"k"),
                    payload: &payload,
                    revision: 0,
                }],
            },
        )
        .unwrap();
        let segment = [raw::header(raw::VERSION, 0, 1), group].concat();
        let tail = &segment[segment.len() - codec::LOCATOR_LEN..];
        assert!(
            matches!(
                codec::decode(&log_id, tail).unwrap().body,
                codec::Body::Locator { .. }
            ),
            "the last bytes are a valid locator"
        );

        let sim = Sim::new(331);
        raw::install_segments(&sim, &[(0, segment.clone())]);
        let log = open_with(&sim, unrolled());
        let records = log.read(0, 10).unwrap();
        assert_records(&records, &[stored(b"s", Some(b"k"), &payload, 0, 0)]);
        assert_eq!(append_new(&log), 1);
        let segments = raw::segments(&sim);
        assert_eq!(segments.len(), 1);
        assert!(segments[0].1.starts_with(&segment));
    });
}

/// Given a segment of keyed groups and a complete roll's closing run, where a power loss kept the
/// locator but lost the footer's bytes, or lost the key filter or a page of the key index while
/// the footer and the locator survived
/// When the log opens
/// Then open cuts the closing run and keeps the segment active, and does not fail with `Corrupt`
/// And every record is readable, a retry of a key returns its original position, and a power loss
/// after open gives the same log
///
/// Cases: RECOVERY-FIXTURES, ORACLE-4, ORACLE-5
#[test]
fn a_closing_run_that_lost_a_page_is_truncated() {
    bounded(|| {
        let groups = keyed_segment(1);
        let run = raw::roll_run(&groups, 1);
        let start = groups.len();
        let frames = frames_at(&run, start);
        let [filter, index, footer, _] = frames[..] else {
            panic!("a roll's run has four frames")
        };
        let second_index_page = (index.0 + 8 + 24 + 4096, 4096);
        for (label, (lost, length)) in [
            ("footer", footer),
            ("key filter", filter),
            ("second key index page", second_index_page),
        ] {
            let mut bytes = [groups.clone(), run.clone()].concat();
            bytes[lost..lost + length].fill(0);
            assert_cut_to(bytes, &groups, label);
        }
    });
}

/// Given a segment with a successor whose key filter frame has a bad checksum, or a valid checksum
/// over fences out of order
/// When the log opens
/// Then open fails with `Corrupt` at that key filter, and no file changes
///
/// Cases: RECOVERY-FIXTURES, OPEN-LAYOUT, ORACLE-6
#[test]
fn a_bad_key_filter_in_a_sealed_segment_fails_open() {
    bounded(|| {
        let groups = keyed_segment(1);
        let run = raw::roll_run(&groups, 1);
        let filter_at = groups.len();
        for rechecksum in [false, true] {
            let mut bytes = [groups.clone(), run.clone()].concat();
            let fences = filter_at + 8 + 24;
            if rechecksum {
                let (first, second) = bytes[fences..fences + 16].split_at_mut(8);
                first.swap_with_slice(second);
                let length = frames_at(&run, filter_at)[0].1;
                raw::rechecksum(&raw::LOG_ID, &mut bytes[filter_at..filter_at + length]);
            } else {
                bytes[fences + 16] ^= 1;
            }
            let sim = Sim::new(332);
            raw::install_segments(
                &sim,
                &[
                    (0, bytes),
                    (KEYS as u64, raw::header(raw::VERSION, KEYS as u64, 1)),
                ],
            );
            let before = sim.image();
            let result = try_open(&sim);
            assert!(
                matches!(
                    result,
                    Err(OpenError::Corrupt { segment: 0, position })
                        if position == filter_at as u64
                ),
                "rechecksum {rechecksum}: {:?}",
                result.err()
            );
            assert_eq!(sim.image(), before);
        }
    });
}

/// Given a newest segment that ends in a complete roll's closing run, where an earlier group frame
/// has a bad checksum
/// When the log opens
/// Then open fails with `Corrupt` at that group frame, because the segment is sealed and recovery
/// never truncates one
/// And no file changes
///
/// Cases: RECOVERY-FIXTURES, ORACLE-6
#[test]
fn a_bad_group_before_a_complete_closing_run_fails_open() {
    bounded(|| {
        let first = raw::group(1, 0, &[b"a"]);
        let second = raw::group(1, 1, &[b"b", b"c"]);
        let groups = [raw::header(raw::VERSION, 0, 1), first.clone(), second].concat();
        let sealed = raw::rolled(groups, 1);
        for bad in [64, 64 + first.len()] {
            let mut bytes = sealed.clone();
            bytes[bad + 40] ^= 1;
            let sim = Sim::new(333);
            raw::install_segments(&sim, &[(0, bytes)]);
            let before = sim.image();
            let result = try_open(&sim);
            assert!(
                matches!(
                    result,
                    Err(OpenError::Corrupt { segment: 0, position }) if position == bad as u64
                ),
                "bad group at {bad}: {:?}",
                result.err()
            );
            assert_eq!(sim.image(), before);
        }
    });
}

/// Given a last group whose payload ends with a complete roll's closing run, computed with another
/// log's id or with no log id, and a power loss that kept the run's pages and lost an earlier page
/// of that group
/// When the log opens
/// Then open cuts that group, keeps the group before it, and does not fail with `Corrupt`
/// And a power loss after open gives the same log
/// When the run inside the payload uses the log's own id
/// Then open fails with `Corrupt` at that group and changes no file, as for a sealed segment
///
/// Cases: RECOVERY-FIXTURES, ORACLE-5
#[test]
fn a_torn_group_whose_payload_ends_in_a_foreign_closing_run_is_truncated() {
    bounded(|| {
        let kept = [raw::header(raw::VERSION, 0, 1), raw::group(1, 0, &[b"a"])].concat();
        let torn_at = kept.len();
        let prefix = vec![b'p'; 3 * 4096];
        let filter_at = torn_at + 8 + 24 + 16 + b"s".len() + prefix.len();
        let own = codec::LogId::new(raw::LOG_ID);
        let other = codec::LogId::new([9; 16]);
        for (label, log_id, strip) in [
            ("another log id", &other, false),
            ("no log id", &other, true),
            ("the log's own id", &own, false),
        ] {
            let mut run = closing_run_at(log_id, filter_at, 2);
            if strip {
                for (at, length) in frames_at(&run.clone(), 0) {
                    let frame = &mut run[at..at + length];
                    let checksum = raw::crc(&[&frame[..4], &frame[8..]].concat());
                    frame[4..8].copy_from_slice(&checksum.to_le_bytes());
                }
            }
            let mut torn = raw::group(1, 1, &[&[prefix.clone(), run].concat()]);
            torn[4096..2 * 4096].fill(0);
            let sim = Sim::new(334);
            raw::install_segments(&sim, &[(0, [kept.clone(), torn].concat())]);
            if std::ptr::eq(log_id, &own) {
                let before = sim.image();
                let result = try_open(&sim);
                assert!(
                    matches!(
                        result,
                        Err(OpenError::Corrupt { segment: 0, position })
                            if position == torn_at as u64
                    ),
                    "{label}: {:?}",
                    result.err()
                );
                assert_eq!(sim.image(), before);
                continue;
            }
            let log = open_with(&sim, unrolled());
            assert_eq!(raw::segments(&sim), [(0, kept.clone())], "{label}");
            assert_eq!(log.watermark(), 1);
            drop(log);
            sim.crash(Crash::PowerLoss, 334);
            let log = open_with(&sim, unrolled());
            assert_eq!(log.read(0, 10).unwrap().len(), 1, "{label}");
            assert_eq!(raw::segments(&sim), [(0, kept.clone())], "{label}");
        }
    });
}

/// Given a newest segment that ends in a complete roll's closing run, written under a later writer
/// epoch than its groups, because a crash came after the roll's flush and before the next segment
/// existed
/// When the log opens
/// Then the segment keeps its bytes, and open creates the next segment as the roll would have,
/// named by the next offset, under one more than the highest epoch in any header or frame
/// And every record is readable, and a retry of a key returns its original position
/// And a power loss after open gives the same log, and the next append lands in the new segment
/// When the closing run carries the highest epoch there is
/// Then open fails with `Exhausted` and changes no file
///
/// Cases: RECOVERY-FIXTURES, COUNTER-EXHAUSTION, ORACLE-4, ORACLE-5, ORACLE-6
#[test]
fn a_newest_segment_that_ends_in_a_closing_run_gets_its_successor() {
    bounded(|| {
        let groups = [
            raw::header(raw::VERSION, 0, 1),
            keyed_segment(2)[64..].to_vec(),
        ]
        .concat();
        let rolled = [groups.clone(), raw::roll_run(&groups, 3)].concat();
        let next = KEYS as u64;
        let sim = Sim::new(335);
        raw::install_segments(&sim, &[(0, rolled.clone())]);
        let log = open_with(&sim, unrolled());
        let successor = raw::header(raw::VERSION, next, 4);
        assert_eq!(
            raw::segments(&sim),
            [(0, rolled.clone()), (next, successor.clone())]
        );
        assert_keyed(&log);
        drop(log);
        sim.crash(Crash::PowerLoss, 335);
        let log = open_with(&sim, unrolled());
        assert_eq!(
            raw::segments(&sim),
            [(0, rolled.clone()), (next, successor.clone())]
        );
        assert_keyed(&log);
        assert_eq!(append_new(&log), next);
        let segments = raw::segments(&sim);
        assert_eq!(segments.len(), 2);
        assert!(segments[1].1.starts_with(&successor));

        let sim = Sim::new(335);
        let exhausted = [groups.clone(), raw::roll_run(&groups, u64::MAX)].concat();
        raw::install_segments(&sim, &[(0, exhausted)]);
        let before = sim.image();
        assert!(matches!(try_open(&sim), Err(OpenError::Exhausted)));
        assert_eq!(sim.image(), before);
    });
}
