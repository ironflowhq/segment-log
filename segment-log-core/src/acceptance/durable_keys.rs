use super::support::{bounded, fixtures::*, raw};
use crate::storage::sim::{Crash, Fault};
use crate::storage::{Call, Op, Purpose, Storage};
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
fn original() -> NewRecord {
    record(b"s", Some(b"k"), b"original")
}
fn stored(stream: &[u8], key: Option<&[u8]>, payload: &[u8], offset: u64, revision: u64) -> Record {
    Record {
        stream: stream.to_vec(),
        key: key.map(<[u8]>::to_vec),
        payload: payload.to_vec(),
        offset,
        revision,
    }
}
fn original_history() -> Vec<Record> {
    vec![stored(b"s", Some(b"k"), b"original", 0, 0)]
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
fn calls(sim: &Sim, count: usize) {
    assert_eq!(
        sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
        count
    );
    assert_eq!(
        sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
        count
    );
}
fn lookup(sim: &Sim) -> Call {
    let bytes = raw::bytes(sim);
    let len = u32::from_le_bytes(bytes[64..68].try_into().unwrap()) as usize + 8;
    Call::new(SEG, Purpose::Lookup, Op::Read(64, len))
}

/// Given a durable key on stream s
/// When its payload or stream differs, including after a fresh or matching record
/// Then the first conflicting input wins over stale expectations and partial retry
/// And nothing changes until the next valid append uses the still-unused position and key
/// Cases: KEY-SCOPE, KEY-CONFLICT, KEY-PARTIAL, REV-NO-OFFSET
#[test]
fn conflicts_use_input_order_and_leave_no_changes() {
    bounded(|| {
        for reopen in [false, true] {
            for stream_conflict in [false, true] {
                for prefix in [0, 1, 2] {
                    let sim = Sim::new(401);
                    let mut log = open(&sim);
                    send(&log, vec![original(), record(b"t", Some(b"t"), b"t")])
                        .wait()
                        .unwrap();
                    if reopen {
                        drop(log);
                        log = open(&sim);
                    }
                    let history = vec![
                        stored(b"s", Some(b"k"), b"original", 0, 0),
                        stored(b"t", Some(b"t"), b"t", 1, 0),
                    ];
                    let mut records = Vec::new();
                    if prefix > 0 {
                        records.push(record(b"fresh", Some(b"fresh"), b"new"));
                    }
                    if prefix > 1 {
                        records.push(record(b"t", Some(b"t"), b"t"));
                    }
                    records.push(if stream_conflict {
                        record(b"other", Some(b"k"), b"original")
                    } else {
                        record(b"s", Some(b"k"), b"changed")
                    });
                    records.push(record(
                        b"t",
                        if prefix < 2 {
                            Some(b"t")
                        } else {
                            Some(b"t-conflict-fresh")
                        },
                        b"ignored",
                    ));
                    let error = send(&log, records).wait().unwrap_err();
                    assert!(
                        matches!(error, CommitError::KeyConflict { index } if index == prefix),
                        "{error:?}"
                    );
                    calls(&sim, 1);
                    assert_records(&log.read(0, 10).unwrap(), &history);
                    assert_eq!(log.watermark(), 2);
                    assert!(log.stream_head(b"fresh").is_none());
                    positions(
                        send(&log, vec![record(b"fresh", Some(b"fresh"), b"new")])
                            .wait()
                            .unwrap(),
                        &[(2, 0)],
                        false,
                    );
                }
            }
        }
    });
}

/// Given a durable original and a fresh keyed or unkeyed record
/// When the records are submitted in either order with stale expectations
/// Then PartialRetry writes nothing and leaves the original and fresh key unchanged
/// Cases: KEY-PARTIAL
#[test]
fn matching_and_fresh_records_reject_in_either_order() {
    bounded(|| {
        for keyed in [false, true] {
            for reverse in [false, true] {
                let sim = Sim::new(402);
                let log = open(&sim);
                send(&log, vec![original()]).wait().unwrap();
                let history = original_history();
                let mut records = vec![
                    original(),
                    record(b"fresh", keyed.then_some(b"fresh"), b"new"),
                ];
                if reverse {
                    records.reverse();
                }
                let error = send(&log, records).wait().unwrap_err();
                assert!(matches!(error, CommitError::PartialRetry), "{error:?}");
                calls(&sim, 1);
                assert_records(&log.read(0, 10).unwrap(), &history);
                positions(
                    send(&log, vec![record(b"fresh", Some(b"fresh"), b"new")])
                        .wait()
                        .unwrap(),
                    &[(1, 0)],
                    false,
                );
            }
        }
    });
}

/// Given keyed records committed across two submits with successive stream revisions
/// When subsets, reordered records and regrouped records retry before and after reopen
/// Then stale expectations are ignored and original positions return in request order
/// And the full history survives with no extra append or flush
/// Cases: RETRY-REGROUPED, KEY-LIFETIME, HISTORY, ORACLE-12, ORACLE-4
#[test]
fn durable_subsets_reorder_and_regroup_after_reopen() {
    bounded(|| {
        let sim = Sim::new(403);
        let mut log = open(&sim);
        send(&log, vec![original(), record(b"t", Some(b"t"), b"two")])
            .wait()
            .unwrap();
        let mut third = record(b"s", Some(b"three"), b"third");
        third.expected = ExpectedRevision::At(0);
        send(&log, vec![third]).wait().unwrap();
        let history = vec![
            stored(b"s", Some(b"k"), b"original", 0, 0),
            stored(b"t", Some(b"t"), b"two", 1, 0),
            stored(b"s", Some(b"three"), b"third", 2, 1),
        ];
        for reopen in [false, true] {
            if reopen {
                drop(log);
                log = open(&sim);
            }
            positions(
                send(&log, vec![record(b"t", Some(b"t"), b"two")])
                    .wait()
                    .unwrap(),
                &[(1, 0)],
                true,
            );
            positions(
                send(
                    &log,
                    vec![
                        record(b"s", Some(b"three"), b"third"),
                        original(),
                        record(b"t", Some(b"t"), b"two"),
                    ],
                )
                .wait()
                .unwrap(),
                &[(2, 1), (0, 0), (1, 0)],
                true,
            );
            calls(&sim, 2);
            assert_records(&log.read(0, 10).unwrap(), &history);
            assert_eq!(log.watermark(), 3);
            assert_eq!(log.stream_head(b"s").unwrap().revision, 1);
        }
    });
}

/// Given an acknowledged keyed record
/// When Kill, PowerLoss or PowerLossTorn is followed by reopen and retry
/// Then the exact original bytes and position survive without a new group
/// And a second power loss and reopen rebuild the same key index
/// Cases: ORACLE-1, ORACLE-4, ORACLE-12, KEY-LIFETIME, HISTORY
#[test]
fn acknowledged_keys_survive_each_crash_and_reboot() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            let sim = Sim::new(404);
            let log = open(&sim);
            send(&log, vec![original()]).wait().unwrap();
            let history = original_history();
            drop(log);
            sim.crash(mode, 19);
            for _ in 0..2 {
                let log = open(&sim);
                positions(
                    send(&log, vec![original()]).wait().unwrap(),
                    &[(0, 0)],
                    true,
                );
                assert_records(&log.read(0, 10).unwrap(), &history);
                calls(&sim, 1);
                drop(log);
                sim.crash(Crash::PowerLoss, 27);
            }
        }
    });
}

/// Given a durable original whose frame lookup errors, ends early or has bad CRC
/// When a fully matching retry is classified
/// Then it is Indeterminate and later work is Poisoned with no new write or flush
/// Cases: POISON-LOOKUP, ORACLE-7
#[test]
fn lookup_faults_poison_instead_of_rejecting_or_matching() {
    bounded(|| {
        for fault in [Fault::NoSpace, Fault::Eof, Fault::Corrupt(8, 1)] {
            let sim = Sim::new(405);
            let log = open(&sim);
            send(&log, vec![original()]).wait().unwrap();
            sim.inject(lookup(&sim), fault);
            let answer = send(&log, vec![original()]).wait();
            assert!(
                matches!(answer, Err(CommitError::Indeterminate)),
                "{answer:?}"
            );
            assert!(matches!(
                log.append(Submit {
                    records: vec![record(b"later", None, b"")]
                }),
                Err(AppendError::Poisoned)
            ));
            assert_eq!(log.watermark(), 1);
            calls(&sim, 1);
        }
    });
}

/// Given a durable original whose frame is now malformed but has a valid CRC
/// When a retry looks up that frame
/// Then validation poisons the writer rather than trusting cached bytes
/// Cases: POISON-LOOKUP, ORACLE-7
#[test]
fn malformed_lookup_frame_is_not_a_cached_success() {
    bounded(|| {
        let sim = Sim::new(406);
        let log = open(&sim);
        send(&log, vec![original()]).wait().unwrap();
        let mut bytes = raw::bytes(&sim)[64..].to_vec();
        bytes[9] = 1;
        let input = [&bytes[..4], &bytes[8..]].concat();
        bytes[4..8].copy_from_slice(&raw::crc(&input).to_le_bytes());
        sim.call(Call::new(SEG, Purpose::Open, Op::Write(64, bytes)))
            .unwrap();
        let answer = send(&log, vec![original()]).wait();
        assert!(
            matches!(answer, Err(CommitError::Indeterminate)),
            "{answer:?}"
        );
        calls(&sim, 1);
    });
}

/// Given a held bootstrap flush and queued new work followed by terminal classifications
/// When the next new group's flush is held
/// Then key rejections, revision rejection and durable retry are already finalized
/// And a later failed flush cannot change those answers
/// Cases: ANSWER-TIMING, KEY-CONFLICT, KEY-PARTIAL, ORACLE-7
#[test]
fn terminal_answers_do_not_wait_for_a_new_flush() {
    bounded(|| {
        let sim = Sim::new(407);
        let log = open(&sim);
        let bootstrap_hold = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
        let bootstrap = send(&log, vec![original()]);
        bootstrap_hold.wait();
        let new = send(&log, vec![record(b"new", None, b"")]);
        let conflict = send(&log, vec![record(b"s", Some(b"k"), b"different")]);
        let partial = send(&log, vec![original(), record(b"fresh", None, b"")]);
        let wrong = send(&log, vec![record(b"new", None, b"")]);
        let retry = send(&log, vec![original()]);
        let hold = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
        bootstrap_hold.release(None);
        bootstrap.wait().unwrap();
        hold.wait();
        let answers = (
            conflict.probe(),
            partial.probe(),
            wrong.probe(),
            retry.probe(),
        );
        let new_unanswered = new.probe().is_none();
        hold.release(Some(Fault::NoSpace));
        assert!(new_unanswered);
        assert!(
            matches!(answers.0, Some(Err(CommitError::KeyConflict { index: 0 }))),
            "{:?}",
            answers.0
        );
        assert!(
            matches!(answers.1, Some(Err(CommitError::PartialRetry))),
            "{:?}",
            answers.1
        );
        assert!(matches!(
            answers.2,
            Some(Err(CommitError::WrongRevision {
                index: 0,
                actual: Some(0)
            }))
        ));
        positions(answers.3.unwrap().unwrap(), &[(0, 0)], true);
        assert!(matches!(new.wait(), Err(CommitError::Indeterminate)));
        assert_eq!(log.watermark(), 1);
    });
}

/// Given buffered new work and finalized answers behind a successful bootstrap flush
/// When lookup of that bootstrap record fails before the new group is written
/// Then all unfinished current requests are Indeterminate, queued work is Poisoned
/// And earlier answers stay fixed, with no later write or flush
/// Cases: POISON-LOOKUP, ANSWER-TIMING, ORACLE-7
#[test]
fn lookup_failure_aborts_buffered_work_but_preserves_finalized_answers() {
    bounded(|| {
        for fault in [Fault::NoSpace, Fault::Eof, Fault::Corrupt(8, 1)] {
            let sim = Sim::new(408);
            let log = open(&sim);
            send(&log, vec![original()]).wait().unwrap();
            let held_flush = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
            let bootstrap = send(&log, vec![record(b"b", Some(b"b"), b"bootstrap")]);
            held_flush.wait();
            let (segment, position, frame) = raw::last_frame(&sim);
            let new1 = send(&log, vec![record(b"new1", None, b"a")]);
            let new2 = send(&log, vec![record(b"new2", None, b"b")]);
            let conflict = send(&log, vec![record(b"s", Some(b"k"), b"different")]);
            let partial = send(&log, vec![original(), record(b"fresh", None, b"")]);
            let wrong = send(&log, vec![record(b"new1", None, b"")]);
            let retry = send(&log, vec![original()]);
            let failing = send(&log, vec![record(b"b", Some(b"b"), b"bootstrap")]);
            let held_lookup = sim.hold(Call::new(
                segment,
                Purpose::Lookup,
                Op::Read(position, frame.len()),
            ));
            held_flush.release(None);
            bootstrap.wait().unwrap();
            held_lookup.wait();
            let queued = send(&log, vec![record(b"queued", None, b"")]);
            let answers = (
                conflict.probe(),
                partial.probe(),
                wrong.probe(),
                retry.probe(),
            );
            let unfinished =
                new1.probe().is_none() && new2.probe().is_none() && failing.probe().is_none();
            held_lookup.release(Some(fault));
            assert!(unfinished);
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
            positions(answers.3.unwrap().unwrap(), &[(0, 0)], true);
            for pending in [new1, new2, failing] {
                assert!(matches!(pending.wait(), Err(CommitError::Indeterminate)));
            }
            assert!(matches!(queued.wait(), Err(CommitError::Poisoned)));
            assert_eq!(log.watermark(), 2);
            assert!(log.stream_head(b"new1").is_none());
            assert!(log.stream_head(b"new2").is_none());
            calls(&sim, 2);
            let history = vec![
                stored(b"s", Some(b"k"), b"original", 0, 0),
                stored(b"b", Some(b"b"), b"bootstrap", 1, 0),
            ];
            drop(log);
            for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
                sim.crash(mode, 29);
                let log = open(&sim);
                assert_records(&log.read(0, 10).unwrap(), &history);
                positions(
                    send(&log, vec![original()]).wait().unwrap(),
                    &[(0, 0)],
                    true,
                );
                drop(log);
            }
        }
    });
}

/// Given a cap that fits one new record and queued durable retries and rejections
/// When the next new request exceeds the current group's remaining cap
/// Then only new bytes count and nothing behind the deferred request overtakes it
/// Cases: GROUP-FORMATION, RETRY-REGROUPED, ANSWER-TIMING
#[test]
fn retry_bytes_do_not_fill_groups_or_overtake_deferred_new_work() {
    bounded(|| {
        let sim = Sim::new(409);
        let log = SegmentLog::with_storage(
            Path::new("/log"),
            Options {
                group_byte_cap: 50,
                ..options()
            },
            std::sync::Arc::new(sim.clone()),
        )
        .unwrap();
        let held = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
        let bootstrap = send(&log, vec![original()]);
        held.wait();
        let first = send(&log, vec![record(b"a", None, b"12345678")]);
        let retry = send(&log, vec![original()]);
        let conflict = send(&log, vec![record(b"x", Some(b"k"), b"original")]);
        let deferred = send(&log, vec![record(b"b", None, b"12345678")]);
        let behind = send(&log, vec![original()]);
        let next_flush = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
        held.release(None);
        bootstrap.wait().unwrap();
        next_flush.wait();
        let answer = retry.probe();
        let rejection = conflict.probe();
        let unanswered =
            first.probe().is_none() && deferred.probe().is_none() && behind.probe().is_none();
        next_flush.release(None);
        positions(answer.unwrap().unwrap(), &[(0, 0)], true);
        assert!(matches!(
            rejection,
            Some(Err(CommitError::KeyConflict { index: 0 }))
        ));
        assert!(unanswered);
        positions(first.wait().unwrap(), &[(1, 0)], false);
        positions(deferred.wait().unwrap(), &[(2, 0)], false);
        positions(behind.wait().unwrap(), &[(0, 0)], true);
        calls(&sim, 3);
    });
}

/// Given a durable original and a byte budget that fits only one large request
/// When a durable success or terminal rejection finishes while a new flush is held
/// Then its reservation is released before its Pending is consumed
/// And an oversized retry still fails admission without a lookup
/// Cases: ADM-RESERVATION, ADM-SIZE, ANSWER-TIMING
#[test]
fn terminal_classification_releases_capacity_before_wait() {
    bounded(|| {
        for variant in 0..4 {
            let sim = Sim::new(410);
            let log = SegmentLog::with_storage(
                Path::new("/log"),
                Options {
                    pending_byte_budget: 4096,
                    ..options()
                },
                std::sync::Arc::new(sim.clone()),
            )
            .unwrap();
            send(&log, vec![original()]).wait().unwrap();
            let held = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
            let bootstrap = send(&log, vec![record(b"b", None, b"")]);
            held.wait();
            let new = send(&log, vec![record(b"new", None, b"")]);
            let mut records = match variant {
                0 => vec![original()],
                1 => vec![record(b"s", Some(b"k"), b"changed")],
                2 => vec![original(), record(b"fresh", None, b"")],
                _ => vec![record(b"s", None, b"original")],
            };
            let payload = records[0].payload.clone();
            records[0].payload = Vec::with_capacity(2500);
            records[0].payload.extend(payload);
            let terminal = send(&log, records);
            let large = |capacity| {
                let mut r = original();
                r.payload = Vec::with_capacity(capacity);
                r.payload.extend(b"original");
                Submit { records: vec![r] }
            };
            assert!(matches!(log.append(large(2500)), Err(AppendError::Full)));
            let next_flush = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
            held.release(None);
            bootstrap.wait().unwrap();
            next_flush.wait();
            let admitted = log.append(large(2500));
            let too_large = log.append(large(5000));
            next_flush.release(None);
            assert!(matches!(too_large, Err(AppendError::TooLarge)));
            let admitted = admitted.expect("terminal answer must release its reservation");
            match variant {
                0 => positions(terminal.wait().unwrap(), &[(0, 0)], true),
                1 => assert!(matches!(
                    terminal.wait(),
                    Err(CommitError::KeyConflict { index: 0 })
                )),
                2 => assert!(matches!(terminal.wait(), Err(CommitError::PartialRetry))),
                _ => assert!(matches!(
                    terminal.wait(),
                    Err(CommitError::WrongRevision { .. })
                )),
            }
            new.wait().unwrap();
            positions(admitted.wait().unwrap(), &[(0, 0)], true);
            calls(&sim, 3);
        }
    });
}

/// Given durable records whose combined retry exceeds the new standalone group cap
/// When the log reopens with that cap and the caller retries them together
/// Then TooLarge still precedes key classification and individual retries succeed
/// Cases: ADM-SIZE, RETRY-REGROUPED
#[test]
fn regrouped_retries_still_obey_the_admission_group_cap() {
    bounded(|| {
        let sim = Sim::new(411);
        let log = open(&sim);
        send(&log, vec![original(), record(b"t", Some(b"t"), b"two")])
            .wait()
            .unwrap();
        drop(log);
        let log = SegmentLog::with_storage(
            Path::new("/log"),
            Options {
                group_byte_cap: 50,
                ..options()
            },
            std::sync::Arc::new(sim.clone()),
        )
        .unwrap();
        assert!(matches!(
            log.append(Submit {
                records: vec![original(), record(b"t", Some(b"t"), b"two")]
            }),
            Err(AppendError::TooLarge)
        ));
        positions(
            send(&log, vec![original()]).wait().unwrap(),
            &[(0, 0)],
            true,
        );
        calls(&sim, 1);
    });
}

/// Given no matching keys, or repeated occurrences of a durable key in one submit
/// When fresh keyed and unkeyed records are submitted or a durable key is repeated
/// Then ordinary revisions apply to fresh inputs and repeated keys fail before admission
/// Cases: KEY-REPEATED, KEY-PARTIAL, REV-EXPECTED
#[test]
fn fresh_mixes_use_revisions_and_repeated_durable_keys_remain_invalid() {
    bounded(|| {
        let sim = Sim::new(412);
        let log = open(&sim);
        send(&log, vec![original()]).wait().unwrap();
        for changed in [false, true] {
            let other = if changed {
                record(b"other", Some(b"k"), b"changed")
            } else {
                original()
            };
            assert!(matches!(
                log.append(Submit {
                    records: vec![original(), other]
                }),
                Err(AppendError::Invalid(_))
            ));
        }
        let mut bad = record(b"s", Some(b"fresh"), b"new");
        bad.expected = ExpectedRevision::Empty;
        assert!(matches!(
            send(&log, vec![record(b"t", None, b"two"), bad]).wait(),
            Err(CommitError::WrongRevision {
                index: 1,
                actual: Some(0)
            })
        ));
        let mut good = record(b"s", Some(b"fresh"), b"new");
        good.expected = ExpectedRevision::At(0);
        positions(
            send(&log, vec![record(b"t", None, b"two"), good])
                .wait()
                .unwrap(),
            &[(1, 0), (2, 1)],
            false,
        );
        calls(&sim, 2);
    });
}

/// Given a durable key and a stream or watermark at its last representable value
/// When the key retries, conflicts or mixes with fresh records
/// Then key classification precedes exhaustion and retries advance no counters
/// Cases: COUNTER-EXHAUSTION, KEY-CONFLICT, KEY-PARTIAL
#[test]
fn durable_classification_precedes_counter_exhaustion() {
    bounded(|| {
        for offset_limit in [false, true] {
            let sim = Sim::new(413);
            let log = open(&sim);
            log.seed_counters(
                if offset_limit { u64::MAX - 1 } else { 0 },
                if offset_limit {
                    &[]
                } else {
                    &[(b"s", u64::MAX - 1)]
                },
            );
            let make = || {
                let mut r = original();
                if !offset_limit {
                    r.expected = ExpectedRevision::At(u64::MAX - 1);
                }
                r
            };
            let expected = if offset_limit {
                (u64::MAX - 1, 0)
            } else {
                (0, u64::MAX)
            };
            positions(send(&log, vec![make()]).wait().unwrap(), &[expected], false);
            positions(
                send(&log, vec![original()]).wait().unwrap(),
                &[expected],
                true,
            );
            assert!(matches!(
                send(&log, vec![record(b"s", Some(b"k"), b"changed")]).wait(),
                Err(CommitError::KeyConflict { index: 0 })
            ));
            assert!(matches!(
                send(&log, vec![original(), record(b"fresh", None, b"")]).wait(),
                Err(CommitError::PartialRetry)
            ));
            calls(&sim, 1);
        }
    });
}

/// Given an unknown fully keyed outcome at a held flush
/// When each crash mode reopens the log and the exact request retries
/// Then surviving bytes deduplicate and an absent key follows ordinary revisions
/// And a second power loss preserves the resolved record exactly
/// Cases: ORACLE-4, RETRY-UNKNOWN-1, RETRY-UNKNOWN-2, KEY-LIFETIME
#[test]
fn unknown_keyed_outcomes_resolve_from_the_recovered_prefix() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            for seed in [1, 2, 3, 4] {
                let sim = Sim::new(414);
                let log = open(&sim);
                let held = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
                let pending = send(&log, vec![original()]);
                held.wait();
                assert!(pending.probe().is_none());
                sim.crash(mode, seed);
                held.release(None);
                assert!(matches!(pending.wait(), Err(CommitError::Indeterminate)));
                drop(log);
                let log = open(&sim);
                let survived = log.watermark() == 1;
                assert!(log.watermark() <= 1);
                positions(
                    send(&log, vec![original()]).wait().unwrap(),
                    &[(0, 0)],
                    survived,
                );
                let history = original_history();
                assert_records(&log.read(0, 10).unwrap(), &history);
                drop(log);
                sim.crash(Crash::PowerLoss, 5);
                let log = open(&sim);
                assert_records(&log.read(0, 10).unwrap(), &history);
                positions(
                    send(&log, vec![original()]).wait().unwrap(),
                    &[(0, 0)],
                    true,
                );
            }
        }
    });
}

/// Given a durable original and a second keyed frame torn during append
/// When Kill recovery truncates the invalid suffix
/// Then only kept-frame keys deduplicate and the discarded key is new at offset one
/// Cases: ORACLE-4, RECOVERY-FIXTURES, KEY-LIFETIME
#[test]
fn recovery_does_not_index_keys_from_a_discarded_frame() {
    bounded(|| {
        let sim = Sim::new(415);
        let log = open(&sim);
        send(&log, vec![original()]).wait().unwrap();
        let bytes = crate::codec::encode(&crate::codec::Group {
            epoch: 1,
            base_offset: 1,
            entries: vec![crate::codec::Entry {
                stream: b"t",
                key: Some(b"t"),
                payload: b"two",
                revision: 0,
            }],
        })
        .unwrap();
        sim.inject(
            Call::new(ANY_SEG, Purpose::Group, Op::Append(bytes)),
            Fault::Partial(12),
        );
        assert!(matches!(
            send(&log, vec![record(b"t", Some(b"t"), b"two")]).wait(),
            Err(CommitError::Indeterminate)
        ));
        drop(log);
        sim.crash(Crash::Kill, 1);
        let log = open(&sim);
        positions(
            send(&log, vec![original()]).wait().unwrap(),
            &[(0, 0)],
            true,
        );
        positions(
            send(&log, vec![record(b"t", Some(b"t"), b"two")])
                .wait()
                .unwrap(),
            &[(1, 0)],
            false,
        );
    });
}

/// Given a durable key whose stored stream or payload changes in a valid frame
/// When the old input retries
/// Then the writer compares the actual stored bytes and reports KeyConflict
/// Cases: KEY-SCOPE, KEY-CONFLICT, POISON-LOOKUP
#[test]
fn lookup_compares_stored_stream_and_payload_not_cached_contents() {
    bounded(|| {
        for change_stream in [false, true] {
            let sim = Sim::new(416);
            let log = open(&sim);
            send(&log, vec![original()]).wait().unwrap();
            let bytes = crate::codec::encode(&crate::codec::Group {
                epoch: 1,
                base_offset: 0,
                entries: vec![crate::codec::Entry {
                    stream: if change_stream { b"t" } else { b"s" },
                    key: Some(b"k"),
                    payload: if change_stream {
                        b"original"
                    } else {
                        b"modified"
                    },
                    revision: 0,
                }],
            })
            .unwrap();
            sim.call(Call::new(SEG, Purpose::Open, Op::Write(64, bytes)))
                .unwrap();
            assert!(matches!(
                send(&log, vec![original()]).wait(),
                Err(CommitError::KeyConflict { index: 0 })
            ));
            calls(&sim, 1);
        }
    });
}

/// Given an existing durable key and a linger longer than the scenario watchdog
/// When retries and terminal rejections arrive without new records
/// Then each answers without waiting for linger and the writer makes no new group
/// Cases: ANSWER-TIMING, GROUP-FORMATION
#[test]
fn durable_only_classification_does_not_wait_for_linger() {
    bounded(|| {
        let sim = Sim::new(417);
        let log = open(&sim);
        send(&log, vec![original()]).wait().unwrap();
        drop(log);
        let log = SegmentLog::with_storage(
            Path::new("/log"),
            Options {
                linger: std::time::Duration::from_secs(60),
                ..options()
            },
            std::sync::Arc::new(sim.clone()),
        )
        .unwrap();
        positions(
            send(&log, vec![original()]).wait().unwrap(),
            &[(0, 0)],
            true,
        );
        assert!(matches!(
            send(&log, vec![record(b"s", Some(b"k"), b"changed")]).wait(),
            Err(CommitError::KeyConflict { .. })
        ));
        assert!(matches!(
            send(&log, vec![original(), record(b"fresh", None, b"")]).wait(),
            Err(CommitError::PartialRetry)
        ));
        assert!(matches!(
            send(&log, vec![record(b"s", None, b"")]).wait(),
            Err(CommitError::WrongRevision { .. })
        ));
        calls(&sim, 1);
    });
}

/// Given new work buffered behind a bootstrap flush and a queued durable-key lookup
/// When the original frame becomes malformed with a valid CRC before lookup runs
/// Then both buffered work and the lookup are Indeterminate and queued work is Poisoned
/// And the new group is never written or flushed
/// Cases: POISON-LOOKUP, ORACLE-7
#[test]
fn malformed_lookup_also_aborts_the_unwritten_group() {
    bounded(|| {
        let sim = Sim::new(418);
        let log = open(&sim);
        send(&log, vec![original()]).wait().unwrap();
        let lookup_call = lookup(&sim);
        let held = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
        let bootstrap = send(&log, vec![record(b"b", None, b"")]);
        held.wait();
        let new = send(&log, vec![record(b"new", None, b"")]);
        let retry = send(&log, vec![original()]);
        let held_lookup = sim.hold(lookup_call);
        held.release(None);
        bootstrap.wait().unwrap();
        held_lookup.wait();
        let queued = send(&log, vec![record(b"queued", None, b"")]);
        let all_bytes = raw::bytes(&sim);
        let len = u32::from_le_bytes(all_bytes[64..68].try_into().unwrap()) as usize + 8;
        let mut frame = all_bytes[64..64 + len].to_vec();
        frame[9] = 1;
        let input = [&frame[..4], &frame[8..]].concat();
        frame[4..8].copy_from_slice(&raw::crc(&input).to_le_bytes());
        sim.call(Call::new(SEG, Purpose::Open, Op::Write(64, frame)))
            .unwrap();
        held_lookup.release(None);
        assert!(matches!(new.wait(), Err(CommitError::Indeterminate)));
        assert!(matches!(retry.wait(), Err(CommitError::Indeterminate)));
        assert!(matches!(queued.wait(), Err(CommitError::Poisoned)));
        assert_eq!(log.watermark(), 2);
        calls(&sim, 2);
    });
}

/// Given one new group at its byte cap, a deferred request and a retry behind it
/// When that group's flush fails
/// Then the new group is Indeterminate and both unprocessed requests are Poisoned
/// And already finalized durable retries keep their original positions
/// Cases: ORACLE-7, ANSWER-TIMING, GROUP-FORMATION
#[test]
fn a_failed_group_poisons_deferred_work_without_overtaking() {
    bounded(|| {
        let sim = Sim::new(419);
        let log = SegmentLog::with_storage(
            Path::new("/log"),
            Options {
                group_byte_cap: 50,
                ..options()
            },
            std::sync::Arc::new(sim.clone()),
        )
        .unwrap();
        let held = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
        let bootstrap = send(&log, vec![original()]);
        held.wait();
        let new = send(&log, vec![record(b"a", None, b"12345678")]);
        let retry = send(&log, vec![original()]);
        let deferred = send(&log, vec![record(b"b", None, b"12345678")]);
        let queued = send(&log, vec![original()]);
        let next_flush = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
        held.release(None);
        bootstrap.wait().unwrap();
        next_flush.wait();
        let answer = retry.probe();
        next_flush.release(Some(Fault::NoSpace));
        positions(answer.unwrap().unwrap(), &[(0, 0)], true);
        assert!(matches!(new.wait(), Err(CommitError::Indeterminate)));
        assert!(matches!(deferred.wait(), Err(CommitError::Poisoned)));
        assert!(matches!(queued.wait(), Err(CommitError::Poisoned)));
        assert_eq!(log.watermark(), 1);
        calls(&sim, 2);
    });
}

/// Given an indexed frame replaced with a shorter but checksum-valid frame
/// When a retry reads fewer bytes than that original frame's indexed length
/// Then the unexpected EOF is Indeterminate even if the retried entry is unchanged
/// And subsequent requests are Poisoned without another write or flush
/// Cases: POISON-LOOKUP, ORACLE-7
#[test]
fn a_short_valid_lookup_frame_cannot_ack_or_reject_a_retry() {
    bounded(|| {
        for unchanged_target in [true, false] {
            let sim = Sim::new(420);
            let log = open(&sim);
            let mut records = vec![original()];
            if unchanged_target {
                records.push(record(b"t", Some(b"tail"), b"longtail"));
            }
            send(&log, records).wait().unwrap();
            let mut bytes = raw::bytes(&sim)[64..].to_vec();
            let payload_length = if unchanged_target {
                32 + 16 + 1 + 1 + 8 + 4
            } else {
                36
            };
            bytes[payload_length..payload_length + 4].copy_from_slice(&5u32.to_le_bytes());
            bytes.truncate(bytes.len() - 3);
            let body_length = (bytes.len() - 8) as u32;
            bytes[..4].copy_from_slice(&body_length.to_le_bytes());
            let checksum = raw::crc(&[&bytes[..4], &bytes[8..]].concat());
            bytes[4..8].copy_from_slice(&checksum.to_le_bytes());
            let end = 64 + bytes.len() as u64;
            sim.call(Call::new(SEG, Purpose::Open, Op::Write(64, bytes)))
                .unwrap();
            sim.call(Call::new(SEG, Purpose::Open, Op::Truncate(end)))
                .unwrap();
            let answer = send(&log, vec![original()]).wait();
            assert!(
                matches!(answer, Err(CommitError::Indeterminate)),
                "short lookup returned {answer:?}"
            );
            assert!(matches!(
                log.append(Submit {
                    records: vec![record(b"later", None, b"")]
                }),
                Err(AppendError::Poisoned)
            ));
            calls(&sim, 1);
        }
    });
}

fn numbered(offset: u64, payload: &str) -> NewRecord {
    record(
        format!("s{offset}").as_bytes(),
        Some(format!("k{offset}").as_bytes()),
        format!("{payload}{offset}").as_bytes(),
    )
}

/// Given durable groups of one, three, two and five keyed records
/// When one submit retries keys from the first, middle and last entries of several groups
/// And another reuses a middle group's key with a different payload
/// Then the retry returns the original positions in request order without a write
/// And the reuse gets `KeyConflict` at its own index
/// When a fifth group of four records is appended and each of its storage boundaries is
/// replayed through every crash mode
/// Then each key of a surviving group still returns its original position after reopen
/// And the records of a lost fifth group append as new at the next unused offsets
/// Cases: KEY-LIFETIME, RETRY-REGROUPED, KEY-CONFLICT, ORACLE-1, ORACLE-4, ORACLE-12
#[test]
fn keys_resolve_to_their_own_frames_across_groups_and_crashes() {
    bounded(|| {
        let sim = Sim::new(733);
        let log = open(&sim);
        let group = |log: &SegmentLog, offsets: std::ops::Range<u64>| {
            let expected: Vec<_> = offsets.clone().map(|offset| (offset, 0)).collect();
            let records = offsets.map(|offset| numbered(offset, "p")).collect();
            positions(send(log, records).wait().unwrap(), &expected, false);
        };
        for offsets in [0..1, 1..4, 4..6, 6..11] {
            group(&log, offsets);
        }
        let retry = |log: &SegmentLog, offsets: &[u64]| {
            let expected: Vec<_> = offsets.iter().map(|offset| (*offset, 0)).collect();
            let records = offsets
                .iter()
                .map(|offset| numbered(*offset, "p"))
                .collect();
            positions(send(log, records).wait().unwrap(), &expected, true);
        };
        retry(&log, &[10, 0, 4, 3, 6, 1, 5]);
        let mut reused = numbered(5, "p");
        reused.payload = b"other".to_vec();
        assert!(matches!(
            send(&log, vec![numbered(1, "p"), reused]).wait(),
            Err(CommitError::KeyConflict { index: 1 })
        ));
        calls(&sim, 4);
        let start = sim.trace().1.len();
        group(&log, 11..15);
        retry(&log, &[14, 11, 0, 12]);
        calls(&sim, 5);
        let history = log.read(0, 100).unwrap();
        assert_eq!(history.len(), 15);
        let (seed, trace) = sim.trace();
        drop(log);
        for end in start..=trace.len() {
            for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
                let replay = Sim::replay(seed, &trace[..end]);
                replay.crash(mode, end as u64);
                let recovered = open(&replay);
                let kept = recovered.watermark();
                assert!(kept == 11 || kept == 15, "end {end} kept {kept}");
                if end == trace.len() {
                    assert_eq!(kept, 15);
                }
                let writes = replay.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![]));
                for offset in 0..kept {
                    retry(&recovered, &[offset]);
                }
                retry(&recovered, &[kept - 1, 0, 5]);
                assert_eq!(
                    replay.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
                    writes
                );
                if kept == 11 {
                    group(&recovered, 11..15);
                }
                assert_records(&recovered.read(0, 100).unwrap(), &history);
            }
        }
    });
}
