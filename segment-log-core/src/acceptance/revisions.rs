use super::support::{bounded, fixtures::*};
use crate::storage::sim::{Crash, Fault};
use crate::storage::{Call, Op, Purpose};
use crate::*;
use std::path::Path;

/// Given an empty log and one successfully committed stream
/// When a new submit expects a nonexistent revision or Empty on the existing stream
/// Then it is rejected without consuming an offset and a correct At append gets revision 1
/// Cases: REV-EXPECTED, REV-NO-OFFSET
#[test]
fn wrong_revision_does_not_consume_a_position() {
    bounded(|| {
        let sim = Sim::new(7);
        let log = open(&sim);
        let mut wrong = submit(b"s");
        wrong.records[0].expected = ExpectedRevision::At(0);
        assert!(matches!(
            log.append(wrong).unwrap().wait(),
            Err(CommitError::WrongRevision {
                index: 0,
                actual: None
            })
        ));
        append(&log, b"s").wait().unwrap();
        let mut wrong = submit(b"s");
        wrong.records[0].key = None;
        assert!(matches!(
            log.append(wrong).unwrap().wait(),
            Err(CommitError::WrongRevision {
                index: 0,
                actual: Some(0)
            })
        ));
        let mut next = submit(b"s");
        next.records[0].key = None;
        next.records[0].expected = ExpectedRevision::At(0);
        let result = log.append(next).unwrap().wait().unwrap();
        assert_eq!(
            (result.records[0].offset, result.records[0].revision),
            (1, 1)
        );
        assert_eq!(
            sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
            2
        );
    });
}

fn new(stream: &[u8], expected: ExpectedRevision, key: Option<&[u8]>) -> NewRecord {
    NewRecord {
        stream: stream.to_vec(),
        expected,
        key: key.map(<[u8]>::to_vec),
        payload: vec![0, 255, 7],
    }
}

fn stored(stream: &[u8], offset: u64, revision: u64, key: Option<&[u8]>) -> Record {
    Record {
        stream: stream.to_vec(),
        offset,
        revision,
        key: key.map(<[u8]>::to_vec),
        payload: vec![0, 255, 7],
    }
}

fn assert_history(log: &SegmentLog, expected: &[Record]) {
    assert_records(&log.read(0, 100).unwrap(), expected);
    assert_eq!(log.watermark(), expected.len() as u64);
    for record in expected {
        let last = expected
            .iter()
            .rev()
            .find(|r| r.stream == record.stream)
            .unwrap();
        let head = log.stream_head(&record.stream).unwrap();
        assert_eq!((head.offset, head.revision), (last.offset, last.revision));
    }
}

fn assert_positions(answer: Committed, expected: &[(u64, u64)]) {
    assert!(!answer.deduplicated);
    let positions: Vec<_> = answer
        .records
        .iter()
        .map(|p| (p.offset, p.revision))
        .collect();
    assert_eq!(positions, expected);
}

fn assert_wrong(error: &CommitError, index: usize, actual: Option<u64>) {
    assert!(
        matches!(error, CommitError::WrongRevision { index: i, actual: a }
        if *i == index && *a == actual),
        "{error:?}"
    );
}

fn assert_group_calls(sim: &Sim, count: usize) {
    assert_eq!(
        sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
        count
    );
    assert_eq!(
        sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
        count
    );
}

/// Given separate absent streams and separate streams at revision zero
/// When Empty and At(0) are checked against each state
/// Then only Empty creates a stream and only At(0) advances revision zero
/// And each rejection leaves reads and heads unchanged and the next position unused
/// Cases: REV-EXPECTED, REV-NO-OFFSET
#[test]
fn empty_and_revision_zero_are_distinct_preconditions() {
    bounded(|| {
        let sim = Sim::new(301);
        let log = open(&sim);
        let mut expected = Vec::new();
        for stream in [b"empty".as_slice(), b"at"] {
            if stream == b"at" {
                let error = log
                    .append(Submit {
                        records: vec![new(stream, ExpectedRevision::At(0), None)],
                    })
                    .unwrap()
                    .wait()
                    .unwrap_err();
                assert_wrong(&error, 0, None);
                assert_history(&log, &expected);
                assert!(log.stream_head(stream).is_none());
                assert_group_calls(&sim, 1);
            }
            let offset = expected.len() as u64;
            assert_positions(
                log.append(Submit {
                    records: vec![new(stream, ExpectedRevision::Empty, None)],
                })
                .unwrap()
                .wait()
                .unwrap(),
                &[(offset, 0)],
            );
            expected.push(stored(stream, offset, 0, None));
            assert_history(&log, &expected);
        }
        let error = log
            .append(Submit {
                records: vec![new(b"empty", ExpectedRevision::Empty, None)],
            })
            .unwrap()
            .wait()
            .unwrap_err();
        assert_wrong(&error, 0, Some(0));
        assert_history(&log, &expected);
        assert_group_calls(&sim, 2);
        assert_positions(
            log.append(Submit {
                records: vec![new(b"at", ExpectedRevision::At(0), None)],
            })
            .unwrap()
            .wait()
            .unwrap(),
            &[(2, 1)],
        );
        expected.push(stored(b"at", 2, 1, None));
        assert_history(&log, &expected);
    });
}

/// Given absent streams whose alphabetical order differs from request order
/// When one submit interleaves successive records in those streams
/// Then each record sees its earlier stream revision and positions preserve input order
/// Cases: REV-EXPECTED
#[test]
fn interleaved_records_see_successive_revisions_in_input_order() {
    bounded(|| {
        let sim = Sim::new(302);
        let log = open(&sim);
        let records = vec![
            new(b"z", ExpectedRevision::Empty, None),
            new(b"a", ExpectedRevision::Empty, None),
            new(b"z", ExpectedRevision::At(0), None),
            new(b"b", ExpectedRevision::Empty, None),
            new(b"a", ExpectedRevision::At(0), None),
            new(b"z", ExpectedRevision::At(1), None),
        ];
        assert_positions(
            log.append(Submit { records }).unwrap().wait().unwrap(),
            &[(0, 0), (1, 0), (2, 1), (3, 0), (4, 1), (5, 2)],
        );
        assert_history(
            &log,
            &[
                stored(b"z", 0, 0, None),
                stored(b"a", 1, 0, None),
                stored(b"z", 2, 1, None),
                stored(b"b", 3, 0, None),
                stored(b"a", 4, 1, None),
                stored(b"z", 5, 2, None),
            ],
        );
    });
}

/// Given multiple failing preconditions with the alphabetically first stream last
/// When an absent-stream check or a successive same-stream check fails first in input
/// Then that input index and its actual revision win and no prefix is retained
/// And the next valid append gets offset zero and revision zero
/// Cases: REV-EXPECTED, REV-NO-OFFSET
#[test]
fn first_failing_input_wins_regardless_of_stream_sort_order() {
    bounded(|| {
        for successive in [false, true] {
            let sim = Sim::new(303);
            let log = open(&sim);
            let mut records = Vec::new();
            if successive {
                records.push(new(b"z", ExpectedRevision::Empty, None));
            }
            records.push(new(
                b"z",
                if successive {
                    ExpectedRevision::Empty
                } else {
                    ExpectedRevision::At(0)
                },
                None,
            ));
            records.push(new(b"a", ExpectedRevision::At(9), None));
            let error = log.append(Submit { records }).unwrap().wait().unwrap_err();
            assert_wrong(&error, usize::from(successive), successive.then_some(0));
            assert_history(&log, &[]);
            assert!(log.stream_head(b"z").is_none());
            assert!(log.stream_head(b"a").is_none());
            assert_group_calls(&sim, 0);
            assert_positions(
                log.append(Submit {
                    records: vec![new(b"z", ExpectedRevision::Empty, None)],
                })
                .unwrap()
                .wait()
                .unwrap(),
                &[(0, 0)],
            );
            assert_history(&log, &[stored(b"z", 0, 0, None)]);
        }
    });
}

/// Given a durable stream and a submit with a fresh stream, fresh keys and valid advances
/// When a later record has the wrong revision
/// Then the entire submit is rejected without a write or flush, including its earlier changes
/// When each crash mode reopens the log and a corrected new submit uses the rejected fresh keys
/// Then only the durable prefix survived and the new submit consumes the next unused positions
/// Cases: REV-EXPECTED, REV-NO-OFFSET, ORACLE-1, ORACLE-3, ORACLE-10
#[test]
fn later_failure_discards_fresh_streams_keys_and_revision_advances_across_crashes() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            let sim = Sim::new(304);
            let log = open(&sim);
            assert_positions(
                log.append(Submit {
                    records: vec![new(b"s", ExpectedRevision::Empty, None)],
                })
                .unwrap()
                .wait()
                .unwrap(),
                &[(0, 0)],
            );
            let records = vec![
                new(b"fresh", ExpectedRevision::Empty, Some(b"fresh-1")),
                new(b"s", ExpectedRevision::At(0), Some(b"fresh-2")),
                new(b"s", ExpectedRevision::At(0), Some(b"fresh-3")),
            ];
            let error = log.append(Submit { records }).unwrap().wait().unwrap_err();
            assert_wrong(&error, 2, Some(1));
            assert_history(&log, &[stored(b"s", 0, 0, None)]);
            assert!(log.stream_head(b"fresh").is_none());
            assert_group_calls(&sim, 1);
            sim.crash(mode, 11);
            drop(log);
            let log = open(&sim);
            assert_history(&log, &[stored(b"s", 0, 0, None)]);
            assert!(log.stream_head(b"fresh").is_none());
            let records = vec![
                new(b"fresh", ExpectedRevision::Empty, Some(b"fresh-1")),
                new(b"s", ExpectedRevision::At(0), Some(b"fresh-2")),
                new(b"s", ExpectedRevision::At(1), Some(b"fresh-3")),
            ];
            assert_positions(
                log.append(Submit { records }).unwrap().wait().unwrap(),
                &[(1, 0), (2, 1), (3, 2)],
            );
            let mut expected = vec![
                stored(b"s", 0, 0, None),
                stored(b"fresh", 1, 0, Some(b"fresh-1")),
                stored(b"s", 2, 1, Some(b"fresh-2")),
                stored(b"s", 3, 2, Some(b"fresh-3")),
            ];
            assert_history(&log, &expected);
            sim.crash(mode, 12);
            drop(log);
            let log = open(&sim);
            assert_history(&log, &expected);
            assert_positions(
                log.append(Submit {
                    records: vec![new(b"s", ExpectedRevision::At(2), None)],
                })
                .unwrap()
                .wait()
                .unwrap(),
                &[(4, 3)],
            );

            expected.push(stored(b"s", 4, 3, None));
            assert_history(&log, &expected);
        }
    });
}

#[derive(Clone, Copy)]
enum FlushEnd {
    Success,
    Failure,
    Crash(Crash, u64),
}

fn pending_rejection(end: FlushEnd) {
    let sim = Sim::new(305);
    let log = open(&sim);
    let bootstrap_hold = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
    let bootstrap = log
        .append(Submit {
            records: vec![new(b"s", ExpectedRevision::Empty, None)],
        })
        .unwrap();
    bootstrap_hold.wait();
    let original = log
        .append(Submit {
            records: vec![new(b"s", ExpectedRevision::At(0), None)],
        })
        .unwrap();
    let second = log
        .append(Submit {
            records: vec![
                new(b"s", ExpectedRevision::At(1), None),
                new(b"z", ExpectedRevision::Empty, None),
            ],
        })
        .unwrap();
    let rejected = log
        .append(Submit {
            records: vec![
                new(b"fresh", ExpectedRevision::Empty, Some(b"rejected-fresh")),
                new(b"s", ExpectedRevision::At(0), None),
            ],
        })
        .unwrap();
    let following = log
        .append(Submit {
            records: vec![
                new(b"fresh", ExpectedRevision::Empty, Some(b"rejected-fresh")),
                new(b"s", ExpectedRevision::At(2), None),
            ],
        })
        .unwrap();
    let hold = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
    bootstrap_hold.release(None);
    assert_positions(bootstrap.wait().unwrap(), &[(0, 0)]);
    hold.wait();
    // Classification has finished, so wait must return while the group's flush is held.
    let error = rejected.wait().unwrap_err();
    assert_wrong(&error, 1, Some(2));
    assert!(original.probe().is_none());
    assert!(second.probe().is_none());
    assert!(following.probe().is_none());
    assert_history(&log, &[stored(b"s", 0, 0, None)]);
    assert!(log.stream_head(b"z").is_none());
    assert!(log.stream_head(b"fresh").is_none());
    let mut expected = vec![
        stored(b"s", 0, 0, None),
        stored(b"s", 1, 1, None),
        stored(b"s", 2, 2, None),
        stored(b"z", 3, 0, None),
        stored(b"fresh", 4, 0, Some(b"rejected-fresh")),
        stored(b"s", 5, 3, None),
    ];
    match end {
        FlushEnd::Success => hold.release(None),
        FlushEnd::Failure => hold.release(Some(Fault::NoSpace)),
        FlushEnd::Crash(mode, seed) => {
            sim.crash(mode, seed);
            hold.release(None);
        }
    }
    if matches!(end, FlushEnd::Success) {
        assert_positions(original.wait().unwrap(), &[(1, 1)]);
        assert_positions(second.wait().unwrap(), &[(2, 2), (3, 0)]);
        assert_positions(following.wait().unwrap(), &[(4, 0), (5, 3)]);
        assert_history(&log, &expected);
    } else {
        assert!(matches!(original.wait(), Err(CommitError::Indeterminate)));
        assert!(matches!(second.wait(), Err(CommitError::Indeterminate)));
        assert!(matches!(following.wait(), Err(CommitError::Indeterminate)));
        assert_history(&log, &[stored(b"s", 0, 0, None)]);
        assert!(log.stream_head(b"fresh").is_none());
        assert!(log.stream_head(b"z").is_none());
    }
    assert_wrong(&error, 1, Some(2));
    if !matches!(end, FlushEnd::Crash(..)) {
        assert_group_calls(&sim, 2);
    }
    drop(log);
    if matches!(end, FlushEnd::Failure) {
        sim.crash(Crash::Kill, 0);
    }
    let log = open(&sim);
    let kept = log.watermark() as usize;
    assert!(kept == 1 || kept == 6);
    if matches!(
        end,
        FlushEnd::Success | FlushEnd::Failure | FlushEnd::Crash(Crash::Kill, _)
    ) {
        assert_eq!(kept, 6);
    }
    assert_history(&log, &expected[..kept]);
    if kept == 1 {
        assert!(log.stream_head(b"fresh").is_none());
        assert!(log.stream_head(b"z").is_none());
    }
    let revision = if kept == 1 { 0 } else { 3 };
    assert_positions(
        log.append(Submit {
            records: vec![new(b"s", ExpectedRevision::At(revision), None)],
        })
        .unwrap()
        .wait()
        .unwrap(),
        &[(kept as u64, revision + 1)],
    );
    expected.truncate(kept);
    expected.push(stored(b"s", kept as u64, revision + 1, None));
    assert_history(&log, &expected);
}

/// Given all next-group requests queued behind a held bootstrap flush
/// When earlier valid submits advance revisions and a later submit fails classification
/// Then WrongRevision reports pending revision two before flush, while the public head stays zero
/// And later valid work sees no rejected changes and commits at the next unused positions
/// Cases: REV-EXPECTED, REV-NO-OFFSET, ANSWER-TIMING, READ-BOUNDED, ORACLE-3, ORACLE-10
#[test]
fn pending_revisions_are_checked_before_flush_and_publish_only_after_success() {
    bounded(|| pending_rejection(FlushEnd::Success));
}

/// Given a finalized WrongRevision that observed earlier buffered records
/// When the held group flush later fails
/// Then that rejection stays fixed, unfinished writers get Indeterminate and durable heads stay old
/// And recovery keeps complete original submits without including rejected records or reusing offsets
/// Cases: REV-EXPECTED, REV-NO-OFFSET, ANSWER-TIMING, ORACLE-2, ORACLE-3, ORACLE-10
#[test]
fn finalized_revision_rejection_is_unchanged_by_a_later_flush_failure() {
    bounded(|| pending_rejection(FlushEnd::Failure));
}

/// Given a finalized WrongRevision and a group still held at its flush
/// When Kill, PowerLoss or PowerLossTorn interrupts the writer
/// Then the rejection remains fixed and recovery keeps only a complete prefix of original submits
/// And the next append never reuses an earlier durable offset
/// Cases: REV-EXPECTED, REV-NO-OFFSET, ANSWER-TIMING, ORACLE-1, ORACLE-2, ORACLE-3, ORACLE-10
#[test]
fn finalized_revision_rejection_is_unchanged_by_each_crash_mode() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            for seed in 0..8 {
                pending_rejection(FlushEnd::Crash(mode, seed));
            }
        }
    });
}

fn numbered(prefix: u8, number: usize) -> Vec<u8> {
    format!("{}{number:03}", prefix as char).into_bytes()
}

fn assert_heads_follow_history(log: &SegmentLog, streams: &[Vec<u8>]) -> Vec<Record> {
    let history = log.read(0, 1000).unwrap();
    assert_eq!(history.len() as u64, log.watermark());
    for (offset, record) in history.iter().enumerate() {
        assert_eq!(record.offset, offset as u64);
    }
    for stream in streams {
        let last = history.iter().rev().find(|record| record.stream == *stream);
        let head = log.stream_head(stream);
        assert_eq!(
            head.map(|head| (head.offset, head.revision)),
            last.map(|record| (record.offset, record.revision)),
            "stream {stream:?}"
        );
    }
    history
}

/// Given 300 durable streams, half of them at revision one, and requests queued behind a held flush
/// When one group mixes untouched durable streams, streams that its earlier submits advanced
/// and new streams, with a stale expectation against each kind
/// Then each check sees the group's earlier revisions over the durable head
/// And each stale submit is rejected with that revision before the flush, without changing a head
/// When the flush succeeds
/// Then the valid submits get the next positions and only then do the public heads move
/// When each storage boundary of that group is replayed through every crash mode
/// Then recovery exposes the group whole or not at all, and every head matches its last record
/// Cases: REV-EXPECTED, REV-NO-OFFSET, ANSWER-TIMING, ORACLE-2, ORACLE-3, ORACLE-10
#[test]
fn a_group_checks_revisions_against_its_own_changes_over_many_durable_streams() {
    bounded(|| {
        let sim = Sim::new(331);
        let log = open(&sim);
        let durable: Vec<_> = (0..300).map(|n| numbered(b'd', n)).collect();
        let seeded = |records: Vec<NewRecord>| {
            log.append(Submit { records }).unwrap().wait().unwrap();
        };
        seeded(
            durable
                .iter()
                .map(|stream| new(stream, ExpectedRevision::Empty, None))
                .collect(),
        );
        seeded(
            durable
                .iter()
                .step_by(2)
                .map(|stream| new(stream, ExpectedRevision::At(0), None))
                .collect(),
        );
        let start = sim.trace().1.len();
        let bootstrap_hold = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
        let bootstrap = log
            .append(Submit {
                records: vec![new(b"boot", ExpectedRevision::Empty, None)],
            })
            .unwrap();
        bootstrap_hold.wait();
        let queue = |records: Vec<NewRecord>| log.append(Submit { records }).unwrap();
        let (d10, d11, d13, d299) = (&durable[10], &durable[11], &durable[13], &durable[299]);
        let advances = queue(vec![
            new(d10, ExpectedRevision::At(1), None),
            new(b"n0", ExpectedRevision::Empty, None),
        ]);
        let follows = queue(vec![
            new(d10, ExpectedRevision::At(2), None),
            new(d11, ExpectedRevision::At(0), None),
        ]);
        let stale_advanced = queue(vec![new(d10, ExpectedRevision::At(1), None)]);
        let stale_untouched = queue(vec![new(d13, ExpectedRevision::Empty, None)]);
        let stale_new = queue(vec![new(b"n0", ExpectedRevision::Empty, None)]);
        let absent = queue(vec![new(b"n1", ExpectedRevision::At(0), None)]);
        let last = queue(vec![
            new(d299, ExpectedRevision::At(0), None),
            new(b"n0", ExpectedRevision::At(0), None),
            new(d10, ExpectedRevision::At(3), None),
        ]);
        let hold = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
        bootstrap_hold.release(None);
        assert_positions(bootstrap.wait().unwrap(), &[(450, 0)]);
        hold.wait();
        assert_wrong(&stale_advanced.wait().unwrap_err(), 0, Some(3));
        assert_wrong(&stale_untouched.wait().unwrap_err(), 0, Some(0));
        assert_wrong(&stale_new.wait().unwrap_err(), 0, Some(0));
        assert_wrong(&absent.wait().unwrap_err(), 0, None);
        assert!(advances.probe().is_none());
        assert!(follows.probe().is_none());
        assert!(last.probe().is_none());
        let mut streams = durable.clone();
        streams.extend([b"boot".to_vec(), b"n0".to_vec(), b"n1".to_vec()]);
        assert_eq!(assert_heads_follow_history(&log, &streams).len(), 451);
        hold.release(None);
        assert_positions(advances.wait().unwrap(), &[(451, 2), (452, 0)]);
        assert_positions(follows.wait().unwrap(), &[(453, 3), (454, 1)]);
        assert_positions(last.wait().unwrap(), &[(455, 1), (456, 1), (457, 4)]);
        let history = assert_heads_follow_history(&log, &streams);
        assert_eq!(history.len(), 458);
        assert!(log.stream_head(b"n1").is_none());
        let head = log.stream_head(d13).unwrap();
        assert_eq!((head.offset, head.revision), (13, 0));
        assert_group_calls(&sim, 4);
        let (seed, trace) = sim.trace();
        drop(log);
        for end in start..=trace.len() {
            for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
                let replay = Sim::replay(seed, &trace[..end]);
                replay.crash(mode, end as u64);
                let recovered = open(&replay);
                let kept = assert_heads_follow_history(&recovered, &streams);
                assert!(
                    [450, 451, 458].contains(&kept.len()),
                    "end {end} kept {}",
                    kept.len()
                );
                if end == trace.len() {
                    assert_eq!(kept.len(), 458);
                }
                assert_records(&kept, &history[..kept.len()]);
                let next = recovered
                    .stream_head(d10)
                    .map(|head| head.revision)
                    .unwrap();
                assert_eq!(next, if kept.len() == 458 { 4 } else { 1 });
                assert_positions(
                    recovered
                        .append(Submit {
                            records: vec![new(d10, ExpectedRevision::At(next), None)],
                        })
                        .unwrap()
                        .wait()
                        .unwrap(),
                    &[(kept.len() as u64, next + 1)],
                );
            }
        }
    });
}
