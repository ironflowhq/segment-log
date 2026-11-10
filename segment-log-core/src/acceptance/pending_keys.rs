use super::support::{bounded, fixtures::*, raw};
use crate::storage::sim::{Crash, Fault};
use crate::storage::{Call, Op, Purpose};
use crate::*;
use std::{path::Path, sync::Arc};

fn record(stream: &[u8], key: Option<&[u8]>, payload: &[u8]) -> NewRecord {
    NewRecord {
        stream: stream.to_vec(),
        key: key.map(<[u8]>::to_vec),
        payload: payload.to_vec(),
        expected: ExpectedRevision::Empty,
    }
}
fn a() -> NewRecord {
    record(b"s", Some(b"a"), b"one")
}
fn b() -> NewRecord {
    let mut r = record(b"s", Some(b"b"), b"two");
    r.expected = ExpectedRevision::At(0);
    r
}
fn c() -> NewRecord {
    let mut r = record(b"s", Some(b"c"), b"three");
    r.expected = ExpectedRevision::At(1);
    r
}
fn boot() -> NewRecord {
    record(b"boot", Some(b"boot"), b"durable")
}
fn send(log: &SegmentLog, records: Vec<NewRecord>) -> Pending {
    log.append(Submit { records }).unwrap()
}
fn positions(result: Committed, expected: &[(u64, u64)], retry: bool) {
    assert_eq!(result.deduplicated, retry);
    assert_eq!(
        result
            .records
            .iter()
            .map(|p| (p.offset, p.revision))
            .collect::<Vec<_>>(),
        expected
    );
}
fn flush() -> Call {
    Call::new(ANY_SEG, Purpose::Group, Op::SyncData)
}
fn calls(sim: &Sim, writes: usize, flushes: usize) {
    assert_eq!(
        sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
        writes
    );
    assert_eq!(
        sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
        flushes
    );
}
fn history() -> Vec<Record> {
    vec![
        stored(b"boot", Some(b"boot"), b"durable", 0, 0),
        stored(b"s", Some(b"a"), b"one", 1, 0),
        stored(b"s", Some(b"b"), b"two", 2, 1),
        stored(b"s", Some(b"c"), b"three", 3, 2),
    ]
}

/// Given valid pending originals and a durable key behind a held bootstrap flush
/// When fresh prefixes, multiple conflicts and keyed/unkeyed partial retries arrive
/// Then first conflict wins before revisions and partials reject in either order
/// And rejections finalize before flush without reserving their fresh keys
/// Cases: KEY-CONFLICT, KEY-PARTIAL, ANSWER-TIMING, REV-EXPECTED, KEY-REPEATED
#[test]
fn pending_rejections_are_immediate_and_leave_no_reservations() {
    bounded(|| {
        for changed_stream in [false, true] {
            let sim = Sim::new(501);
            let log = open(&sim);
            let held = sim.hold(flush());
            let bootstrap = send(&log, vec![boot()]);
            held.wait();
            let original = send(&log, vec![a(), b()]);
            assert!(matches!(
                log.append(Submit {
                    records: vec![a(), a()]
                }),
                Err(AppendError::Invalid(_))
            ));
            let fresh = || record(b"fresh", Some(b"fresh"), b"new");
            let conflict = send(
                &log,
                vec![
                    fresh(),
                    a(),
                    if changed_stream {
                        record(b"other", Some(b"b"), b"two")
                    } else {
                        record(b"s", Some(b"b"), b"changed")
                    },
                    record(b"other", Some(b"boot"), b"bad"),
                ],
            );
            let mut partials = vec![];
            for keyed in [false, true] {
                for reverse in [false, true] {
                    let mut records = vec![
                        a(),
                        boot(),
                        record(b"fresh", keyed.then_some(b"fresh"), b"new"),
                    ];
                    if reverse {
                        records.reverse();
                    }
                    partials.push(send(&log, records));
                }
            }
            let wrong = send(&log, vec![fresh(), record(b"s", Some(b"unused"), b"wrong")]);
            let valid = send(
                &log,
                vec![fresh(), record(b"other", Some(b"unused"), b"valid")],
            );
            let next = sim.hold(flush());
            held.release(None);
            bootstrap.wait().unwrap();
            next.wait();
            let conflict = conflict.probe();
            let partials: Vec<_> = partials.iter().map(Pending::probe).collect();
            let wrong = wrong.probe();
            assert_eq!(log.watermark(), 1);
            assert!(log.stream_head(b"s").is_none());
            next.release(None);
            assert!(
                matches!(conflict, Some(Err(CommitError::KeyConflict { index: 2 }))),
                "{conflict:?}"
            );
            for answer in partials {
                assert!(
                    matches!(answer, Some(Err(CommitError::PartialRetry))),
                    "{answer:?}"
                );
            }
            assert!(matches!(
                wrong,
                Some(Err(CommitError::WrongRevision {
                    index: 1,
                    actual: Some(1)
                }))
            ));
            positions(original.wait().unwrap(), &[(1, 0), (2, 1)], false);
            positions(valid.wait().unwrap(), &[(3, 0), (4, 0)], false);
            assert_eq!(log.watermark(), 5);
            assert_records(
                &log.read(0, 10).unwrap(),
                &[
                    stored(b"boot", Some(b"boot"), b"durable", 0, 0),
                    stored(b"s", Some(b"a"), b"one", 1, 0),
                    stored(b"s", Some(b"b"), b"two", 2, 1),
                    stored(b"fresh", Some(b"fresh"), b"new", 3, 0),
                    stored(b"other", Some(b"unused"), b"valid", 4, 0),
                ],
            );
            calls(&sim, 2, 2);
        }
    });
}

/// Given two pending originals on one stream and one durable record
/// When exact, stale, subset, reordered and mixed retries join the queued group
/// Then all wait for its flush and watermark and return positions in input order
/// And retries consume no records, counters or additional flush, including after reopen
/// Cases: RETRY-PENDING, RETRY-REGROUPED, REV-EXPECTED, ORACLE-8, READ-BOUNDED
#[test]
fn pending_and_mixed_retries_wait_for_the_one_original_group() {
    bounded(|| {
        let sim = Sim::new(502);
        let log = open(&sim);
        let held = sim.hold(flush());
        let bootstrap = send(&log, vec![boot()]);
        held.wait();
        let first = send(&log, vec![a(), b()]);
        let second = send(&log, vec![c()]);
        let exact = send(&log, vec![a(), b()]);
        let mut stale_b = b();
        stale_b.expected = ExpectedRevision::Empty;
        let stale = send(&log, vec![a(), stale_b]);
        let subset = send(&log, vec![b()]);
        let regrouped = send(&log, vec![c(), a()]);
        let mixed = send(&log, vec![c(), boot(), b()]);
        let next = sim.hold(flush());
        held.release(None);
        bootstrap.wait().unwrap();
        next.wait();
        let unanswered = [&first, &second, &exact, &stale, &subset, &regrouped, &mixed]
            .iter()
            .all(|p| p.probe().is_none());
        assert_eq!(log.watermark(), 1);
        assert!(log.stream_head(b"s").is_none());
        assert_records(&log.read(0, 10).unwrap(), &history()[..1]);
        next.release(None);
        assert!(
            unanswered,
            "matched pending retries must not be rejected or answered before flush"
        );
        positions(first.wait().unwrap(), &[(1, 0), (2, 1)], false);
        positions(second.wait().unwrap(), &[(3, 2)], false);
        for p in [exact, stale] {
            positions(p.wait().unwrap(), &[(1, 0), (2, 1)], true);
        }
        positions(subset.wait().unwrap(), &[(2, 1)], true);
        positions(regrouped.wait().unwrap(), &[(3, 2), (1, 0)], true);
        positions(mixed.wait().unwrap(), &[(3, 2), (0, 0), (2, 1)], true);
        assert_records(&log.read(0, 10).unwrap(), &history());
        assert_eq!(log.watermark(), 4);
        assert_eq!(log.stream_head(b"s").unwrap().revision, 2);
        calls(&sim, 2, 2);
        drop(log);
        let log = open(&sim);
        positions(
            send(&log, vec![c(), a(), boot()]).wait().unwrap(),
            &[(3, 2), (1, 0), (0, 0)],
            true,
        );
        assert_records(&log.read(0, 10).unwrap(), &history());
        calls(&sim, 2, 2);
    });
}

/// Given matched pending and mixed waiters in a group, followed by queued retries
/// When its append partially writes, runs out of space or its flush fails, including Drop drain
/// Then originals and matched waiters are Indeterminate and unclassified retries Poisoned
/// And no later write or flush occurs
/// Cases: RETRY-PENDING-FAIL, RETRY-REGROUPED-FAIL, ORACLE-7, DROP
#[test]
fn failed_groups_drain_matched_waiters_and_poison_queued_retries() {
    bounded(|| {
        for draining in [false, true] {
            for fault in [
                Fault::Partial(35),
                Fault::NoSpace,
                Fault::error(std::io::ErrorKind::Other).unwrap(),
            ] {
                let sim = Sim::new(503);
                let log = open(&sim);
                let held = sim.hold(flush());
                let bootstrap = send(&log, vec![boot()]);
                held.wait();
                let first = send(&log, vec![a(), b()]);
                let second = send(&log, vec![c()]);
                let exact = send(&log, vec![a(), b()]);
                let mixed = send(&log, vec![c(), boot(), a()]);
                let at_flush = matches!(fault, Fault::Error(_));
                let next = sim.hold(if at_flush {
                    flush()
                } else {
                    Call::new(ANY_SEG, Purpose::Group, Op::Append(pending_frame()))
                });
                held.release(None);
                bootstrap.wait().unwrap();
                next.wait();
                let queued = send(&log, vec![c(), a()]);
                let dropper = if draining {
                    Some(std::thread::spawn(move || drop(log)))
                } else {
                    None
                };
                next.release(Some(fault));
                for p in [first, second, exact, mixed] {
                    let answer = p.wait();
                    assert!(
                        matches!(answer, Err(CommitError::Indeterminate)),
                        "{answer:?}"
                    );
                }
                assert!(matches!(queued.wait(), Err(CommitError::Poisoned)));
                if let Some(t) = dropper {
                    t.join().unwrap();
                }
                calls(&sim, 2, if at_flush { 2 } else { 1 });
            }
        }
    });
}

fn pending_frame() -> Vec<u8> {
    crate::codec::encode(&crate::codec::Group {
        epoch: 1,
        base_offset: 1,
        entries: vec![
            crate::codec::Entry {
                stream: b"s",
                key: Some(b"a"),
                payload: b"one",
                revision: 0,
            },
            crate::codec::Entry {
                stream: b"s",
                key: Some(b"b"),
                payload: b"two",
                revision: 1,
            },
            crate::codec::Entry {
                stream: b"s",
                key: Some(b"c"),
                payload: b"three",
                revision: 2,
            },
        ],
    })
    .unwrap()
}

/// Given a pending original and a large-capacity matched retry whose handle is dropped
/// When its group flush is held and another large request seeks admission
/// Then the full retained capacity remains charged until answer, then becomes reusable
/// Cases: ADM-RESERVATION, ADM-CHARGE, RETRY-PENDING
#[test]
fn dropped_matched_waiter_holds_its_full_charge_until_answer() {
    bounded(|| {
        let sim = Sim::new(504);
        let log = SegmentLog::with_storage(
            Path::new("/log"),
            Options {
                pending_byte_budget: 4096,
                ..options()
            },
            Arc::new(sim.clone()),
        )
        .unwrap();
        let large = || {
            let mut r = a();
            r.payload = Vec::with_capacity(2500);
            r.payload.extend(b"one");
            Submit { records: vec![r] }
        };
        let held = sim.hold(flush());
        let bootstrap = send(&log, vec![boot()]);
        held.wait();
        let original = send(&log, vec![a()]);
        drop(log.append(large()).unwrap());
        assert!(matches!(log.append(large()), Err(AppendError::Full)));
        let next = sim.hold(flush());
        held.release(None);
        bootstrap.wait().unwrap();
        next.wait();
        let admission = log.append(large());
        next.release(None);
        assert!(
            matches!(admission, Err(AppendError::Full)),
            "dropped waiter must retain its capacity charge"
        );
        original.wait().unwrap();
        positions(send(&log, vec![a()]).wait().unwrap(), &[(1, 0)], true);
        positions(
            log.append(large()).unwrap().wait().unwrap(),
            &[(1, 0)],
            true,
        );
        calls(&sim, 2, 2);
    });
}

/// Given new originals and all-pending and mixed matched waiters in an unwritten group
/// When a later mixed retry's durable lookup fails after matching pending records
/// Then every unfinished request is Indeterminate while queued retries are Poisoned
/// And finalized answers remain fixed and no group write or flush follows the fault
/// Cases: POISON-LOOKUP, RETRY-REGROUPED-FAIL, ANSWER-TIMING, ORACLE-7
#[test]
fn lookup_failure_drains_pending_waiters_before_any_group_write() {
    bounded(|| {
        for fault in [
            Some(Fault::NoSpace),
            Some(Fault::Eof),
            Some(Fault::Corrupt(8, 1)),
            None,
        ] {
            let sim = Sim::new(505);
            let log = open(&sim);
            send(&log, vec![boot()]).wait().unwrap();
            let held = sim.hold(flush());
            let bootstrap = send(&log, vec![record(b"lookup", Some(b"lookup"), b"target")]);
            held.wait();
            let (segment, position, frame) = raw::last_frame(&sim);
            let first = send(&log, vec![a(), b()]);
            let second = send(&log, vec![c()]);
            let exact = send(&log, vec![a(), b()]);
            let mixed = send(&log, vec![c(), boot(), a()]);
            let conflict = send(&log, vec![record(b"other", Some(b"a"), b"one")]);
            let partial = send(&log, vec![a(), record(b"fresh", None, b"")]);
            let wrong = send(&log, vec![record(b"s", None, b"")]);
            let durable = send(&log, vec![boot()]);
            let failing = send(
                &log,
                vec![c(), boot(), record(b"lookup", Some(b"lookup"), b"target")],
            );
            let lookup = Call::new(&segment, Purpose::Lookup, Op::Read(position, frame.len()));
            let next = sim.hold(lookup);
            held.release(None);
            bootstrap.wait().unwrap();
            next.wait();
            let queued = send(&log, vec![a(), b()]);
            let answers = (
                conflict.probe(),
                partial.probe(),
                wrong.probe(),
                durable.probe(),
            );
            if fault.is_none() {
                use crate::storage::Storage;
                let mut bytes = frame;
                bytes[9] = 1;
                let input = [&bytes[..4], &bytes[8..]].concat();
                bytes[4..8].copy_from_slice(&raw::crc(&input).to_le_bytes());
                sim.call(Call::new(
                    &segment,
                    Purpose::Open,
                    Op::Write(position, bytes),
                ))
                .unwrap();
            }
            let malformed = fault.is_none();
            next.release(fault);
            for p in [first, second, exact, mixed, failing] {
                let answer = p.wait();
                assert!(
                    matches!(answer, Err(CommitError::Indeterminate)),
                    "{answer:?}"
                );
            }
            assert!(matches!(queued.wait(), Err(CommitError::Poisoned)));
            assert!(matches!(
                answers.0,
                Some(Err(CommitError::KeyConflict { index: 0 }))
            ));
            assert!(matches!(answers.1, Some(Err(CommitError::PartialRetry))));
            assert!(matches!(
                answers.2,
                Some(Err(CommitError::WrongRevision {
                    actual: Some(2),
                    ..
                }))
            ));
            positions(answers.3.unwrap().unwrap(), &[(0, 0)], true);
            assert_eq!(log.watermark(), 2);
            assert!(log.stream_head(b"s").is_none());
            if !malformed {
                assert_records(
                    &log.read(0, 10).unwrap(),
                    &[
                        stored(b"boot", Some(b"boot"), b"durable", 0, 0),
                        stored(b"lookup", Some(b"lookup"), b"target", 1, 0),
                    ],
                );
            } else {
                assert_records(&log.read(0, 1).unwrap(), &history()[..1]);
            }
            calls(&sim, 2, 2);
        }
    });
}

/// Given fully keyed atomic originals, matched waiters and finalized pending rejections
/// When each crash mode interrupts append or flush, or follows acknowledgment
/// Then recovery keeps an atomic prefix of original records and no rejected fresh record
/// And replay resolves surviving keys once or appends absent originals at their original positions
/// Cases: KEY-CONFLICT-CRASH, KEY-PARTIAL-CRASH, RETRY-PENDING-FAIL, ORACLE-1, ORACLE-2, ORACLE-3, ORACLE-4, ORACLE-5
#[test]
fn pending_rejections_and_originals_survive_only_as_a_valid_prefix() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            for point in 0..3 {
                for seed in 0..8 {
                    let sim = Sim::new(506);
                    let log = open(&sim);
                    let held = sim.hold(flush());
                    let bootstrap = send(&log, vec![boot()]);
                    held.wait();
                    let first = send(&log, vec![a(), b()]);
                    let second = send(&log, vec![c()]);
                    let exact = send(&log, vec![a(), b()]);
                    let mixed = send(&log, vec![c(), boot(), a()]);
                    let conflict = send(
                        &log,
                        vec![
                            record(b"fresh", Some(b"fresh"), b"never"),
                            record(b"other", Some(b"a"), b"one"),
                        ],
                    );
                    let partial = send(
                        &log,
                        vec![record(b"fresh", Some(b"fresh"), b"never"), b(), boot()],
                    );
                    let next = sim.hold(if point == 0 {
                        Call::new(ANY_SEG, Purpose::Group, Op::Append(pending_frame()))
                    } else {
                        flush()
                    });
                    held.release(None);
                    bootstrap.wait().unwrap();
                    next.wait();
                    let conflict = conflict.probe();
                    let partial = partial.probe();
                    assert!(matches!(
                        conflict,
                        Some(Err(CommitError::KeyConflict { index: 1 }))
                    ));
                    assert!(matches!(partial, Some(Err(CommitError::PartialRetry))));
                    if point == 2 {
                        next.release(None);
                        positions(first.wait().unwrap(), &[(1, 0), (2, 1)], false);
                        positions(second.wait().unwrap(), &[(3, 2)], false);
                        positions(exact.wait().unwrap(), &[(1, 0), (2, 1)], true);
                        positions(mixed.wait().unwrap(), &[(3, 2), (0, 0), (1, 0)], true);
                        sim.crash(mode, seed);
                    } else {
                        drop((first, second, exact, mixed));
                        sim.crash(mode, seed);
                        next.release(None);
                    }
                    drop(log);
                    let log = open(&sim);
                    let end = log.watermark();
                    assert!(
                        end == 1 || end == 4,
                        "mode {mode:?}, point {point}, seed {seed}: {end}"
                    );
                    if point == 0 {
                        assert_eq!(end, 1);
                    }
                    if point == 2 || (point == 1 && mode == Crash::Kill) {
                        assert_eq!(end, 4);
                    }
                    assert_records(&log.read(0, 10).unwrap(), &history()[..end as usize]);
                    assert!(log.stream_head(b"fresh").is_none());
                    drop(log);
                    sim.crash(Crash::PowerLoss, seed);
                    let log = open(&sim);
                    assert_eq!(log.watermark(), end);
                    assert_records(&log.read(0, 10).unwrap(), &history()[..end as usize]);
                    positions(
                        send(&log, vec![a(), b()]).wait().unwrap(),
                        &[(1, 0), (2, 1)],
                        end == 4,
                    );
                    positions(send(&log, vec![c()]).wait().unwrap(), &[(3, 2)], end == 4);
                    positions(
                        send(&log, vec![c(), a(), boot()]).wait().unwrap(),
                        &[(3, 2), (1, 0), (0, 0)],
                        true,
                    );
                    assert_records(&log.read(0, 10).unwrap(), &history());
                    positions(
                        send(&log, vec![record(b"fresh", Some(b"fresh"), b"now")])
                            .wait()
                            .unwrap(),
                        &[(4, 0)],
                        false,
                    );
                    drop(log);
                    sim.crash(Crash::PowerLoss, seed);
                    let log = open(&sim);
                    let mut expected = history();
                    expected.push(stored(b"fresh", Some(b"fresh"), b"now", 4, 0));
                    assert_records(&log.read(0, 10).unwrap(), &expected);
                }
            }
        }
    });
}

/// Given each write or flush failure with matched regrouped waiters
/// When Kill, PowerLoss or PowerLossTorn is followed by reopen
/// Then the two original submits survive atomically or not at all
/// And retries restore each key exactly once without any regrouped retry record
/// Cases: RETRY-PENDING-FAIL, RETRY-REGROUPED-FAIL, ORACLE-2, ORACLE-3, ORACLE-4
#[test]
fn failed_pending_groups_recover_originals_without_retry_records() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            for fault in [
                Fault::Partial(35),
                Fault::Partial(pending_frame().len()),
                Fault::NoSpace,
                Fault::error(std::io::ErrorKind::Other).unwrap(),
            ] {
                let sim = Sim::new(507);
                let log = open(&sim);
                let held = sim.hold(flush());
                let bootstrap = send(&log, vec![boot()]);
                held.wait();
                let first = send(&log, vec![a(), b()]);
                let second = send(&log, vec![c()]);
                let retry = send(&log, vec![c(), boot(), a()]);
                let at_flush = matches!(fault, Fault::Error(_));
                let next = sim.hold(if at_flush {
                    flush()
                } else {
                    Call::new(ANY_SEG, Purpose::Group, Op::Append(pending_frame()))
                });
                held.release(None);
                bootstrap.wait().unwrap();
                next.wait();
                next.release(Some(fault));
                for p in [first, second, retry] {
                    assert!(matches!(p.wait(), Err(CommitError::Indeterminate)));
                }
                assert_eq!(log.watermark(), 1);
                drop(log);
                calls(&sim, 2, if at_flush { 2 } else { 1 });
                sim.crash(mode, 17);
                let log = open(&sim);
                let end = log.watermark();
                assert!(end == 1 || end == 4);
                assert_records(&log.read(0, 10).unwrap(), &history()[..end as usize]);
                positions(
                    send(&log, vec![a(), b()]).wait().unwrap(),
                    &[(1, 0), (2, 1)],
                    end == 4,
                );
                positions(send(&log, vec![c()]).wait().unwrap(), &[(3, 2)], end == 4);
                assert_records(&log.read(0, 10).unwrap(), &history());
                assert_eq!(log.watermark(), 4);
            }
        }
    });
}

/// Given a group cap that defers the second original and a retry queued behind it
/// When the first group flush succeeds or fails
/// Then the deferred key is not reserved, no retry overtakes it, and each waiter follows its group
/// Cases: GROUP-FORMATION, RETRY-PENDING, RETRY-PENDING-FAIL
#[test]
fn deferred_original_keys_do_not_leak_into_the_previous_group() {
    bounded(|| {
        for fail in [false, true] {
            let sim = Sim::new(508);
            let log = SegmentLog::with_storage(
                Path::new("/log"),
                Options {
                    group_byte_cap: 65,
                    ..options()
                },
                Arc::new(sim.clone()),
            )
            .unwrap();
            let held = sim.hold(flush());
            let bootstrap = send(&log, vec![boot()]);
            held.wait();
            let first = send(&log, vec![a()]);
            let retry_first = send(&log, vec![a()]);
            let second = send(&log, vec![b()]);
            let retry_second = send(&log, vec![b()]);
            let next = sim.hold(flush());
            held.release(None);
            bootstrap.wait().unwrap();
            next.wait();
            let unanswered = [&first, &retry_first, &second, &retry_second]
                .iter()
                .all(|p| p.probe().is_none());
            next.release(fail.then_some(Fault::NoSpace));
            assert!(unanswered);
            if fail {
                for p in [first, retry_first] {
                    assert!(matches!(p.wait(), Err(CommitError::Indeterminate)));
                }
                for p in [second, retry_second] {
                    assert!(matches!(p.wait(), Err(CommitError::Poisoned)));
                }
                calls(&sim, 2, 2);
            } else {
                positions(first.wait().unwrap(), &[(1, 0)], false);
                positions(retry_first.wait().unwrap(), &[(1, 0)], true);
                positions(second.wait().unwrap(), &[(2, 1)], false);
                positions(retry_second.wait().unwrap(), &[(2, 1)], true);
                assert_records(&log.read(0, 10).unwrap(), &history()[..3]);
                calls(&sim, 3, 3);
            }
        }
    });
}

/// Given the last assignable revision or offset and queued matching pending retries
/// When conflict, partial and exhausted new requests are classified
/// Then keys win first, retry positions are retained, and exhausted requests reserve no fresh key
/// Cases: COUNTER-EXHAUSTION, RETRY-PENDING, KEY-CONFLICT, KEY-PARTIAL
#[test]
fn pending_keys_precede_exhausted_counters() {
    bounded(|| {
        for offset_limit in [false, true] {
            let sim = Sim::new(509);
            let log = open(&sim);
            log.seed_counters(
                if offset_limit { u64::MAX - 2 } else { 0 },
                if offset_limit {
                    &[]
                } else {
                    &[(b"s", u64::MAX - 1)]
                },
            );
            let held = sim.hold(flush());
            let bootstrap = send(&log, vec![boot()]);
            held.wait();
            let original = || {
                let mut r = a();
                if !offset_limit {
                    r.expected = ExpectedRevision::At(u64::MAX - 1);
                }
                r
            };
            let new = send(&log, vec![original()]);
            let retry = send(&log, vec![a()]);
            let conflict = send(&log, vec![record(b"other", Some(b"a"), b"one")]);
            let partial = send(&log, vec![a(), record(b"fresh", Some(b"fresh"), b"")]);
            let exhausted = send(
                &log,
                vec![record(b"fresh", Some(b"fresh"), b""), {
                    let mut r = record(b"s", Some(b"last"), b"");
                    r.expected = ExpectedRevision::At(if offset_limit { 0 } else { u64::MAX });
                    r
                }],
            );
            let next = sim.hold(flush());
            held.release(None);
            bootstrap.wait().unwrap();
            next.wait();
            let answers = (conflict.probe(), partial.probe(), exhausted.probe());
            let waiting = retry.probe().is_none();
            next.release(None);
            assert!(waiting);
            assert!(matches!(
                answers.0,
                Some(Err(CommitError::KeyConflict { index: 0 }))
            ));
            assert!(matches!(answers.1, Some(Err(CommitError::PartialRetry))));
            assert!(matches!(answers.2, Some(Err(CommitError::Exhausted))));
            let pos = if offset_limit {
                (u64::MAX - 1, 0)
            } else {
                (1, u64::MAX)
            };
            positions(new.wait().unwrap(), &[pos], false);
            positions(retry.wait().unwrap(), &[pos], true);
            if !offset_limit {
                positions(
                    send(&log, vec![record(b"fresh", Some(b"fresh"), b"valid")])
                        .wait()
                        .unwrap(),
                    &[(2, 0)],
                    false,
                );
            }
        }
    });
}

/// Given a sixty-second linger and a cap that forces the bootstrap group to flush
/// When pending-key rejections and a revision rejection precede a held durable lookup
/// Then their answers finalize before linger expires or the new group writes
/// And a durable retry also finalizes before a later group failure while matched waiters fail
/// Cases: ANSWER-TIMING, RETRY-PENDING-FAIL, KEY-CONFLICT, KEY-PARTIAL
#[test]
fn terminal_answers_do_not_wait_for_positive_linger() {
    bounded(|| {
        let sim = Sim::new(510);
        let log = SegmentLog::with_storage(
            Path::new("/log"),
            Options {
                linger: std::time::Duration::from_secs(60),
                group_byte_cap: 65,
                ..options()
            },
            Arc::new(sim.clone()),
        )
        .unwrap();
        let held = sim.hold(flush());
        let bootstrap = send(&log, vec![boot()]);
        let original = send(&log, vec![a()]);
        held.wait();
        let retry = send(&log, vec![a()]);
        let conflict = send(&log, vec![record(b"x", Some(b"a"), b"one")]);
        let partial = send(&log, vec![a(), record(b"x", Some(b"x"), b"")]);
        let wrong = send(&log, vec![record(b"s", None, b"")]);
        let durable = send(&log, vec![boot()]);
        let deferred = send(&log, vec![b()]);
        let lookup = sim.hold(Call::new(
            SEG,
            Purpose::Lookup,
            Op::Read(64, raw::bytes(&sim).len() - 64),
        ));
        held.release(None);
        bootstrap.wait().unwrap();
        lookup.wait();
        let answers = (conflict.probe(), partial.probe(), wrong.probe());
        let waiting = original.probe().is_none() && retry.probe().is_none();
        let next = sim.hold(flush());
        lookup.release(None);
        next.wait();
        let durable_answer = durable.probe();
        next.release(Some(Fault::NoSpace));
        assert!(waiting);
        assert!(matches!(
            answers.0,
            Some(Err(CommitError::KeyConflict { index: 0 }))
        ));
        assert!(matches!(answers.1, Some(Err(CommitError::PartialRetry))));
        assert!(matches!(
            answers.2,
            Some(Err(CommitError::WrongRevision {
                actual: Some(0),
                ..
            }))
        ));
        positions(durable_answer.unwrap().unwrap(), &[(0, 0)], true);
        for p in [original, retry] {
            assert!(matches!(p.wait(), Err(CommitError::Indeterminate)));
        }
        assert!(matches!(deferred.wait(), Err(CommitError::Poisoned)));
        assert_eq!(log.watermark(), 1);
        assert!(log.stream_head(b"s").is_none());
        assert_records(&log.read(0, 10).unwrap(), &history()[..1]);
        calls(&sim, 2, 2);
    });
}

/// Given queued originals and matched retries behind a held bootstrap flush
/// When the log is dropped while that group drains
/// Then all result handles outlive the log and receive the original durable positions
/// And reopen rebuilds only the original records and keys
/// Cases: DROP, RETRY-PENDING, RETRY-REGROUPED
#[test]
fn dropping_the_log_drains_originals_and_matched_waiters() {
    bounded(|| {
        let sim = Sim::new(511);
        let log = open(&sim);
        let held = sim.hold(flush());
        let bootstrap = send(&log, vec![boot()]);
        held.wait();
        let first = send(&log, vec![a(), b()]);
        let second = send(&log, vec![c()]);
        let exact = send(&log, vec![a(), b()]);
        let mixed = send(&log, vec![c(), boot(), a()]);
        let next = sim.hold(flush());
        let dropper = std::thread::spawn(move || drop(log));
        held.release(None);
        bootstrap.wait().unwrap();
        next.wait();
        let waiting = [&first, &second, &exact, &mixed]
            .iter()
            .all(|p| p.probe().is_none());
        next.release(None);
        assert!(waiting);
        positions(first.wait().unwrap(), &[(1, 0), (2, 1)], false);
        positions(second.wait().unwrap(), &[(3, 2)], false);
        positions(exact.wait().unwrap(), &[(1, 0), (2, 1)], true);
        positions(mixed.wait().unwrap(), &[(3, 2), (0, 0), (1, 0)], true);
        dropper.join().unwrap();
        let log = open(&sim);
        assert_records(&log.read(0, 10).unwrap(), &history());
        assert_eq!(log.watermark(), 4);
        positions(
            send(&log, vec![c(), a()]).wait().unwrap(),
            &[(3, 2), (1, 0)],
            true,
        );
        calls(&sim, 2, 2);
    });
}
