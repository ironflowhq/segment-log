use super::support::{bounded, fixtures::*};
use crate::storage::{Call, Op, Purpose};
use crate::*;
use std::{path::Path, sync::Arc};

/// Given invalid structure and a valid request larger than the configured group
/// When append is called
/// Then it refuses before writing and the next valid request still gets offset zero
/// Cases: ADM-PRECEDENCE, ADM-SIZE, KEY-REPEATED
#[test]
fn malformed_and_oversized_requests_are_refused_before_admission() {
    bounded(|| {
        let sim = Sim::new(8);
        let log = open(&sim);
        assert!(matches!(
            log.append(Submit { records: vec![] }),
            Err(AppendError::Invalid(_))
        ));
        let mut bad = submit(b"");
        assert!(matches!(log.append(bad), Err(AppendError::Invalid(_))));
        bad = submit(b"s");
        bad.records[0].key = Some(vec![]);
        assert!(matches!(log.append(bad), Err(AppendError::Invalid(_))));
        bad = submit(b"s");
        bad.records.push(submit(b"t").records.remove(0));
        assert!(matches!(log.append(bad), Err(AppendError::Invalid(_))));
        bad = submit(b"s");
        bad.records[0].payload = vec![0; 1024 * 1024];
        assert!(matches!(log.append(bad), Err(AppendError::TooLarge)));
        append(&log, b"s").wait().unwrap();
        assert_record(&log);
    });
}

/// Given a held group and a byte budget sufficient for one request only
/// When another eligible request arrives or a single request exceeds the whole budget
/// Then Full and TooLarge are immediate, and answering releases the first reservation
/// Cases: ADM-BUDGETS, ADM-RESERVATION, ADM-CHARGE
#[test]
fn pending_bytes_remain_charged_until_the_answer() {
    bounded(|| {
        let sim = Sim::new(10);
        let log = SegmentLog::with_storage(
            Path::new("/log"),
            Options {
                pending_byte_budget: 4096,
                ..options()
            },
            Arc::new(sim.clone()),
        )
        .unwrap();
        let make = || {
            let mut s = submit(b"s");
            s.records[0].payload = Vec::with_capacity(2500);
            s
        };
        let hold = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
        let pending = log.append(make()).unwrap();
        hold.wait();
        assert!(matches!(log.append(make()), Err(AppendError::Full)));
        let mut huge = make();
        huge.records[0].payload = Vec::with_capacity(5000);
        assert!(matches!(log.append(huge), Err(AppendError::TooLarge)));
        hold.release(None);
        pending.wait().unwrap();
        assert!(log.append(make()).is_ok());
    });
}

use crate::engine::admission_support as accounting;
use crate::storage::sim::Fault;
use std::mem::size_of;

fn configured(sim: &Sim, opts: Options) -> SegmentLog {
    SegmentLog::with_storage(Path::new("/log"), opts, Arc::new(sim.clone())).unwrap()
}
fn flush() -> Call {
    Call::new(ANY_SEG, Purpose::Group, Op::SyncData)
}
fn record(stream: &[u8], key: Option<&[u8]>, payload: &[u8]) -> NewRecord {
    NewRecord {
        stream: stream.to_vec(),
        key: key.map(<[u8]>::to_vec),
        payload: payload.to_vec(),
        expected: ExpectedRevision::Empty,
    }
}
fn request(stream: &[u8]) -> Submit {
    Submit {
        records: vec![record(stream, None, b"")],
    }
}
fn expected_charge(s: &Submit) -> usize {
    accounting::bookkeeping()
        + s.records.capacity() * size_of::<NewRecord>()
        + s.records.len() * size_of::<Position>()
        + s.records
            .iter()
            .map(|r| {
                r.stream.capacity() + r.key.as_ref().map_or(0, Vec::capacity) + r.payload.capacity()
            })
            .sum::<usize>()
}

/// Given a one-slot queue and enough bytes for three requests
/// When one request is flushing and a second occupies the queue
/// Then a third gets Full without leaking its attempted reservation
/// When a later barrier flush starts before either old answer is consumed
/// Then admission succeeds using the released bytes and queue slot
/// Cases: ADM-BUDGETS, ADM-RESERVATION, ADM-PRECEDENCE
#[test]
fn queue_slots_end_at_dequeue_and_failed_insertion_releases_bytes() {
    bounded(|| {
        let sim = Sim::new(601);
        let charge = expected_charge(&request(b"a"));
        let log = configured(
            &sim,
            Options {
                queue_capacity: 1,
                pending_byte_budget: 3 * charge,
                ..options()
            },
        );
        let held = sim.hold(flush());
        let first = log.append(request(b"a")).unwrap();
        held.wait();
        let second = log.append(request(b"b")).unwrap();
        for _ in 0..8 {
            assert!(matches!(log.append(request(b"c")), Err(AppendError::Full)));
        }
        assert_eq!(accounting::used(&log), 2 * charge);
        let next = sim.hold(flush());
        held.release(None);
        next.wait();
        let third = log.append(request(b"c")).unwrap();
        next.release(None);
        first.wait().unwrap();
        second.wait().unwrap();
        third.wait().unwrap();
        assert_records(
            &log.read(0, 10).unwrap(),
            &[
                stored(b"a", None, b"", 0, 0),
                stored(b"b", None, b"", 1, 0),
                stored(b"c", None, b"", 2, 0),
            ],
        );
    });
}

/// Given equal encoded contents with spare capacity in each input allocation
/// When each request has exactly its retained-allocation budget or one byte less
/// Then equality admits and the smaller budget refuses before writing
/// And many tiny records include both structural and result-position allowances
/// Cases: ADM-CHARGE, ADM-SIZE, ADM-RESERVATION
#[test]
fn each_retained_capacity_and_result_slot_is_charged() {
    bounded(|| {
        for variant in 0..6 {
            let make = || {
                let mut s = Submit {
                    records: vec![record(b"s", Some(b"k"), b"")],
                };
                match variant {
                    0 => {}
                    1 => s.records.reserve_exact(31),
                    2 => s.records[0].stream.reserve_exact(97),
                    3 => s.records[0].key.as_mut().unwrap().reserve_exact(97),
                    4 => s.records[0].payload.reserve_exact(97),
                    _ => {
                        s.records = (0..64).map(|i| record(&[i], None, b"")).collect();
                    }
                }
                s
            };
            let charge = expected_charge(&make());
            for short in [true, false] {
                let sim = Sim::new(602);
                let log = configured(
                    &sim,
                    Options {
                        pending_byte_budget: charge - usize::from(short),
                        ..options()
                    },
                );
                if short {
                    assert!(matches!(log.append(make()), Err(AppendError::TooLarge)));
                    assert_eq!(accounting::used(&log), 0);
                    assert_eq!(log.watermark(), 0);
                    assert_eq!(
                        sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
                        0
                    );
                } else {
                    let held = sim.hold(flush());
                    let p = log.append(make()).unwrap();
                    held.wait();
                    assert_eq!(accounting::used(&log), charge);
                    assert!(matches!(log.append(make()), Err(AppendError::Full)));
                    held.release(None);
                    let result = p.wait().unwrap();
                    assert_eq!(result.records.len(), if variant == 5 { 64 } else { 1 });
                }
            }
        }
    });
}

/// Given counts and allocation totals beyond representable address space
/// When the private charge arithmetic evaluates their contract-derived bounds
/// Then every multiplication and addition refuses overflow and equality remains representable
/// Cases: ADM-CHARGE, ADM-SIZE
#[test]
fn charge_arithmetic_refuses_unrepresentable_counts_and_totals() {
    bounded(|| {
        assert_eq!(
            accounting::base(usize::MAX / size_of::<NewRecord>() + 1, 0),
            None
        );
        assert_eq!(
            accounting::base(0, usize::MAX / size_of::<Position>() + 1),
            None
        );
        assert_eq!(
            accounting::base(0, usize::MAX / size_of::<Position>()),
            None
        );
        let allowance = accounting::bookkeeping();
        assert_eq!(
            accounting::base(usize::MAX / size_of::<NewRecord>(), 0),
            None
        );
        assert_eq!(
            accounting::buffers(allowance, usize::MAX - allowance, 0, 0),
            Some(usize::MAX)
        );
        for (stream, key, payload) in [(usize::MAX, 0, 0), (0, usize::MAX, 0), (0, 0, usize::MAX)] {
            assert_eq!(accounting::buffers(allowance, stream, key, payload), None);
        }
    });
}

/// Given the minimum group cap and a held request that exactly fills it
/// When oversized input or a retry arrives while capacity is exhausted
/// Then standalone size is checked first and spare buffer capacity does not enlarge encoded size
/// When temporary pressure clears and a new barrier holds storage
/// Then the same retry is eligible again under the unchanged standalone limits
/// Cases: ADM-SIZE, ADM-BUDGETS, ADM-RESERVATION
#[test]
fn encoded_size_equality_and_retry_eligibility_are_independent_of_pressure() {
    bounded(|| {
        let sim = Sim::new(603);
        let make = || Submit {
            records: vec![record(b"s", Some(b"k"), b"")],
        };
        let charge = expected_charge(&make());
        let log = configured(
            &sim,
            Options {
                group_byte_cap: 24 + 16 + 1 + 1,
                pending_byte_budget: 2 * charge,
                ..options()
            },
        );
        let held = sim.hold(flush());
        let original = log.append(make()).unwrap();
        held.wait();
        let retry = log.append(make()).unwrap();
        assert!(matches!(log.append(make()), Err(AppendError::Full)));
        let mut large = make();
        large.records[0].payload.push(0);
        assert!(matches!(log.append(large), Err(AppendError::TooLarge)));
        held.release(None);
        original.wait().unwrap();
        retry.wait().unwrap();
        let held = sim.hold(flush());
        let barrier = log.append(request(b"b")).unwrap();
        held.wait();
        let retry = log.append(make()).unwrap();
        assert!(matches!(log.append(make()), Err(AppendError::Full)));
        held.release(None);
        barrier.wait().unwrap();
        assert!(retry.wait().unwrap().deduplicated);
        assert_records(
            &log.read(0, 10).unwrap(),
            &[
                stored(b"s", Some(b"k"), b"", 0, 0),
                stored(b"b", None, b"", 1, 0),
            ],
        );
    });
}

/// Given full capacity or a poisoned or stopped writer
/// When malformed fields, repeated keys and oversized requests overlap
/// Then Invalid precedes TooLarge, which precedes state, which precedes Full
/// And a panic loses outstanding answers as Indeterminate without leaving charges
/// Cases: ADM-PRECEDENCE, ADM-RESERVATION, KEY-REPEATED
#[test]
fn structural_size_state_and_capacity_failures_have_defined_precedence() {
    bounded(|| {
        for panic in [false, true] {
            let sim = Sim::new(604);
            let charge = expected_charge(&request(b"s"));
            let mut log = configured(
                &sim,
                Options {
                    queue_capacity: 1,
                    group_byte_cap: 41,
                    pending_byte_budget: 2 * charge,
                    ..options()
                },
            );
            let held = sim.hold(flush());
            let original = log.append(request(b"s")).unwrap();
            held.wait();
            let queued = log.append(request(b"q")).unwrap();
            let check = |log: &SegmentLog, stopped: bool| {
                for kind in 0..6 {
                    let mut bad = request(b"s");
                    bad.records[0].payload = vec![0; 100];
                    match kind {
                        0 => bad.records.clear(),
                        1 => bad.records[0].stream.clear(),
                        2 => bad.records[0].stream = vec![0; 1025],
                        3 => bad.records[0].key = Some(vec![]),
                        4 => bad.records[0].key = Some(vec![0; 257]),
                        _ => {
                            bad.records = vec![
                                record(b"s", Some(b"k"), b""),
                                record(b"t", Some(b"k"), b"different"),
                            ]
                        }
                    }
                    assert!(matches!(log.append(bad), Err(AppendError::Invalid(_))));
                }
                let mut large = request(b"s");
                large.records[0].payload.push(0);
                assert!(matches!(log.append(large), Err(AppendError::TooLarge)));
                let mut retained = request(b"s");
                retained.records[0].payload = Vec::with_capacity(2 * charge);
                assert!(matches!(log.append(retained), Err(AppendError::TooLarge)));
                assert!(
                    matches!(log.append(request(b"x")), Err(e) if if stopped { matches!(e, AppendError::Poisoned) } else { matches!(e, AppendError::Full) })
                );
            };
            check(&log, false);
            held.release(Some(if panic { Fault::Panic } else { Fault::NoSpace }));
            accounting::finish(&mut log, panic);
            assert_eq!(accounting::used(&log), 0);
            check(&log, true);
            assert!(matches!(original.wait(), Err(CommitError::Indeterminate)));
            assert!(
                matches!(queued.wait(), Err(e) if if panic { matches!(e, CommitError::Indeterminate) } else { matches!(e, CommitError::Poisoned) })
            );
            assert_eq!(log.watermark(), 0);
            assert!(log.stream_head(b"s").is_none());
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
                1
            );
        }
    });
}

/// Given repeated key bytes in records with every stream, payload and expected-revision combination
/// When appended to an otherwise empty log
/// Then every combination is Invalid and consumes no offset, revision or reservation
/// And multiple unkeyed records remain valid
/// Cases: KEY-REPEATED, ADM-PRECEDENCE, ADM-RESERVATION
#[test]
fn repeated_keys_ignore_stream_payload_and_expected_revision_differences() {
    bounded(|| {
        let sim = Sim::new(605);
        let log = open(&sim);
        for same_stream in [false, true] {
            for same_payload in [false, true] {
                for expected in [
                    ExpectedRevision::Empty,
                    ExpectedRevision::At(0),
                    ExpectedRevision::At(u64::MAX),
                ] {
                    let mut second = record(
                        if same_stream { b"s" } else { b"t" },
                        Some(b"k"),
                        if same_payload { b"one" } else { b"two" },
                    );
                    second.expected = expected;
                    assert!(matches!(
                        log.append(Submit {
                            records: vec![record(b"s", Some(b"k"), b"one"), second]
                        }),
                        Err(AppendError::Invalid(_))
                    ));
                    assert_eq!(accounting::used(&log), 0);
                }
            }
        }
        assert_eq!(log.watermark(), 0);
        assert!(log.stream_head(b"s").is_none());
        let mut second = record(b"s", None, b"two");
        second.expected = ExpectedRevision::At(0);
        let result = log
            .append(Submit {
                records: vec![record(b"s", None, b"one"), second],
            })
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(result.records.len(), 2);
        assert_records(
            &log.read(0, 10).unwrap(),
            &[
                stored(b"s", None, b"one", 0, 0),
                stored(b"s", None, b"two", 1, 1),
            ],
        );
    });
}

/// Given a durable key and a queued request that will finish with each terminal outcome
/// When a later barrier reaches storage, without consuming the earlier answer
/// Then the earlier reservation is available for a new request
/// Cases: ADM-RESERVATION, ADM-CHARGE
#[test]
fn every_terminal_answer_releases_bytes_before_the_caller_receives_it() {
    bounded(|| {
        for outcome in 0..5 {
            let sim = Sim::new(620);
            let target = || match outcome {
                0 => Submit {
                    records: vec![record(b"n", None, &[0, 0])],
                },
                1 => request(b"s"),
                2 => Submit {
                    records: vec![record(b"x", Some(b"k"), b"")],
                },
                3 => Submit {
                    records: vec![record(b"s", Some(b"k"), b""), record(b"x", None, b"")],
                },
                _ => Submit {
                    records: vec![record(b"s", Some(b"k"), b"")],
                },
            };
            let target_charge = expected_charge(&target());
            let barrier_charge = expected_charge(&request(b"b"));
            let budget = target_charge + 2 * barrier_charge;
            let log = configured(
                &sim,
                Options {
                    group_byte_cap: 59,
                    pending_byte_budget: budget,
                    ..options()
                },
            );
            log.append(Submit {
                records: vec![record(b"s", Some(b"k"), b"")],
            })
            .unwrap()
            .wait()
            .unwrap();
            let held = sim.hold(flush());
            let bootstrap = log.append(request(b"a")).unwrap();
            held.wait();
            let target_pending = log.append(target()).unwrap();
            let barrier = log.append(request(b"b")).unwrap();
            let next = sim.hold(flush());
            held.release(None);
            next.wait();
            let barrier_hold = if outcome == 0 {
                let final_hold = sim.hold(flush());
                next.release(None);
                final_hold.wait();
                final_hold
            } else {
                next
            };
            assert_eq!(accounting::used(&log), barrier_charge);
            let mut fill = request(b"f");
            fill.records[0].payload = Vec::with_capacity(budget - 2 * barrier_charge);
            let admitted = log.append(fill).unwrap();
            assert!(matches!(log.append(request(b"z")), Err(AppendError::Full)));
            barrier_hold.release(None);
            bootstrap.wait().unwrap();
            match outcome {
                0 => {
                    assert!(!target_pending.wait().unwrap().deduplicated);
                }
                1 => assert!(matches!(
                    target_pending.wait(),
                    Err(CommitError::WrongRevision {
                        index: 0,
                        actual: Some(0)
                    })
                )),
                2 => assert!(matches!(
                    target_pending.wait(),
                    Err(CommitError::KeyConflict { index: 0 })
                )),
                3 => assert!(matches!(
                    target_pending.wait(),
                    Err(CommitError::PartialRetry)
                )),
                _ => {
                    assert!(target_pending.wait().unwrap().deduplicated);
                }
            }
            barrier.wait().unwrap();
            admitted.wait().unwrap();
            let mut expected = vec![
                stored(b"s", Some(b"k"), b"", 0, 0),
                stored(b"a", None, b"", 1, 0),
            ];
            if outcome == 0 {
                expected.push(stored(b"n", None, &[0, 0], 2, 0));
            }
            let offset = expected.len() as u64;
            expected.push(stored(b"b", None, b"", offset, 0));
            expected.push(stored(b"f", None, b"", offset + 1, 0));
            assert_records(&log.read(0, 10).unwrap(), &expected);
        }
    });
}

/// Given an original and a matched retry queued behind a held bootstrap
/// When they reach their group flush, with either result handle abandoned
/// Then both byte charges remain held until the answer
/// When a later barrier starts its flush
/// Then a new request can use both released charges before any old result is consumed
/// Cases: ADM-RESERVATION, ADM-BUDGETS, ADM-CHARGE
#[test]
fn matched_and_abandoned_pending_handles_keep_fixed_charges_until_answered() {
    bounded(|| {
        for abandon in [false, true] {
            let sim = Sim::new(621);
            let make = || Submit {
                records: vec![record(b"s", Some(b"k"), b"")],
            };
            let c = expected_charge(&make());
            let b = expected_charge(&request(b"b"));
            let log = configured(
                &sim,
                Options {
                    pending_byte_budget: 2 * c + b,
                    group_byte_cap: 42,
                    ..options()
                },
            );
            let held = sim.hold(flush());
            let boot = log.append(request(b"a")).unwrap();
            held.wait();
            let original = log.append(make()).unwrap();
            let retry = log.append(make()).unwrap();
            let next = sim.hold(flush());
            held.release(None);
            next.wait();
            let handles = if abandon {
                drop((original, retry));
                None
            } else {
                Some((original, retry))
            };
            assert_eq!(accounting::used(&log), 2 * c);
            let barrier = log.append(request(b"b")).unwrap();
            assert!(matches!(log.append(make()), Err(AppendError::Full)));
            let last = sim.hold(flush());
            next.release(None);
            last.wait();
            let new = log.append(make()).unwrap();
            let other = log.append(make()).unwrap();
            assert!(matches!(log.append(make()), Err(AppendError::Full)));
            last.release(None);
            boot.wait().unwrap();
            if let Some((original, retry)) = handles {
                assert!(!original.wait().unwrap().deduplicated);
                assert!(retry.wait().unwrap().deduplicated);
            }
            barrier.wait().unwrap();
            assert!(new.wait().unwrap().deduplicated);
            assert!(other.wait().unwrap().deduplicated);
            assert_records(
                &log.read(0, 10).unwrap(),
                &[
                    stored(b"a", None, b"", 0, 0),
                    stored(b"s", Some(b"k"), b"", 1, 0),
                    stored(b"b", None, b"", 2, 0),
                ],
            );
        }
    });
}

/// Given multiple producers released together while storage holds a group
/// When they compete for two remaining byte reservations in a roomy queue
/// Then exactly two are admitted and the others receive immediate Full
/// Cases: ADM-BUDGETS, ADM-RESERVATION
#[test]
fn concurrent_producers_cannot_overrun_the_byte_ceiling() {
    bounded(|| {
        let sim = Sim::new(622);
        let c = expected_charge(&request(b"a"));
        let log = Arc::new(configured(
            &sim,
            Options {
                pending_byte_budget: 3 * c,
                queue_capacity: 32,
                ..options()
            },
        ));
        let held = sim.hold(flush());
        let original = log.append(request(b"a")).unwrap();
        held.wait();
        let start = Arc::new(std::sync::Barrier::new(9));
        let workers: Vec<_> = (0..8)
            .map(|i| {
                let log = log.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    log.append(request(&[i]))
                })
            })
            .collect();
        start.wait();
        let mut accepted = Vec::new();
        for worker in workers {
            match worker.join().unwrap() {
                Ok(p) => accepted.push(p),
                Err(AppendError::Full) => {}
                Err(e) => panic!("{e:?}"),
            }
        }
        assert_eq!(accepted.len(), 2);
        assert_eq!(accounting::used(&log), 3 * c);
        held.release(None);
        original.wait().unwrap();
        for p in accepted {
            p.wait().unwrap();
        }
        assert_eq!(log.watermark(), 3);
    });
}

/// Given the maximum valid stream and key lengths with spare payload capacity
/// When the body cap equals the encoded lengths and a one-byte-larger record is attempted
/// Then equality writes the literal input and the larger record gets TooLarge
/// Cases: ADM-SIZE, ADM-CHARGE, ADM-PRECEDENCE
#[test]
fn maximum_fields_fit_by_length_even_with_unused_buffers() {
    bounded(|| {
        let sim = Sim::new(623);
        let stream = vec![1; 1024];
        let key = vec![2; 256];
        let log = configured(
            &sim,
            Options {
                group_byte_cap: 24 + 16 + 1024 + 256,
                ..options()
            },
        );
        let mut r = record(&stream, Some(&key), b"");
        r.payload = Vec::with_capacity(9000);
        let held = sim.hold(flush());
        let original = log.append(Submit { records: vec![r] }).unwrap();
        held.wait();
        assert!(matches!(
            log.append(Submit {
                records: vec![record(&stream, Some(&key), b"x")]
            }),
            Err(AppendError::TooLarge)
        ));
        held.release(None);
        original.wait().unwrap();
        assert_records(
            &log.read(0, 10).unwrap(),
            &[stored(&stream, Some(&key), b"", 0, 0)],
        );
    });
}

/// Given a matched pending retry, a dropped result handle and a later queued request
/// When their group fails or the writer panics at storage
/// Then shutdown releases every charge before any remaining answer is consumed
/// Cases: ADM-RESERVATION, ADM-PRECEDENCE
#[test]
fn failed_groups_release_matched_abandoned_and_queued_reservations() {
    bounded(|| {
        for panic in [false, true] {
            let sim = Sim::new(624);
            let make = || Submit {
                records: vec![record(b"s", Some(b"k"), b"")],
            };
            let c = expected_charge(&make());
            let mut log = configured(
                &sim,
                Options {
                    pending_byte_budget: 4 * c,
                    group_byte_cap: 42,
                    ..options()
                },
            );
            let held = sim.hold(flush());
            let boot = log.append(request(b"a")).unwrap();
            held.wait();
            let original = log.append(make()).unwrap();
            let retry = log.append(make()).unwrap();
            let dropped = log.append(make()).unwrap();
            drop(dropped);
            let next = sim.hold(flush());
            held.release(None);
            next.wait();
            assert_eq!(accounting::used(&log), 3 * c);
            let queued = log.append(request(b"b")).unwrap();
            assert!(matches!(log.append(make()), Err(AppendError::Full)));
            next.release(Some(if panic { Fault::Panic } else { Fault::NoSpace }));
            accounting::finish(&mut log, panic);
            assert_eq!(accounting::used(&log), 0);
            assert!(matches!(log.append(make()), Err(AppendError::Poisoned)));
            boot.wait().unwrap();
            for p in [original, retry] {
                assert!(matches!(p.wait(), Err(CommitError::Indeterminate)));
            }
            assert!(
                matches!(queued.wait(),Err(e) if if panic { matches!(e,CommitError::Indeterminate) } else { matches!(e,CommitError::Poisoned) })
            );
            assert_records(&log.read(0, 10).unwrap(), &[stored(b"a", None, b"", 0, 0)]);
        }
    });
}

/// Given a durable keyed record created under larger standalone limits
/// When reopened with a smaller group cap or charge budget
/// Then even an exact retry gets TooLarge before lookup, without another write
/// Cases: ADM-SIZE, ADM-CHARGE
#[test]
fn durable_retries_must_still_fit_standalone_limits() {
    bounded(|| {
        for charge_limit in [false, true] {
            let sim = Sim::new(625);
            let make = || Submit {
                records: vec![record(b"s", Some(b"k"), b"payload")],
            };
            let log = open(&sim);
            log.append(make()).unwrap().wait().unwrap();
            drop(log);
            let writes = sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![]));
            let log = configured(
                &sim,
                Options {
                    group_byte_cap: if charge_limit {
                        options().group_byte_cap
                    } else {
                        24 + 16 + 1 + 1 + 7 - 1
                    },
                    pending_byte_budget: if charge_limit {
                        expected_charge(&make()) - 1
                    } else {
                        options().pending_byte_budget
                    },
                    ..options()
                },
            );
            assert!(matches!(log.append(make()), Err(AppendError::TooLarge)));
            assert_eq!(accounting::used(&log), 0);
            assert_eq!(
                sim.count(Path::new(ANY_SEG), Purpose::Group, &Op::Append(vec![])),
                writes
            );
            assert_records(
                &log.read(0, 10).unwrap(),
                &[stored(b"s", Some(b"k"), b"payload", 0, 0)],
            );
        }
    });
}

/// Given a held original and a queued retry with independently calculated charges
/// When both reservations coexist, finish, and a later smaller request finishes
/// Then the high-water probe retains their exact sum while current use returns to zero
/// Cases: ADM-RESERVATION, ADM-CHARGE
#[test]
fn peak_charges_remember_answered_requests() {
    bounded(|| {
        let sim = Sim::new(1010);
        let log = open(&sim);
        let bytes = expected_charge(&submit(b"s"));
        let hold = sim.hold(flush());
        let first = append(&log, b"s");
        hold.wait();
        let retry = append(&log, b"s");
        assert_eq!(accounting::peak(&log), 2 * bytes);
        hold.release(None);
        let original = first.wait().unwrap();
        let repeated = retry.wait().unwrap();
        assert_eq!(original.records.len(), 1);
        assert_eq!(repeated.records.len(), 1);
        assert_eq!(
            (original.records[0].offset, repeated.records[0].offset),
            (0, 0)
        );
        assert_eq!(
            (original.records[0].revision, repeated.records[0].revision),
            (0, 0)
        );
        assert!(repeated.deduplicated);
        assert_record(&log);
        assert_eq!(accounting::used(&log), 0);
        log.append(request(b"t")).unwrap().wait().unwrap();
        assert_eq!(log.watermark(), 2);
        assert_eq!(accounting::used(&log), 0);
        assert_eq!(accounting::peak(&log), 2 * bytes);
    });
}
