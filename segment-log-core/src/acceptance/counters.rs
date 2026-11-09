use super::support::{bounded, fixtures::*};
use crate::{
    storage::{Call, Op, Purpose, Storage},
    *,
};
use std::{path::Path, sync::Arc};

fn record(stream: &[u8], expected: ExpectedRevision) -> NewRecord {
    NewRecord {
        stream: stream.to_vec(),
        expected,
        key: None,
        payload: vec![],
    }
}
fn mutations(sim: &Sim) -> usize {
    sim.trace()
        .1
        .iter()
        .filter(|t| {
            matches!(
                t,
                crate::storage::sim::Event::Call(
                    Call {
                        op: Op::Append(_)
                            | Op::Write(..)
                            | Op::Truncate(_)
                            | Op::SyncAll
                            | Op::SyncData
                            | Op::SyncDir,
                        ..
                    },
                    _,
                    _
                )
            )
        })
        .count()
}

/// Given synthetic counters with one offset remaining
/// When a two-record submit is refused and a smaller submit follows
/// Then refusal writes nothing, the last offset commits, and further appends are exhausted
/// Cases: COUNTER-EXHAUSTION
#[test]
fn offset_refusal_preserves_room_for_a_smaller_submit() {
    bounded(|| {
        let sim = Sim::new(41);
        let log = open(&sim);
        log.seed_counters(u64::MAX - 1, &[]);
        let before = mutations(&sim);
        let result = log
            .append(Submit {
                records: vec![
                    record(b"a", ExpectedRevision::Empty),
                    record(b"b", ExpectedRevision::Empty),
                ],
            })
            .unwrap()
            .wait();
        assert!(matches!(result, Err(CommitError::Exhausted)), "{result:?}");
        assert_eq!(mutations(&sim), before);
        assert_eq!(log.watermark(), u64::MAX - 1);
        assert!(log.stream_head(b"a").is_none());
        let result = log
            .append(Submit {
                records: vec![record(b"a", ExpectedRevision::Empty)],
            })
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(result.records[0].offset, u64::MAX - 1);
        assert_eq!(log.watermark(), u64::MAX);
        let before = mutations(&sim);
        assert!(matches!(
            log.append(Submit {
                records: vec![record(b"b", ExpectedRevision::Empty)]
            })
            .unwrap()
            .wait(),
            Err(CommitError::Exhausted)
        ));
        assert_eq!(mutations(&sim), before);
    });
}

/// Given a stream one revision below the limit
/// When a submit would advance it twice
/// Then the whole submit is refused and a single append can still reach MAX
/// And another stream remains writable after exhaustion
/// Cases: COUNTER-EXHAUSTION
#[test]
fn revision_refusal_preserves_the_last_revision_and_other_streams() {
    bounded(|| {
        let sim = Sim::new(42);
        let log = open(&sim);
        log.seed_counters(1, &[(b"s", u64::MAX - 1)]);
        let before = mutations(&sim);
        let result = log
            .append(Submit {
                records: vec![
                    record(b"s", ExpectedRevision::At(u64::MAX - 1)),
                    record(b"s", ExpectedRevision::At(u64::MAX)),
                ],
            })
            .unwrap()
            .wait();
        assert!(matches!(result, Err(CommitError::Exhausted)), "{result:?}");
        assert_eq!(mutations(&sim), before);
        assert_eq!(log.watermark(), 1);
        assert_eq!(log.stream_head(b"s").unwrap().revision, u64::MAX - 1);
        let result = log
            .append(Submit {
                records: vec![record(b"s", ExpectedRevision::At(u64::MAX - 1))],
            })
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(result.records[0].revision, u64::MAX);
        let before = mutations(&sim);
        assert!(matches!(
            log.append(Submit {
                records: vec![record(b"s", ExpectedRevision::At(u64::MAX))]
            })
            .unwrap()
            .wait(),
            Err(CommitError::Exhausted)
        ));
        assert_eq!(mutations(&sim), before);
        assert_eq!(
            log.append(Submit {
                records: vec![record(b"other", ExpectedRevision::Empty)]
            })
            .unwrap()
            .wait()
            .unwrap()
            .records[0]
                .offset,
            2
        );
    });
}

/// Given exhausted streams whose alphabetical order differs from input order
/// When expected-revision and exhaustion checks both fail
/// Then the earliest input failure wins, including a mismatched expectation at MAX
/// Cases: COUNTER-EXHAUSTION
#[test]
fn first_input_failure_wins_over_stream_sort_order_and_offset_exhaustion() {
    bounded(|| {
        for exhausted_first in [true, false] {
            for exhausted_stream in [b"a", b"z"] {
                let wrong_stream = if exhausted_stream == b"a" { b"z" } else { b"a" };
                let sim = Sim::new(43);
                let log = open(&sim);
                log.seed_counters(
                    u64::MAX,
                    &[(exhausted_stream, u64::MAX), (wrong_stream, u64::MAX)],
                );
                let mut records = vec![
                    record(exhausted_stream, ExpectedRevision::At(u64::MAX)),
                    record(wrong_stream, ExpectedRevision::Empty),
                ];
                if !exhausted_first {
                    records.reverse();
                }
                let before = mutations(&sim);
                let result = log.append(Submit { records }).unwrap().wait();
                if exhausted_first {
                    assert!(matches!(result, Err(CommitError::Exhausted)), "{result:?}");
                } else {
                    assert!(
                        matches!(
                            result,
                            Err(CommitError::WrongRevision {
                                index: 0,
                                actual: Some(u64::MAX)
                            })
                        ),
                        "{result:?}"
                    );
                }
                assert_eq!(mutations(&sim), before);
                assert_eq!(log.watermark(), u64::MAX);
            }
        }
    });
}

fn epoch_refusal(header: u64, group: Option<u64>, tail: bool) {
    let sim = Sim::new(44);
    epoch_fixture(&sim, header, group, tail);
    let before = mutations(&sim);
    let image = sim.image();
    let result = SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone()));
    assert!(matches!(result, Err(OpenError::Exhausted)));
    assert_eq!(mutations(&sim), before);
    assert_eq!(sim.image(), image);
    for mode in [
        crate::storage::sim::Crash::Kill,
        crate::storage::sim::Crash::PowerLoss,
        crate::storage::sim::Crash::PowerLossTorn,
    ] {
        let (seed, trace) = sim.trace();
        let sim = Sim::replay(seed, &trace);
        sim.crash(mode, 44);
        let before = mutations(&sim);
        assert!(matches!(
            SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone())),
            Err(OpenError::Exhausted)
        ));
        assert_eq!(mutations(&sim), before);
    }
    sim.call(Call::new(SEG, Purpose::Recovery, Op::Truncate(64)))
        .unwrap();
    sim.call(Call::new(
        SEG,
        Purpose::Recovery,
        Op::Write(
            0,
            crate::codec::encode_header(&crate::codec::Header {
                log_id: [7; 16],
                base_offset: 0,
                epoch: 1,
            })
            .to_vec(),
        ),
    ))
    .unwrap();
    drop(open(&sim));
}

/// Given a legal header with the maximum epoch
/// When open must allocate its successor
/// Then it refuses without changing storage and releases the lock
/// Cases: COUNTER-EXHAUSTION
#[test]
fn maximum_header_epoch_refuses_before_recovery_mutations() {
    bounded(|| epoch_refusal(u64::MAX, None, false));
}

/// Given a valid maximum-epoch group followed by an invalid tail
/// When open scans the kept prefix
/// Then it refuses before truncation or recovery syncs and releases the lock
/// Cases: COUNTER-EXHAUSTION
#[test]
fn maximum_kept_epoch_refuses_before_truncating_invalid_tail() {
    bounded(|| epoch_refusal(1, Some(u64::MAX), true));
}

/// Given a maximum epoch that occurs only in a checksum-invalid frame
/// When recovery scans and repairs the tail
/// Then open succeeds and a new record starts at offset zero
/// Cases: COUNTER-EXHAUSTION
#[test]
fn invalid_frame_epoch_does_not_exhaust_the_kept_prefix() {
    bounded(|| {
        let sim = Sim::new(45);
        epoch_fixture(&sim, 1, Some(u64::MAX), false);
        sim.call(Call::new(SEG, Purpose::Recovery, Op::Write(68, vec![0; 4])))
            .unwrap();
        let log = open(&sim);
        assert_eq!(log.watermark(), 0);
        let result = append(&log, b"s").wait().unwrap();
        assert_eq!(result.records[0].offset, 0);
        assert_eq!(result.records[0].revision, 0);
        sim.crash(crate::storage::sim::Crash::PowerLoss, 45);
        drop(log);
        assert_record(&open(&sim));
    });
}

/// Given a legal header one epoch below MAX
/// When open allocates MAX and commits a record before a crash
/// Then the record commits successfully and the next open refuses exhaustion
/// Cases: COUNTER-EXHAUSTION
#[test]
fn maximum_writer_epoch_can_commit_but_cannot_reopen() {
    bounded(|| {
        for mode in [
            crate::storage::sim::Crash::Kill,
            crate::storage::sim::Crash::PowerLoss,
            crate::storage::sim::Crash::PowerLossTorn,
        ] {
            let sim = Sim::new(46);
            epoch_fixture(&sim, u64::MAX - 1, None, false);
            let log = open(&sim);
            assert_eq!(append(&log, b"s").wait().unwrap().records[0].offset, 0);
            assert_record(&log);
            sim.crash(mode, 46);
            drop(log);
            let before = mutations(&sim);
            assert!(matches!(
                SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone())),
                Err(OpenError::Exhausted)
            ));
            assert_eq!(mutations(&sim), before);
        }
    });
}
