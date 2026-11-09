use super::support::{bounded, fixtures::*};
use crate::storage;
use crate::storage::{Op, Purpose};
use crate::*;
use std::{path::Path, sync::Arc};

/// Given open has returned on a newly created log
/// When its storage trace is examined and power is lost
/// Then the header and log directory and parent have been synced before publication
/// And another live writer is refused and the durable empty log can reopen
/// Cases: OPEN-DIR, ORACLE-11
#[test]
fn creation_is_durable_and_exclusively_locked_before_returning() {
    bounded(|| {
        let sim = Sim::new(4);
        let log = open(&sim);
        assert_eq!(sim.count(Path::new(SEG), Purpose::Open, &Op::SyncData), 1);
        assert_eq!(
            sim.count(Path::new(SEG), Purpose::Recovery, &Op::SyncAll),
            1
        );
        assert!(sim.count(Path::new("/log"), Purpose::Recovery, &Op::SyncDir) > 0);
        assert!(sim.count(Path::new("/"), Purpose::Recovery, &Op::SyncDir) > 0);
        assert!(matches!(
            SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone())),
            Err(OpenError::Locked)
        ));
        sim.crash(storage::sim::Crash::PowerLossTorn, 11);
        drop(log);
        assert_eq!(open(&sim).watermark(), 0);
    });
}

/// Given options outside their documented ranges
/// When open is called
/// Then InvalidOptions is returned before any storage operation
/// Cases: OPT-VALIDATION
#[test]
fn invalid_options_do_not_touch_storage() {
    bounded(|| {
        for options in [
            Options {
                queue_capacity: 0,
                ..options()
            },
            Options {
                pending_byte_budget: 0,
                ..options()
            },
            Options {
                group_byte_cap: 40,
                ..options()
            },
            Options {
                group_byte_cap: 8 * 1024 * 1024 + 1,
                ..options()
            },
            Options {
                roll_bytes: 0,
                ..options()
            },
            Options {
                roll_bytes: 4000 * 1024 * 1024 + 1,
                ..options()
            },
        ] {
            let sim = Sim::new(9);
            assert!(matches!(
                SegmentLog::with_storage(Path::new("/log"), options, Arc::new(sim.clone())),
                Err(OpenError::InvalidOptions(_))
            ));
            assert!(sim.trace().1.is_empty());
        }
    });
}

/// Given Options::default and a changed byte budget
/// When their values are inspected
/// Then the API defaults are exact and changing bytes does not change queue or group capacity
/// Cases: OPT-DEFAULTS
#[test]
fn defaults_match_the_api_and_budgets_are_independent() {
    bounded(|| {
        let o = Options::default();
        assert_eq!(o.linger, std::time::Duration::from_millis(1));
        assert_eq!(o.group_byte_cap, 1024 * 1024);
        assert_eq!(o.roll_bytes, 64 * 1024 * 1024);
        assert_eq!(o.queue_capacity, 4096);
        assert_eq!(o.pending_byte_budget, 16 * 1024 * 1024);
        let changed = Options {
            pending_byte_budget: 1,
            ..o
        };
        assert_eq!(changed.queue_capacity, 4096);
        assert_eq!(changed.group_byte_cap, 1024 * 1024);
    });
}

/// Given inclusive group and roll boundaries, zero linger and positive byte budgets
/// When logs open and submit the smallest record
/// Then minimum and maximum options are accepted, tiny budgets refuse only the request
/// And an address-space-sized accounting ceiling opens without preallocation
/// Cases: OPT-VALIDATION, OPT-DEFAULTS, ADM-SIZE
#[test]
fn inclusive_option_boundaries_open_without_budget_sized_allocation() {
    bounded(|| {
        let minimum_body = 24 + 16 + 1;
        for cap in [minimum_body, 8 * 1024 * 1024] {
            for roll_bytes in [1, 4000 * 1024 * 1024] {
                for budget in [1, usize::MAX] {
                    let sim = Sim::new(610);
                    let log = SegmentLog::with_storage(
                        Path::new("/log"),
                        Options {
                            linger: std::time::Duration::ZERO,
                            queue_capacity: 1,
                            group_byte_cap: cap,
                            roll_bytes,
                            pending_byte_budget: budget,
                        },
                        Arc::new(sim.clone()),
                    )
                    .unwrap();
                    let result = log.append(Submit {
                        records: vec![NewRecord {
                            stream: b"s".to_vec(),
                            key: None,
                            payload: vec![],
                            expected: ExpectedRevision::Empty,
                        }],
                    });
                    if budget == 1 {
                        assert!(matches!(result, Err(AppendError::TooLarge)));
                    } else {
                        result.unwrap().wait().unwrap();
                        assert_records(&log.read(0, 1).unwrap(), &[stored(b"s", None, b"", 0, 0)]);
                        assert_eq!(
                            sim.count(Path::new(SEG), Purpose::Group, &Op::Append(vec![])),
                            1
                        );
                    }
                }
            }
        }
    });
}

/// Given a locked log, missing ancestors, or a denied create operation
/// When invalid options are supplied
/// Then validation wins without even attempting a storage call
/// Cases: OPT-VALIDATION
#[test]
fn invalid_options_precede_locks_missing_ancestors_and_permissions() {
    bounded(|| {
        use crate::storage::{Call, sim::Fault};
        let sim = Sim::new(801);
        let _log = open(&sim);
        for path in ["/log", "/missing/log", "/denied"] {
            sim.inject(
                Call::new(path, Purpose::Open, Op::CreateDir),
                Fault::error(std::io::ErrorKind::PermissionDenied).unwrap(),
            );
            let before = sim.trace().1;
            assert!(matches!(
                SegmentLog::with_storage(
                    Path::new(path),
                    Options {
                        queue_capacity: 0,
                        ..options()
                    },
                    Arc::new(sim.clone())
                ),
                Err(OpenError::InvalidOptions(_))
            ));
            assert_eq!(sim.trace().1, before);
        }
    });
}

/// Given two recognized segments, with an incomplete header or invalid tail beside a valid file
/// When open rejects this unsupported layout
/// Then all bytes and directory entries remain unchanged, before cleanup or recovery writes
/// Cases: OPEN-LAYOUT
#[test]
fn multiple_segments_are_refused_before_any_recovery_mutation() {
    bounded(|| {
        use super::support::raw;
        use crate::storage::{Call, Storage};
        let valid = [raw::header(0, 0, 1), raw::group(1, 0, &[b"kept"])].concat();
        for damaged in [vec![], vec![0; 63], [valid.clone(), vec![0; 33]].concat()] {
            for damaged_is_newest in [false, true] {
                let sim = Sim::new(802);
                raw::install(
                    &sim,
                    if damaged_is_newest {
                        valid.clone()
                    } else {
                        damaged.clone()
                    },
                );
                let second = "/log/00000000000000000001.seg";
                sim.call(Call::new(second, Purpose::Open, Op::Create))
                    .unwrap();
                sim.call(Call::new(
                    second,
                    Purpose::Open,
                    Op::Write(
                        0,
                        if damaged_is_newest {
                            damaged.clone()
                        } else {
                            raw::header(0, 1, 1)
                        },
                    ),
                ))
                .unwrap();
                sim.call(Call::new(second, Purpose::Open, Op::SyncAll))
                    .unwrap();
                sim.call(Call::new("/log", Purpose::Open, Op::SyncDir))
                    .unwrap();
                let before = sim.image();
                let start = sim.trace().1.len();
                assert!(
                    matches!(SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone())),
                    Err(OpenError::Io(e)) if e.kind() == std::io::ErrorKind::Unsupported)
                );
                assert_eq!(sim.image(), before);
                assert!(!sim.trace().1[start..].iter().any(|e| matches!(e,
                    crate::storage::sim::Event::Call(c, _, _) if matches!(c.op,
                        Op::Write(..) | Op::Truncate(_) | Op::Remove | Op::SyncAll | Op::SyncData | Op::SyncDir))));
            }
        }
    });
}

/// Given a durable parent and fresh log creation or incomplete-header replacement
/// When a crash cuts creation at each storage boundary, including every directory sync
/// Then reopening can acknowledge a literal record that remains reachable after power loss
/// And successful open itself preserves the segment entry and bytes across immediate power loss
/// Cases: OPEN-DIR, ORACLE-1, ORACLE-5
#[test]
fn every_creation_boundary_recovers_a_reachable_log() {
    bounded(|| {
        use super::support::raw;
        use crate::storage::{Call, Storage, Value, sim::Crash};
        for incomplete in [false, true] {
            let sim = Sim::new(803);
            if incomplete {
                raw::install(&sim, vec![0; 17]);
            }
            let start = sim.trace().1.len();
            drop(open(&sim));
            let (seed, trace) = sim.trace();
            for cut in start..=trace.len() {
                for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
                    let replay = Sim::replay(seed, &trace[..cut]);
                    replay.crash(mode, cut as u64);
                    let log = open(&replay);
                    assert_eq!(log.watermark(), 0);
                    drop(log);
                    let bytes = raw::bytes(&replay);
                    replay.crash(Crash::PowerLoss, 0);
                    let entries = replay
                        .call(Call::new("/log", Purpose::Open, Op::List))
                        .unwrap();
                    assert!(
                        matches!(entries, Value::Names(names) if names.contains(&"00000000000000000000.seg".into()))
                    );
                    assert_eq!(raw::bytes(&replay), bytes);
                    let log = open(&replay);
                    append(&log, b"s").wait().unwrap();
                    drop(log);
                    replay.crash(Crash::PowerLoss, 0);
                    assert_record(&open(&replay));
                }
            }
        }
    });
}

/// Given creation paused at its final parent directory sync
/// When that call has started but has not completed
/// Then open has not returned a handle that could acknowledge an append
/// When the sync succeeds, an acknowledged record survives immediate power loss
/// Cases: OPEN-DIR, ORACLE-1
#[test]
fn open_cannot_publish_before_parent_directory_sync() {
    bounded(|| {
        use crate::storage::{Call, sim::Crash};
        let sim = Sim::new(804);
        let hold = sim.hold(Call::new("/", Purpose::Recovery, Op::SyncDir));
        let (tx, rx) = std::sync::mpsc::channel();
        let s = sim.clone();
        let worker = std::thread::spawn(move || tx.send(open(&s)).unwrap());
        hold.wait();
        assert!(matches!(
            rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        hold.release(None);
        let log = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
        append(&log, b"s").wait().unwrap();
        sim.crash(Crash::PowerLoss, 0);
        drop(log);
        assert_record(&open(&sim));
    });
}
