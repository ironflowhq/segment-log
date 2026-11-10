use super::support::{bounded, fixtures::*};
use crate::storage;
use crate::storage::{Call, Op, Purpose};
use crate::*;
use std::path::Path;

/// Given an open log with a held group flush and another request queued behind it
/// When the flush fails
/// Then the in-flight request is indeterminate, queued and later requests are poisoned
/// And no record becomes visible and the failed handle receives no further writes or flushes
/// Cases: ORACLE-7, ORACLE-8
#[test]
fn failed_flush_hides_the_group_and_stops_the_writer() {
    bounded(|| {
        let sim = Sim::new(2);
        let log = open(&sim);
        let hold = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
        let pending = append(&log, b"s");
        hold.wait();
        let queued = append(&log, b"next");
        assert!(pending.probe().is_none());
        hold.release(Some(storage::sim::Fault::NoSpace));
        assert!(matches!(pending.wait(), Err(CommitError::Indeterminate)));
        assert!(matches!(queued.wait(), Err(CommitError::Poisoned)));
        assert!(matches!(
            log.append(submit(b"later")),
            Err(AppendError::Poisoned)
        ));
        assert!(log.read(0, 100).unwrap().is_empty());
        assert_eq!(log.watermark(), 0);
        assert!(log.stream_head(b"s").is_none());
        drop(log);
        assert_eq!(
            sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
            1
        );
        assert_eq!(
            sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
            1
        );
        sim.crash(storage::sim::Crash::Kill, 3);
        let reopened = open(&sim);
        assert_record(&reopened);
    });
}

/// Given a group write that fails after zero, some or all frame bytes
/// When the caller receives its result
/// Then it is indeterminate with no reader visibility and no attempted flush
/// When the process is killed and reopened
/// Then only a complete valid frame survives
/// Cases: ORACLE-7, ORACLE-8, ORACLE-2
#[test]
fn failed_writes_never_ack_and_recovery_keeps_only_complete_frames() {
    bounded(|| {
        for (fault, kept) in [
            (storage::sim::Fault::NoSpace, false),
            (storage::sim::Fault::Partial(12), false),
            (storage::sim::Fault::Partial(first_frame().len()), true),
        ] {
            let sim = Sim::new(11);
            let log = open(&sim);
            sim.inject(
                Call::new(ANY_SEG, Purpose::Group, Op::Append(first_frame())),
                fault,
            );
            assert!(matches!(
                append(&log, b"s").wait(),
                Err(CommitError::Indeterminate)
            ));
            assert!(log.read(0, 100).unwrap().is_empty());
            assert_eq!(log.watermark(), 0);
            assert!(log.stream_head(b"s").is_none());
            assert!(matches!(
                log.append(submit(b"later")),
                Err(AppendError::Poisoned)
            ));
            drop(log);
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
                0
            );
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
                1
            );
            sim.crash(storage::sim::Crash::Kill, 12);
            let reopened = open(&sim);
            if kept {
                assert_record(&reopened);
            } else {
                assert_eq!(reopened.watermark(), 0);
                assert!(reopened.read(0, 100).unwrap().is_empty());
            }
        }
    });
}

use super::support::raw;
use crate::storage::Storage;
use crate::storage::sim::{Crash, Fault};
use std::sync::Arc;

fn keyed(stream: u8, payload: &[u8]) -> NewRecord {
    NewRecord {
        stream: vec![stream],
        key: Some(vec![stream]),
        payload: payload.to_vec(),
        expected: ExpectedRevision::Empty,
    }
}
fn send(log: &SegmentLog, records: Vec<NewRecord>) -> Pending {
    log.append(Submit { records }).unwrap()
}
fn positions(answer: Committed, expected: &[(u64, u64)], deduplicated: bool) {
    assert_eq!(answer.deduplicated, deduplicated);
    assert_eq!(
        answer
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

fn frame(base: u64, records: &[NewRecord]) -> Vec<u8> {
    let mut body = vec![1, 0, 0, 0];
    body.extend(1u64.to_le_bytes());
    body.extend(base.to_le_bytes());
    body.extend((records.len() as u32).to_le_bytes());
    for r in records {
        let key = r.key.as_deref().unwrap();
        body.extend((r.stream.len() as u16).to_le_bytes());
        body.extend((key.len() as u16).to_le_bytes());
        body.extend((r.payload.len() as u32).to_le_bytes());
        body.extend(0u64.to_le_bytes());
        body.extend(&r.stream);
        body.extend(key);
        body.extend(&r.payload);
    }
    let len = (body.len() as u32).to_le_bytes();
    let checksum = raw::crc(&[len.as_slice(), &body].concat());
    [len.as_slice(), &checksum.to_le_bytes(), &body].concat()
}

/// Given acknowledged history, two new atomic originals, exact and regrouped waiters,
/// finalized rejections and a durable retry, plus deferred and queued work
/// When a partial write, ENOSPC or data-sync error poisons the group
/// Then unfinished work is Indeterminate, unprocessed work Poisoned, and finalized answers stay fixed
/// When reopened in the same boot or after each crash mode, then power is lost immediately
/// Then the atomic prefix and reader observations survive, keys rebuild and unknowns resolve once
/// Cases: ORACLE-1, ORACLE-2, ORACLE-3, ORACLE-4, ORACLE-5, ORACLE-7, ORACLE-8, ORACLE-9, RETRY-REGROUPED-FAIL
#[test]
fn poisoned_groups_recover_durably_before_unknown_keys_are_retried() {
    bounded(|| {
        for initial_crash in [
            None,
            Some(Crash::Kill),
            Some(Crash::PowerLoss),
            Some(Crash::PowerLossTorn),
        ] {
            for fault_index in 0..5 {
                for seed in 0..4 {
                    let sim = Sim::new(701);
                    let payload = vec![0xa7; 5000];
                    let originals = || vec![keyed(b'a', &payload), keyed(b'b', &payload)];
                    let group = frame(
                        1,
                        &[
                            keyed(b'a', &payload),
                            keyed(b'b', &payload),
                            keyed(b'c', b"third"),
                        ],
                    );
                    let log = SegmentLog::with_storage(
                        Path::new("/log"),
                        Options {
                            group_byte_cap: group.len() - 8,
                            ..options()
                        },
                        Arc::new(sim.clone()),
                    )
                    .unwrap();
                    let held = sim.hold(flush());
                    let boot = send(&log, vec![keyed(b'z', b"durable")]);
                    held.wait();
                    let first = send(&log, originals());
                    let second = send(&log, vec![keyed(b'c', b"third")]);
                    let exact = send(&log, originals());
                    let mixed = send(
                        &log,
                        vec![
                            keyed(b'c', b"third"),
                            keyed(b'z', b"durable"),
                            keyed(b'a', &payload),
                        ],
                    );
                    let conflict = send(&log, vec![keyed(b'a', b"changed")]);
                    let partial = send(&log, vec![keyed(b'c', b"third"), keyed(b'x', b"never")]);
                    let wrong = send(
                        &log,
                        vec![NewRecord {
                            key: None,
                            ..keyed(b'a', b"wrong")
                        }],
                    );
                    let durable = send(&log, vec![keyed(b'z', b"durable")]);
                    let deferred = send(&log, vec![keyed(b'd', b"deferred")]);
                    let queued = send(&log, originals());
                    let at_flush = fault_index >= 3;
                    let next = sim.hold(if at_flush {
                        flush()
                    } else {
                        Call::new(ANY_SEG, Purpose::Group, Op::Append(group.clone()))
                    });
                    held.release(None);
                    positions(boot.wait().unwrap(), &[(0, 0)], false);
                    next.wait();
                    let answers = (
                        conflict.probe(),
                        partial.probe(),
                        wrong.probe(),
                        durable.probe(),
                    );
                    let waiting = [&first, &second, &exact, &mixed, &deferred, &queued]
                        .iter()
                        .all(|p| p.probe().is_none());
                    let observed = log.read(0, 10);
                    let watermark = log.watermark();
                    next.release(Some(match fault_index {
                        0 => Fault::Partial(500),
                        1 => Fault::Partial(group.len()),
                        2 | 3 => Fault::NoSpace,
                        _ => Fault::error(std::io::ErrorKind::Other).unwrap(),
                    }));
                    assert!(waiting);
                    assert_eq!(watermark, 1);
                    let mut history = vec![stored(b"z", Some(b"z"), b"durable", 0, 0)];
                    assert_records(&observed.unwrap(), &history);
                    assert!(matches!(
                        answers.0,
                        Some(Err(CommitError::KeyConflict { index: 0 }))
                    ));
                    assert!(matches!(answers.1, Some(Err(CommitError::PartialRetry))));
                    assert!(matches!(
                        answers.2,
                        Some(Err(CommitError::WrongRevision {
                            index: 0,
                            actual: Some(0)
                        }))
                    ));
                    positions(answers.3.unwrap().unwrap(), &[(0, 0)], true);
                    for p in [first, second, exact, mixed] {
                        assert!(matches!(p.wait(), Err(CommitError::Indeterminate)));
                    }
                    for p in [deferred, queued] {
                        assert!(matches!(p.wait(), Err(CommitError::Poisoned)));
                    }
                    assert!(matches!(
                        log.append(Submit {
                            records: originals()
                        }),
                        Err(AppendError::Poisoned)
                    ));
                    assert_records(&log.read(0, 10).unwrap(), &history);
                    assert_eq!(log.watermark(), 1);
                    assert!(log.stream_head(b"a").is_none());
                    drop(log);
                    assert_eq!(
                        sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
                        2
                    );
                    assert_eq!(
                        sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
                        if at_flush { 2 } else { 1 }
                    );
                    history.extend([
                        stored(b"a", Some(b"a"), &payload, 1, 0),
                        stored(b"b", Some(b"b"), &payload, 2, 0),
                        stored(b"c", Some(b"c"), b"third", 3, 0),
                    ]);
                    if let Some(mode) = initial_crash {
                        sim.crash(mode, seed);
                    }
                    let recovered = open(&sim);
                    let end = recovered.watermark();
                    assert!(
                        end == 1 || end == 4,
                        "crash {initial_crash:?}, fault {fault_index}, seed {seed}: {end}"
                    );
                    if initial_crash.is_none() || initial_crash == Some(Crash::Kill) {
                        assert_eq!(
                            end,
                            if fault_index == 0 || fault_index == 2 {
                                1
                            } else {
                                4
                            }
                        );
                    }
                    assert_records(&recovered.read(0, 10).unwrap(), &history[..end as usize]);
                    drop(recovered);
                    sim.crash(Crash::PowerLoss, seed);
                    let recovered = open(&sim);
                    assert_eq!(recovered.watermark(), end);
                    assert_records(&recovered.read(0, 10).unwrap(), &history[..end as usize]);
                    assert_eq!(
                        recovered.stream_head(b"a").map(|h| (h.offset, h.revision)),
                        (end == 4).then_some((1, 0))
                    );
                    positions(
                        send(&recovered, vec![keyed(b'z', b"durable")])
                            .wait()
                            .unwrap(),
                        &[(0, 0)],
                        true,
                    );
                    positions(
                        send(&recovered, originals()).wait().unwrap(),
                        &[(1, 0), (2, 0)],
                        end == 4,
                    );
                    positions(
                        send(&recovered, vec![keyed(b'c', b"third")])
                            .wait()
                            .unwrap(),
                        &[(3, 0)],
                        end == 4,
                    );
                    positions(
                        send(
                            &recovered,
                            vec![keyed(b'c', b"third"), keyed(b'a', &payload)],
                        )
                        .wait()
                        .unwrap(),
                        &[(3, 0), (1, 0)],
                        true,
                    );
                    assert_records(&recovered.read(0, 10).unwrap(), &history);
                    assert!(recovered.stream_head(b"d").is_none());
                    assert!(recovered.stream_head(b"x").is_none());
                }
            }
        }
    });
}

/// Given buffered originals and matched exact and regrouped waiters
/// When a durable lookup returns a shorter CRC-valid frame, even with the target unchanged
/// Then every unfinished request is Indeterminate and queued work Poisoned without a write
/// When the lookup fault is removed and the poisoned log dropped
/// Then same-boot recovery followed by power loss preserves only the acknowledged history
/// Cases: POISON-LOOKUP, ORACLE-7, ORACLE-1, ORACLE-4, ORACLE-5, RETRY-REGROUPED-FAIL
#[test]
fn short_valid_lookup_aborts_buffered_originals_and_waiters() {
    bounded(|| {
        for unchanged_target in [false, true] {
            let sim = Sim::new(702);
            let log = open(&sim);
            let originals = || vec![keyed(b'z', b"original"), keyed(b'y', b"longtail")];
            let original_frame = frame(0, &originals());
            let held = sim.hold(flush());
            let boot = send(&log, originals());
            held.wait();
            let first = send(&log, vec![keyed(b'a', b"one"), keyed(b'b', b"two")]);
            let exact = send(&log, vec![keyed(b'a', b"one"), keyed(b'b', b"two")]);
            let mixed = send(&log, vec![keyed(b'b', b"two"), keyed(b'z', b"original")]);
            let failing = send(&log, vec![keyed(b'z', b"original")]);
            let lookup = Call::new(SEG, Purpose::Lookup, Op::Read(64, original_frame.len()));
            let first_lookup = sim.hold(lookup.clone());
            held.release(None);
            boot.wait().unwrap();
            first_lookup.wait();
            let failing_lookup = sim.hold(lookup);
            first_lookup.release(None);
            failing_lookup.wait();
            let queued = send(&log, vec![keyed(b'a', b"one")]);
            let waiting = [&first, &exact, &mixed, &failing]
                .iter()
                .all(|p| p.probe().is_none());
            let shorter = frame(
                0,
                &[
                    keyed(
                        b'z',
                        if unchanged_target {
                            b"original"
                        } else {
                            b"short"
                        },
                    ),
                    keyed(
                        b'y',
                        if unchanged_target {
                            b"short"
                        } else {
                            b"longtail"
                        },
                    ),
                ],
            );
            sim.call(Call::new(
                SEG,
                Purpose::Open,
                Op::Write(64, shorter.clone()),
            ))
            .unwrap();
            sim.call(Call::new(
                SEG,
                Purpose::Open,
                Op::Truncate(64 + shorter.len() as u64),
            ))
            .unwrap();
            failing_lookup.release(None);
            assert!(waiting);
            for p in [first, exact, mixed, failing] {
                assert!(matches!(p.wait(), Err(CommitError::Indeterminate)));
            }
            assert!(matches!(queued.wait(), Err(CommitError::Poisoned)));
            assert!(matches!(
                log.append(Submit {
                    records: originals()
                }),
                Err(AppendError::Poisoned)
            ));
            assert_eq!(log.watermark(), 2);
            assert!(log.stream_head(b"a").is_none());
            drop(log);
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
                1
            );
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
                1
            );
            sim.call(Call::new(SEG, Purpose::Open, Op::Write(64, original_frame)))
                .unwrap();
            let history = [
                stored(b"z", Some(b"z"), b"original", 0, 0),
                stored(b"y", Some(b"y"), b"longtail", 1, 0),
            ];
            let recovered = open(&sim);
            assert_records(&recovered.read(0, 10).unwrap(), &history);
            drop(recovered);
            sim.crash(Crash::PowerLoss, 0);
            let recovered = open(&sim);
            assert_records(&recovered.read(0, 10).unwrap(), &history);
            positions(
                send(&recovered, originals()).wait().unwrap(),
                &[(0, 0), (1, 0)],
                true,
            );
        }
    });
}

/// Given a durable original and no buffered new records
/// When its held lookup fails by read error, EOF, CRC, malformed body or shorter valid frame
/// Then the lookup is Indeterminate and queued originals and retries are Poisoned
/// And no write or flush follows, even during Drop
/// When the lookup fault is disabled, same-boot recovery and power loss rebuild the original key
/// Cases: POISON-LOOKUP, ORACLE-7, ORACLE-1, ORACLE-4, ORACLE-5
#[test]
fn lookup_failure_without_new_records_poisons_both_kinds_of_queued_work() {
    bounded(|| {
        for fault_index in 0..5 {
            let sim = Sim::new(705);
            let log = open(&sim);
            let original = || vec![keyed(b'z', b"original")];
            let original_frame = frame(0, &original());
            send(&log, original()).wait().unwrap();
            let history = [stored(b"z", Some(b"z"), b"original", 0, 0)];
            assert_records(&log.read(0, 10).unwrap(), &history);
            let held = sim.hold(Call::new(
                SEG,
                Purpose::Lookup,
                Op::Read(64, original_frame.len()),
            ));
            let lookup = send(&log, original());
            held.wait();
            let queued_original = send(&log, vec![keyed(b'a', b"new")]);
            let queued_retry = send(&log, original());
            let fault = match fault_index {
                0 => Some(Fault::error(std::io::ErrorKind::Other).unwrap()),
                1 => Some(Fault::Eof),
                2 => Some(Fault::Corrupt(8, 1)),
                _ => {
                    let mut damaged = if fault_index == 3 {
                        original_frame.clone()
                    } else {
                        frame(0, &[keyed(b'z', b"short")])
                    };
                    if fault_index == 3 {
                        damaged[9] = 1;
                        let crc = raw::crc(&[&damaged[..4], &damaged[8..]].concat());
                        damaged[4..8].copy_from_slice(&crc.to_le_bytes());
                    }
                    sim.call(Call::new(
                        SEG,
                        Purpose::Open,
                        Op::Write(64, damaged.clone()),
                    ))
                    .unwrap();
                    sim.call(Call::new(
                        SEG,
                        Purpose::Open,
                        Op::Truncate(64 + damaged.len() as u64),
                    ))
                    .unwrap();
                    None
                }
            };
            held.release(fault);
            assert!(matches!(lookup.wait(), Err(CommitError::Indeterminate)));
            for p in [queued_original, queued_retry] {
                assert!(matches!(p.wait(), Err(CommitError::Poisoned)));
            }
            assert!(matches!(
                log.append(Submit {
                    records: original()
                }),
                Err(AppendError::Poisoned)
            ));
            assert_eq!(log.watermark(), 1);
            assert!(log.stream_head(b"a").is_none());
            drop(log);
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
                1
            );
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::SyncData),
                1
            );
            if fault_index >= 3 {
                sim.call(Call::new(SEG, Purpose::Open, Op::Write(64, original_frame)))
                    .unwrap();
            }
            let recovered = open(&sim);
            assert_records(&recovered.read(0, 10).unwrap(), &history);
            drop(recovered);
            sim.crash(Crash::PowerLoss, 0);
            let recovered = open(&sim);
            assert_records(&recovered.read(0, 10).unwrap(), &history);
            positions(
                send(&recovered, original()).wait().unwrap(),
                &[(0, 0)],
                true,
            );
        }
    });
}
