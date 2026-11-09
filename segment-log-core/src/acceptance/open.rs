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
