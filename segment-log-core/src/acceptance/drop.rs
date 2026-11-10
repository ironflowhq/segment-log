use super::support::{bounded, fixtures::*};
use crate::storage::sim::Fault;
use crate::storage::{Call, Op, Purpose};
use crate::*;
use std::{
    path::Path,
    sync::{Arc, mpsc},
    time::Duration,
};

fn try_open(sim: &Sim) -> Result<SegmentLog, OpenError> {
    SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone()))
}
fn keyed(stream: u8) -> NewRecord {
    NewRecord {
        stream: vec![stream],
        key: Some(vec![stream]),
        payload: vec![],
        expected: ExpectedRevision::Empty,
    }
}
fn send(log: &SegmentLog, streams: &[u8]) -> Pending {
    log.append(Submit {
        records: streams.iter().map(|s| keyed(*s)).collect(),
    })
    .unwrap()
}
fn positions(answer: Committed, offsets: &[u64], retry: bool) {
    assert_eq!(answer.deduplicated, retry);
    assert_eq!(
        answer
            .records
            .iter()
            .map(|p| (p.offset, p.revision))
            .collect::<Vec<_>>(),
        offsets.iter().map(|o| (*o, 0)).collect::<Vec<_>>()
    );
}
fn flush() -> Call {
    Call::new(ANY_SEG, Purpose::Group, Op::SyncData)
}
fn group_write(base: u64, streams: &[u8]) -> Call {
    Call::new(
        ANY_SEG,
        Purpose::Group,
        Op::Append(
            crate::codec::encode(&crate::codec::Group {
                epoch: 1,
                base_offset: base,
                entries: streams
                    .iter()
                    .map(|s| crate::codec::Entry {
                        stream: std::slice::from_ref(s),
                        key: Some(std::slice::from_ref(s)),
                        payload: b"",
                        revision: 0,
                    })
                    .collect(),
            })
            .unwrap(),
        ),
    )
}

/// Given queued originals and exact and regrouped retries with a linger beyond the watchdog
/// When Drop closes admission and the drain reaches a held write or flush
/// Then a second open is Locked until that writer finishes and Drop returns
/// And result handles outlive the log, succeeding or reporting the injected failure
/// And same-process reopen rebuilds the complete written prefix and retry keys
/// Cases: DROP, ORACLE-11, ORACLE-7, RETRY-PENDING, RETRY-REGROUPED, GROUP-FORMATION
#[test]
fn drop_holds_the_lock_until_its_drain_finishes() {
    bounded(|| {
        for (at_flush, fault) in [
            (false, None),
            (true, None),
            (false, Some(Fault::NoSpace)),
            (false, Some(Fault::Partial(35))),
            (true, Some(Fault::NoSpace)),
            (true, Some(Fault::Panic)),
        ] {
            let sim = Sim::new(703);
            let log = SegmentLog::with_storage(
                Path::new("/log"),
                Options {
                    linger: Duration::from_secs(60),
                    ..options()
                },
                Arc::new(sim.clone()),
            )
            .unwrap();
            let held = sim.hold(if at_flush {
                flush()
            } else {
                group_write(0, b"abc")
            });
            let first = send(&log, b"ab");
            let second = send(&log, b"c");
            let exact = send(&log, b"ab");
            let regrouped = send(&log, b"ca");
            let (done_tx, done_rx) = mpsc::channel();
            let dropper = std::thread::spawn(move || {
                drop(log);
                done_tx.send(()).unwrap();
            });
            held.wait();
            let still_dropping = matches!(done_rx.try_recv(), Err(mpsc::TryRecvError::Empty));
            let second_open = try_open(&sim);
            let waiting = [&first, &second, &exact, &regrouped]
                .iter()
                .all(|p| p.probe().is_none());
            let failed = fault.is_some();
            held.release(fault);
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            dropper.join().unwrap();
            assert!(still_dropping);
            assert!(matches!(second_open, Err(OpenError::Locked)));
            assert!(waiting);
            if failed {
                for p in [first, second, exact, regrouped] {
                    assert!(matches!(p.wait(), Err(CommitError::Indeterminate)));
                }
            } else {
                positions(first.wait().unwrap(), &[0, 1], false);
                positions(second.wait().unwrap(), &[2], false);
                positions(exact.wait().unwrap(), &[0, 1], true);
                positions(regrouped.wait().unwrap(), &[2, 0], true);
            }
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
                1
            );
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
                usize::from(!failed || at_flush)
            );
            let recovered = try_open(&sim).unwrap();
            let expected = if failed && !at_flush {
                vec![]
            } else {
                vec![
                    stored(b"a", Some(b"a"), b"", 0, 0),
                    stored(b"b", Some(b"b"), b"", 1, 0),
                    stored(b"c", Some(b"c"), b"", 2, 0),
                ]
            };
            assert_eq!(recovered.watermark(), expected.len() as u64);
            assert_records(&recovered.read(0, 10).unwrap(), &expected);
            if !expected.is_empty() {
                positions(send(&recovered, b"ca").wait().unwrap(), &[2, 0], true);
                assert_eq!(
                    sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
                    1
                );
            }
        }
    });
}

/// Given a held group with an original and matched waiters, plus deferred and queued work
/// When Drop has closed admission and the group write or flush fails
/// Then unfinished originals and waiters are Indeterminate and unprocessed work is Poisoned
/// And Drop releases the lock without another write or flush
/// When reopened in the same process
/// Then only the complete written prefix and its keys are recovered
/// Cases: DROP, ORACLE-7, ORACLE-11, RETRY-PENDING-FAIL, RETRY-REGROUPED-FAIL
#[test]
fn failure_after_drop_closes_admission_poisons_deferred_and_queued_work() {
    bounded(|| {
        for (at_flush, fault) in [
            (false, Fault::NoSpace),
            (false, Fault::Partial(35)),
            (true, Fault::NoSpace),
        ] {
            let sim = Sim::new(706);
            let mut log = SegmentLog::with_storage(
                Path::new("/log"),
                Options {
                    group_byte_cap: 24 + 2 * (16 + 1 + 1),
                    ..options()
                },
                Arc::new(sim.clone()),
            )
            .unwrap();
            let held = sim.hold(flush());
            let boot = send(&log, b"z");
            held.wait();
            let first = send(&log, b"ab");
            let exact = send(&log, b"ab");
            let subset = send(&log, b"b");
            let deferred = send(&log, b"c");
            let next = sim.hold(if at_flush {
                flush()
            } else {
                group_write(1, b"ab")
            });
            held.release(None);
            positions(boot.wait().unwrap(), &[0], false);
            next.wait();
            let queued_new = send(&log, b"d");
            let queued_retry = send(&log, b"ab");
            let closed = log.observe_drop();
            let (done_tx, done_rx) = mpsc::channel();
            let dropper = std::thread::spawn(move || {
                drop(log);
                done_tx.send(()).unwrap();
            });
            let closure = closed.recv_timeout(Duration::from_secs(5));
            let second_open = try_open(&sim);
            let waiting = [
                &first,
                &exact,
                &subset,
                &deferred,
                &queued_new,
                &queued_retry,
            ]
            .iter()
            .all(|p| p.probe().is_none());
            next.release(Some(fault));
            closure.unwrap();
            assert!(waiting);
            assert!(matches!(second_open, Err(OpenError::Locked)));
            for p in [first, exact, subset] {
                assert!(matches!(p.wait(), Err(CommitError::Indeterminate)));
            }
            for p in [deferred, queued_new, queued_retry] {
                assert!(matches!(p.wait(), Err(CommitError::Poisoned)));
            }
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            dropper.join().unwrap();
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
                2
            );
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
                1 + usize::from(at_flush)
            );
            let recovered = try_open(&sim).unwrap();
            let mut expected = vec![stored(b"z", Some(b"z"), b"", 0, 0)];
            if at_flush {
                expected.extend([
                    stored(b"a", Some(b"a"), b"", 1, 0),
                    stored(b"b", Some(b"b"), b"", 2, 0),
                ]);
            }
            assert_records(&recovered.read(0, 10).unwrap(), &expected);
            assert_eq!(recovered.watermark(), expected.len() as u64);
            assert!(recovered.stream_head(b"c").is_none());
            assert!(recovered.stream_head(b"d").is_none());
            if at_flush {
                positions(send(&recovered, b"ab").wait().unwrap(), &[1, 2], true);
            }
        }
    });
}

/// Given a poisoned writer or a writer stopped by a storage panic with unanswered work
/// When the caller receives the lost or failed replies and drops the log
/// Then Drop returns, releases its lock and keeps Pending answers usable afterward
/// And same-process reopen preserves acknowledged records and resolves the unknown keyed suffix
/// Cases: DROP, ORACLE-7, ORACLE-1, ORACLE-4
#[test]
fn dropping_an_already_failed_writer_releases_the_lock() {
    bounded(|| {
        for panic in [false, true] {
            let sim = Sim::new(704);
            let log = open(&sim);
            positions(send(&log, b"z").wait().unwrap(), &[0], false);
            let held = sim.hold(flush());
            let original = send(&log, b"ab");
            held.wait();
            let queued = send(&log, b"c");
            held.release(Some(if panic { Fault::Panic } else { Fault::NoSpace }));
            assert!(matches!(original.wait(), Err(CommitError::Indeterminate)));
            assert!(matches!(try_open(&sim), Err(OpenError::Locked)));
            drop(log);
            let answer = queued.wait();
            assert!(
                matches!(answer, Err(e) if if panic { matches!(e, CommitError::Indeterminate) } else { matches!(e, CommitError::Poisoned) })
            );
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
                2
            );
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
                2
            );
            let recovered = try_open(&sim).unwrap();
            assert_records(
                &recovered.read(0, 10).unwrap(),
                &[
                    stored(b"z", Some(b"z"), b"", 0, 0),
                    stored(b"a", Some(b"a"), b"", 1, 0),
                    stored(b"b", Some(b"b"), b"", 2, 0),
                ],
            );
            positions(send(&recovered, b"ab").wait().unwrap(), &[1, 2], true);
            assert!(recovered.stream_head(b"c").is_none());
        }
    });
}
