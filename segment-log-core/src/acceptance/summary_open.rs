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

/// A roll size that a segment passes after a few of `fill`'s groups.
const ROLL_BYTES: u64 = 300;
/// The rounds of `fill` whose records carry no key, so that some sealed segment holds none.
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

/// Each round appends one record to a stream that never appears again, and one to `live`.
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

/// Opens the log as `open_rolling` does, and returns the storage calls that open made on each
/// segment file before the newest.
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

/// Where the closing frames of a segment that a roll sealed are.
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

/// The calls that open makes on a sealed segment that is not the newest.
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

/// Rewrites the bytes at `at` inside the frame that ends `back` frames before the end of
/// `segment`, and makes that frame's checksum right again.
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
/// Where a frame's body header keeps the epoch and the next unused offset.
const EPOCH: usize = 12;
const NEXT: usize = 20;
/// Where a footer keeps its key filter position, and a locator its footer position.
const POSITION: usize = 32;

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
                    (0, patched(first.clone(), FOOTER, NEXT, &3u64.to_le_bytes())),
                    (2, sealed(2, 1, &[b"c"])),
                    (3, newest.clone()),
                ],
                2,
            ),
            (
                vec![
                    (
                        0,
                        patched(first.clone(), LOCATOR, NEXT, &3u64.to_le_bytes()),
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
                    POSITION,
                    &at.filter_at.to_le_bytes(),
                ),
                at.filter_at,
            ),
            (
                patched(first.clone(), LOCATOR, POSITION, &1u64.to_le_bytes()),
                at.locator_at,
            ),
            (
                patched(first.clone(), FOOTER, POSITION, &[0; 8]),
                at.footer_at,
            ),
            (
                patched(
                    first.clone(),
                    FOOTER,
                    POSITION,
                    &(at.filter_at + 8).to_le_bytes(),
                ),
                at.filter_at + 8,
            ),
            (
                patched(first.clone(), FOOTER, EPOCH, &1u64.to_le_bytes()),
                at.footer_at,
            ),
            (
                patched(first.clone(), LOCATOR, EPOCH, &1u64.to_le_bytes()),
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
