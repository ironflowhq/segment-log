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
