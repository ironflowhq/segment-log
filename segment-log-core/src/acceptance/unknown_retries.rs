use super::support::{bounded, fixtures::*};
use crate::storage::sim::Crash;
use crate::storage::{Call, Op, Purpose};
use crate::*;
use std::path::Path;

fn record(stream: &[u8], key: Option<&[u8]>, payload: &[u8]) -> NewRecord {
    NewRecord {
        stream: stream.to_vec(),
        key: key.map(<[u8]>::to_vec),
        payload: payload.to_vec(),
        expected: ExpectedRevision::Empty,
    }
}
fn send(log: &SegmentLog, records: Vec<NewRecord>) -> Pending {
    log.append(Submit { records }).unwrap()
}
fn no_new_group(sim: &Sim, count: usize) {
    assert_eq!(
        sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
        count
    );
    assert_eq!(
        sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
        count
    );
}

/// Given an existing key and an arbitrary fully keyed request with a fresh key or conflicting bytes
/// When its reply is lost and the process crashes
/// Then replay rejects PartialRetry or KeyConflict without implying a partly surviving new atomic submit
/// Cases: RETRY-UNKNOWN-3, ORACLE-4
#[test]
fn unknown_rejections_do_not_imply_an_atomic_original() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            for conflict in [false, true] {
                let sim = Sim::new(520);
                let log = open(&sim);
                send(&log, vec![record(b"s", Some(b"k"), b"one")])
                    .wait()
                    .unwrap();
                let replay = || {
                    vec![
                        record(b"fresh", Some(b"fresh"), b"new"),
                        record(b"s", Some(b"k"), if conflict { b"changed" } else { b"one" }),
                    ]
                };
                drop(send(&log, replay()));
                drop(log);
                sim.crash(mode, 31);
                let log = open(&sim);
                let answer = send(&log, replay()).wait();
                if conflict {
                    assert!(matches!(answer, Err(CommitError::KeyConflict { index: 1 })));
                } else {
                    assert!(matches!(answer, Err(CommitError::PartialRetry)));
                }
                assert_records(
                    &log.read(0, 10).unwrap(),
                    &[stored(b"s", Some(b"k"), b"one", 0, 0)],
                );
                assert_eq!(log.watermark(), 1);
                assert!(log.stream_head(b"fresh").is_none());
                no_new_group(&sim, 1);
                let result = send(&log, vec![record(b"fresh", Some(b"fresh"), b"new")])
                    .wait()
                    .unwrap();
                assert!(!result.deduplicated);
                assert_eq!(result.records[0].offset, 1);
            }
        }
    });
}

/// Given a durable record and a valid pending original selected by a regrouped retry
/// When the retry has matched but a crash cancels the new group's write
/// Then only the durable selection survives and replay is PartialRetry
/// And the pending original can still append as a new atomic submit
/// Cases: RETRY-UNKNOWN-3, RETRY-REGROUPED-FAIL, ORACLE-4
#[test]
fn unknown_regrouped_retry_can_have_only_a_subset_of_matches() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            let sim = Sim::new(521);
            let log = open(&sim);
            let held = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
            let bootstrap = send(&log, vec![record(b"s", Some(b"k"), b"one")]);
            held.wait();
            let original = send(&log, vec![record(b"t", Some(b"t"), b"two")]);
            let retry = send(
                &log,
                vec![
                    record(b"t", Some(b"t"), b"two"),
                    record(b"s", Some(b"k"), b"one"),
                ],
            );
            let bytes = crate::codec::encode_group(
                &log_id(&sim),
                &crate::codec::Group {
                    epoch: 1,
                    base_offset: 1,
                    entries: vec![crate::codec::Entry {
                        stream: b"t",
                        key: Some(b"t"),
                        payload: b"two",
                        revision: 0,
                    }],
                },
            )
            .unwrap();
            let next = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::Append(bytes)));
            held.release(None);
            bootstrap.wait().unwrap();
            next.wait();
            assert!(original.probe().is_none());
            assert!(retry.probe().is_none());
            drop((original, retry));
            sim.crash(mode, 31);
            next.release(None);
            drop(log);
            let log = open(&sim);
            assert!(matches!(
                send(
                    &log,
                    vec![
                        record(b"t", Some(b"t"), b"two"),
                        record(b"s", Some(b"k"), b"one")
                    ]
                )
                .wait(),
                Err(CommitError::PartialRetry)
            ));
            assert_records(
                &log.read(0, 10).unwrap(),
                &[stored(b"s", Some(b"k"), b"one", 0, 0)],
            );
            let result = send(&log, vec![record(b"t", Some(b"t"), b"two")])
                .wait()
                .unwrap();
            assert!(!result.deduplicated);
            assert_eq!(result.records[0].offset, 1);
        }
    });
}

/// Given a fully keyed request with a known WrongRevision rejection
/// When it is replayed unchanged before and after reopen without intervening records
/// Then it remains WrongRevision and consumes no key, revision or offset
/// Cases: RETRY-UNKNOWN-4, ORACLE-4
#[test]
fn known_revision_rejection_has_no_replay_success_guarantee() {
    bounded(|| {
        let sim = Sim::new(522);
        let mut log = open(&sim);
        send(&log, vec![record(b"s", Some(b"k"), b"one")])
            .wait()
            .unwrap();
        for reopen in [false, false, true] {
            if reopen {
                drop(log);
                log = open(&sim);
            }
            assert!(matches!(
                send(
                    &log,
                    vec![
                        record(b"t", Some(b"fresh"), b"new"),
                        record(b"s", Some(b"next"), b"two")
                    ]
                )
                .wait(),
                Err(CommitError::WrongRevision {
                    index: 1,
                    actual: Some(0)
                })
            ));
            assert_records(
                &log.read(0, 10).unwrap(),
                &[stored(b"s", Some(b"k"), b"one", 0, 0)],
            );
            no_new_group(&sim, 1);
        }
    });
}

/// Given a mixed keyed and unkeyed submit with a lost reply, or only the same keyed record
/// When each history survives a crash and the mixed request is replayed
/// Then both histories give PartialRetry without proving the unkeyed record existed
/// Cases: RETRY-UNKNOWN-5, RETRY-UNKNOWN-6, ORACLE-4
#[test]
fn partial_retry_cannot_prove_an_unkeyed_original_existed() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            for mixed in [false, true] {
                let sim = Sim::new(523);
                let log = open(&sim);
                let mut original = vec![record(b"s", Some(b"k"), b"one")];
                if mixed {
                    original.push(record(b"t", None, b"two"));
                }
                drop(send(&log, original));
                drop(log);
                sim.crash(mode, 31);
                let log = open(&sim);
                assert!(matches!(
                    send(
                        &log,
                        vec![record(b"s", Some(b"k"), b"one"), record(b"t", None, b"two")]
                    )
                    .wait(),
                    Err(CommitError::PartialRetry)
                ));
                let mut expected = vec![stored(b"s", Some(b"k"), b"one", 0, 0)];
                if mixed {
                    expected.push(stored(b"t", None, b"two", 1, 0));
                }
                assert_records(&log.read(0, 10).unwrap(), &expected);
                assert_eq!(log.watermark(), expected.len() as u64);
                assert_eq!(log.stream_head(b"t").is_some(), mixed);
                no_new_group(&sim, 1);
            }
        }
    });
}

/// Given a fully unkeyed atomic submit whose reply is lost but whose records survive a crash
/// When replay uses stale expectations, then advances them to the known heads
/// Then exact replay is WrongRevision and the advanced replay writes the payloads again
/// Cases: RETRY-UNKNOWN-7, RETRY-UNKNOWN-8, ORACLE-4
#[test]
fn unkeyed_survivors_require_revisions_and_can_repeat_payloads() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            let sim = Sim::new(524);
            let log = open(&sim);
            let original = || vec![record(b"s", None, b"one"), record(b"t", None, b"two")];
            drop(send(&log, original()));
            drop(log);
            sim.crash(mode, 31);
            let log = open(&sim);
            assert!(matches!(
                send(&log, original()).wait(),
                Err(CommitError::WrongRevision {
                    index: 0,
                    actual: Some(0)
                })
            ));
            no_new_group(&sim, 1);
            assert_records(
                &log.read(0, 10).unwrap(),
                &[
                    stored(b"s", None, b"one", 0, 0),
                    stored(b"t", None, b"two", 1, 0),
                ],
            );
            let mut replay = original();
            for r in &mut replay {
                r.expected = ExpectedRevision::At(0);
            }
            let result = send(&log, replay).wait().unwrap();
            assert!(!result.deduplicated);
            assert_eq!(
                result
                    .records
                    .iter()
                    .map(|p| (p.offset, p.revision))
                    .collect::<Vec<_>>(),
                [(2, 1), (3, 1)]
            );
            assert_records(
                &log.read(0, 10).unwrap(),
                &[
                    stored(b"s", None, b"one", 0, 0),
                    stored(b"t", None, b"two", 1, 0),
                    stored(b"s", None, b"one", 2, 1),
                    stored(b"t", None, b"two", 3, 1),
                ],
            );
            assert_eq!(log.watermark(), 4);
            assert_eq!(log.stream_head(b"s").unwrap().revision, 1);
            no_new_group(&sim, 2);
        }
    });
}
