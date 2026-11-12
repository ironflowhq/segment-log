use super::support::{
    bounded,
    fixtures::*,
    oracle::{self, Answer},
    raw,
};
use crate::storage::{
    Call, Op, Purpose,
    sim::{Event, Fault},
};
use crate::*;
use std::{path::Path, sync::Arc};

const ROLL_BYTES: u64 = 300;
const UNKEYED: std::ops::Range<u64> = 8..12;

fn rolling() -> Options {
    Options {
        roll_bytes: ROLL_BYTES,
        ..options()
    }
}

fn open_rolling(sim: &Sim) -> SegmentLog {
    open_with(sim, rolling())
}

fn try_open(sim: &Sim) -> Result<SegmentLog, OpenError> {
    SegmentLog::with_storage(Path::new("/log"), rolling(), Arc::new(sim.clone()))
}

fn once(round: u64) -> NewRecord {
    NewRecord {
        stream: format!("once-{round}").into_bytes(),
        key: (!UNKEYED.contains(&round)).then(|| format!("k{round}").into_bytes()),
        payload: b"only".to_vec(),
        expected: ExpectedRevision::Empty,
    }
}

fn fill(log: &SegmentLog, rounds: std::ops::Range<u64>) -> Vec<Record> {
    let mut records = vec![];
    for round in rounds {
        let submit = Submit {
            records: vec![
                once(round),
                NewRecord {
                    stream: b"live".to_vec(),
                    key: None,
                    payload: round.to_le_bytes().to_vec(),
                    expected: round
                        .checked_sub(1)
                        .map_or(ExpectedRevision::Empty, ExpectedRevision::At),
                },
            ],
        };
        let sent: Vec<_> = submit
            .records
            .iter()
            .map(|r| (r.stream.clone(), r.key.clone(), r.payload.clone()))
            .collect();
        let answer = log.append(submit).unwrap().wait().unwrap();
        for ((stream, key, payload), position) in sent.into_iter().zip(answer.records) {
            records.push(Record {
                offset: position.offset,
                revision: position.revision,
                stream,
                key,
                payload,
            });
        }
    }
    records
}

type Calls = Vec<(Purpose, Op)>;

fn open_counted(sim: &Sim) -> (SegmentLog, Vec<(u64, Calls)>) {
    let before = raw::segments(sim);
    let start = sim.trace().1.len();
    let log = try_open(sim).unwrap();
    let trace = sim.trace().1;
    let calls = before[..before.len() - 1]
        .iter()
        .map(|(base, _)| {
            let path = raw::segment(*base);
            let calls = trace[start..]
                .iter()
                .filter_map(|event| match event {
                    Event::Call(call, _, _) if call.path == Path::new(&path) => {
                        Some((call.purpose, call.op.clone()))
                    }
                    _ => None,
                })
                .collect();
            (*base, calls)
        })
        .collect();
    oracle::sealed_unchanged(&before, &raw::segments(sim));
    (log, calls)
}

struct Closing {
    filter_at: u64,
    filter_len: usize,
    footer_at: u64,
    footer_len: usize,
    locator_at: u64,
}

fn closing(segment: &[u8]) -> Closing {
    let frames = raw::frames(segment);
    let [filter, index, footer, locator] = frames[frames.len() - 4..] else {
        unreachable!()
    };
    let locator_at = (segment.len() - locator.len()) as u64;
    let footer_at = locator_at - footer.len() as u64;
    let filter_at = footer_at - (index.len() + filter.len()) as u64;
    Closing {
        filter_at,
        filter_len: filter.len(),
        footer_at,
        footer_len: footer.len(),
        locator_at,
    }
}

fn summary_reads(segment: &[u8]) -> Calls {
    let at = closing(segment);
    vec![
        (Purpose::Open, Op::Read(0, 64)),
        (Purpose::Summary, Op::Open),
        (Purpose::Summary, Op::Read(at.locator_at, 40)),
        (Purpose::Summary, Op::Read(at.footer_at, at.footer_len)),
        (Purpose::KeyFilter, Op::Read(at.filter_at, 8)),
        (Purpose::KeyFilter, Op::Read(at.filter_at, at.filter_len)),
    ]
}

/// Given a log with several sealed segments, some with keyed records and one with none
/// When the log reopens
/// Then from each segment before the newest, open reads only the header, the locator, the footer
/// and the key filter frame, at their exact positions and lengths, and so no group frame
/// And the bytes it reads from key filters are each filter frame and its envelope
///
/// Cases: ORACLE-13, OPEN-LAYOUT, ORACLE-6
#[test]
fn open_reads_only_the_summary_of_each_sealed_segment() {
    bounded(|| {
        let sim = Sim::new(400);
        fill(&open_rolling(&sim), 0..24);
        let segments = raw::segments(&sim);
        let sealed = &segments[..segments.len() - 1];
        assert!(sealed.len() >= 5, "{} sealed segments", sealed.len());
        let filters: Vec<usize> = sealed.iter().map(|(_, b)| closing(b).filter_len).collect();
        assert!(
            filters.contains(&32) && filters.iter().any(|&len| len > 32),
            "sealed segments with and without keyed records: {filters:?}"
        );

        let (log, counted) = open_counted(&sim);
        let mut filter_bytes = 0;
        for ((base, bytes), (_, calls)) in sealed.iter().zip(counted) {
            assert_eq!(calls, summary_reads(bytes), "segment {base}");
            filter_bytes += calls
                .iter()
                .filter_map(|(purpose, op)| match (purpose, op) {
                    (Purpose::KeyFilter, Op::Read(_, len)) => Some(len),
                    _ => None,
                })
                .sum::<usize>();
        }
        assert_eq!(filter_bytes, filters.iter().map(|len| 8 + len).sum());
        println!(
            "ORACLE-13: open read {filter_bytes} bytes from {} key filters",
            sealed.len()
        );
        assert_eq!(log.watermark(), 48);
    });
}

/// Given a log with several sealed segments, where an early segment's key filter reads back
/// damaged and the last sealed segment's key filter read fails, reads back damaged or panics
/// When the log opens, many times
/// Then every open fails with `Corrupt` at the early segment's key filter, the answer of a load
/// that reads one segment after another
/// And no file changes
///
/// Cases: ORACLE-13, OPEN-LAYOUT, ORACLE-6
#[test]
fn open_reports_the_lowest_segment_with_a_bad_key_filter() {
    bounded(|| {
        let sim = Sim::new(402);
        fill(&open_rolling(&sim), 0..24);
        let segments = raw::segments(&sim);
        let sealed = &segments[..segments.len() - 1];
        let filter_read = |(base, bytes): &(u64, Vec<u8>)| {
            let at = closing(bytes);
            let call = Call::new(
                raw::segment(*base),
                Purpose::KeyFilter,
                Op::Read(at.filter_at, at.filter_len),
            );
            (call, at.filter_at)
        };
        let (low, low_at) = filter_read(&sealed[1]);
        let (high, _) = filter_read(&sealed[sealed.len() - 1]);
        let before = sim.image();
        let later = [
            Fault::error(std::io::ErrorKind::Other).unwrap(),
            Fault::Corrupt(9, 1),
            Fault::Panic,
        ];
        for fault in later {
            for _ in 0..16 {
                sim.inject(low.clone(), Fault::Corrupt(9, 1));
                sim.inject(high.clone(), fault.clone());
                let result = try_open(&sim);
                assert!(
                    matches!(
                        result,
                        Err(OpenError::Corrupt { segment, position })
                            if segment == sealed[1].0 && position == low_at
                    ),
                    "{fault:?}: {:?}",
                    result.err()
                );
            }
        }
        assert_eq!(sim.image(), before);
    });
}

/// Given a log that rolls every few groups, where each round's first stream appears only in the
/// segment that holds it and one stream appends throughout, with keyed and unkeyed records
/// When the log reopens
/// Then the reopen reads no group frame of a segment before the newest
/// And it gives the same watermark, the same head for every stream, and the same `read` results
/// from every offset across segment boundaries as before
/// And a retry and a conflicting reuse of a key in the oldest segment get the same answers
/// before and after
/// And the first read of a sealed segment builds its frame table, which later reads reuse
/// And no segment file is deleted or changed
///
/// Cases: HISTORY, ORACLE-12, ORACLE-13, ORACLE-6, KEY-LIFETIME, KEY-CONFLICT
#[test]
fn a_reopen_after_many_rolls_keeps_every_answer_and_reads_old_segments_lazily() {
    bounded(|| {
        let sim = Sim::new(401);
        let log = open_rolling(&sim);
        let expected = fill(&log, 0..24);
        let streams: std::collections::BTreeSet<Vec<u8>> =
            expected.iter().map(|r| r.stream.clone()).collect();
        let observe = |log: &SegmentLog| {
            let heads: Vec<_> = streams
                .iter()
                .map(|s| log.stream_head(s).map(|h| (h.offset, h.revision)))
                .collect();
            let reads: Vec<_> = (0..=log.watermark())
                .map(|from| format!("{:?}", log.read(from, 3).unwrap()))
                .collect();
            (log.watermark(), heads, reads)
        };
        let retry = |log: &SegmentLog| {
            let mut conflict = once(0);
            conflict.payload.push(1);
            [once(0), conflict].map(|record| {
                Answer::commit(
                    log.append(Submit {
                        records: vec![record],
                    })
                    .unwrap()
                    .wait(),
                )
            })
        };
        let before = (observe(&log), retry(&log));
        assert_eq!(
            before.1,
            [
                Answer::Ack(true, vec![(0, 0)]),
                Answer::Refused("KeyConflict { index: 0 }".into())
            ]
        );
        drop(log);
        let segments = raw::segments(&sim);
        let sealed = &segments[..segments.len() - 1];
        assert!(sealed.len() >= 5, "{} sealed segments", sealed.len());

        let (log, counted) = open_counted(&sim);
        for ((base, bytes), (_, calls)) in sealed.iter().zip(counted) {
            assert_eq!(calls, summary_reads(bytes), "segment {base}");
        }
        assert_eq!(retry(&log), before.1);
        let lookups = |sim: &Sim| {
            sim.count(
                Path::new(&raw::segment(0)),
                Purpose::Lookup,
                &Op::Read(0, 0),
            )
        };
        let cold = lookups(&sim);
        oracle::history(&log, &expected);
        let first = lookups(&sim) - cold;
        oracle::history(&log, &expected);
        let second = lookups(&sim) - cold - first;
        assert!(
            second < first,
            "the second read of the oldest segment made {second} reads, the first {first}"
        );
        assert_eq!(observe(&log), before.0);
        drop(log);
        assert_eq!(raw::segments(&sim)[..sealed.len()], *sealed);
        assert_eq!(raw::segments(&sim).len(), segments.len());
    });
}

/// Given a reopened log whose oldest segment has no frame table yet, and whose group frames the
/// table walk reads in one call
/// When the first read of that segment meets an I/O error, or bytes that are not a group frame's
/// envelope, during that walk
/// Then that read returns `ReadError::Io`, or `ReadError::Corrupt` naming the segment and the
/// frame's position
/// And the next read walks the segment again and returns every record
///
/// Cases: READ-BOUNDED, ORACLE-12
#[test]
fn a_failed_walk_of_a_sealed_segment_is_an_error_that_the_next_read_retries() {
    bounded(|| {
        let sim = Sim::new(402);
        let expected = fill(&open_rolling(&sim), 0..12);
        let oldest = raw::segments(&sim).remove(0).1;
        let walk = Call::new(
            raw::segment(0),
            Purpose::Lookup,
            Op::Read(64, (closing(&oldest).filter_at - 64) as usize),
        );
        for (fault, corrupt) in [
            (Fault::error(std::io::ErrorKind::Other).unwrap(), false),
            (Fault::Eof, true),
            (Fault::Corrupt(3, 0xff), true),
            (Fault::Corrupt(8, 0xff), true),
            (Fault::Corrupt(20, 1), true),
        ] {
            let log = open_rolling(&sim);
            sim.inject(walk.clone(), fault.clone());
            let result = log.read(0, usize::MAX);
            if corrupt {
                assert!(
                    matches!(
                        result,
                        Err(ReadError::Corrupt {
                            segment: 0,
                            position: 64
                        })
                    ),
                    "{fault:?}: {result:?}"
                );
            } else {
                assert!(matches!(result, Err(ReadError::Io(_))), "{result:?}");
            }
            assert_records(&log.read(0, usize::MAX).unwrap(), &expected);
        }
    });
}

/// Given a sealed segment of about 100 KiB, where a group frame's envelope and body header cross
/// the end of the walk's first read
/// When the log reopens and reads every record
/// Then every record comes back as it was appended
/// And the walk of that segment takes a few storage calls, not one for each frame
///
/// Cases: READ-BOUNDED, HISTORY, ORACLE-12
#[test]
fn a_sealed_segment_larger_than_one_walk_read_is_read_in_full() {
    const FRAME: usize = 1040;
    bounded(|| {
        let sim = Sim::new(405);
        let options = || Options {
            roll_bytes: 100_000,
            ..options()
        };
        let log = open_with(&sim, options());
        let mut expected = vec![];
        for round in 0..120u64 {
            let payload = vec![round as u8; FRAME - 49];
            let submit = Submit {
                records: vec![NewRecord {
                    stream: b"s".to_vec(),
                    key: None,
                    payload: payload.clone(),
                    expected: round
                        .checked_sub(1)
                        .map_or(ExpectedRevision::Empty, ExpectedRevision::At),
                }],
            };
            log.append(submit).unwrap().wait().unwrap();
            expected.push(stored(b"s", None, &payload, round, round));
        }
        drop(log);
        let segments = raw::segments(&sim);
        assert_eq!(segments.len(), 2);
        let groups = closing(&segments[0].1).filter_at;
        let frames = raw::frames(&segments[0].1).len() - 4;
        assert!(groups > 64 * 1024, "{groups} bytes of groups");
        let straddler = 64 + 63 * FRAME as u64;
        assert!(straddler < 64 + 64 * 1024 && straddler + 32 > 64 + 64 * 1024);

        let log = open_with(&sim, options());
        let lookups = || {
            sim.count(
                Path::new(&raw::segment(0)),
                Purpose::Lookup,
                &Op::Read(0, 0),
            )
        };
        let cold = lookups();
        assert_records(&log.read(0, usize::MAX).unwrap(), &expected);
        let walk = lookups() - cold - frames;
        assert!(
            walk <= groups.div_ceil(64 * 1024) as usize,
            "{walk} walk reads"
        );
    });
}

/// Given a log with more sealed segments than the frame tables a log keeps, rolled in this process
/// or found by a reopen
/// When reads need each sealed segment in turn
/// Then every record comes back as it was appended
/// And a later read of the segment whose table was used least recently walks that segment again
/// And a read of a segment whose table is still kept reads only the frame it needs
///
/// Cases: READ-BOUNDED, ORACLE-12
#[test]
fn reads_keep_a_bounded_number_of_frame_tables() {
    bounded(|| {
        let sim = Sim::new(406);
        let options = || Options {
            roll_bytes: 65,
            ..options()
        };
        let sealed = crate::engine::FRAME_TABLES as u64 + 3;
        let log = open_with(&sim, options());
        let mut expected = vec![];
        for offset in 0..=sealed {
            let payload = offset.to_le_bytes();
            let submit = Submit {
                records: vec![NewRecord {
                    stream: b"s".to_vec(),
                    key: None,
                    payload: payload.to_vec(),
                    expected: offset
                        .checked_sub(1)
                        .map_or(ExpectedRevision::Empty, ExpectedRevision::At),
                }],
            };
            log.append(submit).unwrap().wait().unwrap();
            expected.push(stored(b"s", None, &payload, offset, offset));
        }
        assert_eq!(raw::segments(&sim).len() as u64, sealed + 1);
        let reads = |log: &SegmentLog, base: u64| {
            let lookups = || {
                sim.count(
                    Path::new(&raw::segment(base)),
                    Purpose::Lookup,
                    &Op::Read(0, 0),
                )
            };
            let before = lookups();
            let at = base as usize;
            assert_records(&log.read(base, 1).unwrap(), &expected[at..=at]);
            lookups() - before
        };
        let check = |log: &SegmentLog| {
            assert_records(&log.read(0, usize::MAX).unwrap(), &expected);
            assert!(
                reads(log, 0) > 1,
                "the least recently used table is walked again"
            );
            assert_eq!(reads(log, sealed - 1), 1, "a kept table is not walked");
        };
        assert!(
            reads(&log, 0) > 1,
            "the first rolled segment's table is walked again"
        );
        assert_eq!(reads(&log, sealed - 1), 1, "the last roll keeps its table");
        check(&log);
        drop(log);
        check(&open_with(&sim, options()));
    });
}

fn patched(mut segment: Vec<u8>, back: usize, at: usize, bytes: &[u8]) -> Vec<u8> {
    let frames: Vec<usize> = raw::frames(&segment).iter().map(|f| f.len()).collect();
    let end = segment.len() - frames[frames.len() - back + 1..].iter().sum::<usize>();
    let start = end - frames[frames.len() - back];
    segment[start + at..start + at + bytes.len()].copy_from_slice(bytes);
    raw::rechecksum(&raw::LOG_ID, &mut segment[start..end]);
    segment
}

const LOCATOR: usize = 1;
const FOOTER: usize = 2;
/// Frame offsets of the fields that a summary check reads, from format.md: the body header's
/// `epoch` and `base_offset`, the next unused offset, and the footer's `key_filter_position` or
/// the locator's `footer_position`, which follow the body header.
const EPOCH_AT: usize = crate::codec::ENVELOPE_LEN + 4;
const NEXT_AT: usize = crate::codec::ENVELOPE_LEN + 12;
const POSITION_AT: usize = crate::codec::ENVELOPE_LEN + crate::codec::BODY_HEADER_LEN;

fn sealed(base: u64, epoch: u64, payloads: &[&[u8]]) -> Vec<u8> {
    raw::rolled(
        [
            raw::header(raw::VERSION, base, epoch),
            raw::group(epoch, base, payloads),
        ]
        .concat(),
        epoch,
    )
}

/// Given sealed segments whose footer or locator names a next unused offset that is not the
/// next segment's base, or a log with a middle segment missing
/// When the log opens
/// Then open fails with `Gap` naming the segment that does not start where the summary says
/// And no file changes
///
/// Cases: OPEN-LAYOUT, RECOVERY-FIXTURES
#[test]
fn a_summary_that_disagrees_with_the_next_base_is_a_gap() {
    bounded(|| {
        let first = sealed(0, 1, &[b"a", b"b"]);
        let newest = raw::header(raw::VERSION, 3, 1);
        for (segments, gap) in [
            (
                vec![
                    (
                        0,
                        patched(first.clone(), FOOTER, NEXT_AT, &3u64.to_le_bytes()),
                    ),
                    (2, sealed(2, 1, &[b"c"])),
                    (3, newest.clone()),
                ],
                2,
            ),
            (
                vec![
                    (
                        0,
                        patched(first.clone(), LOCATOR, NEXT_AT, &3u64.to_le_bytes()),
                    ),
                    (2, sealed(2, 1, &[b"c"])),
                    (3, newest.clone()),
                ],
                2,
            ),
            (
                vec![
                    (0, first.clone()),
                    (3, sealed(3, 1, &[b"d"])),
                    (4, raw::header(raw::VERSION, 4, 1)),
                ],
                3,
            ),
        ] {
            let sim = Sim::new(403);
            raw::install_segments(&sim, &segments);
            let before = sim.image();
            let result = try_open(&sim);
            assert!(
                matches!(result, Err(OpenError::Gap { segment }) if segment == gap),
                "expected a gap at {gap}: {:?}",
                result.err()
            );
            assert_eq!(sim.image(), before);
        }
    });
}

/// Given a segment with a successor whose locator or footer is not valid on its own bytes: a bad
/// checksum, a lost locator, a footer position or key filter position that names no such frame,
/// a footer of a seal's run, or an epoch below the header's or below the footer's
/// When the log opens
/// Then open fails with `Corrupt` naming that segment and the position of the bad frame
/// And no file changes
///
/// Cases: OPEN-LAYOUT, RECOVERY-FIXTURES, ORACLE-6
#[test]
fn a_bad_summary_in_a_segment_with_a_successor_fails_open_and_changes_nothing() {
    bounded(|| {
        let first = sealed(0, 2, &[b"a", b"b"]);
        let at = closing(&first);
        let mut bad_locator = first.clone();
        bad_locator[at.locator_at as usize + 30] ^= 1;
        let mut bad_footer = first.clone();
        bad_footer[at.footer_at as usize + 30] ^= 1;
        for (damaged, position) in [
            (bad_locator, at.locator_at),
            (bad_footer, at.footer_at),
            (first[..first.len() - 40].to_vec(), at.locator_at - 40),
            (
                patched(
                    first.clone(),
                    LOCATOR,
                    POSITION_AT,
                    &at.filter_at.to_le_bytes(),
                ),
                at.filter_at,
            ),
            (
                patched(first.clone(), LOCATOR, POSITION_AT, &1u64.to_le_bytes()),
                at.locator_at,
            ),
            (
                patched(first.clone(), FOOTER, POSITION_AT, &[0; 8]),
                at.footer_at,
            ),
            (
                patched(
                    first.clone(),
                    FOOTER,
                    POSITION_AT,
                    &(at.filter_at + 8).to_le_bytes(),
                ),
                at.filter_at + 8,
            ),
            (
                patched(first.clone(), FOOTER, EPOCH_AT, &1u64.to_le_bytes()),
                at.footer_at,
            ),
            (
                patched(first.clone(), LOCATOR, EPOCH_AT, &1u64.to_le_bytes()),
                at.locator_at,
            ),
        ] {
            let sim = Sim::new(404);
            raw::install_segments(
                &sim,
                &[
                    (0, damaged),
                    (2, sealed(2, 2, &[b"c"])),
                    (3, raw::header(raw::VERSION, 3, 2)),
                ],
            );
            let before = sim.image();
            let result = try_open(&sim);
            assert!(
                matches!(
                    result,
                    Err(OpenError::Corrupt { segment: 0, position: p }) if p == position
                ),
                "expected corruption at {position}: {:?}",
                result.err()
            );
            assert_eq!(sim.image(), before);
        }
    });
}
