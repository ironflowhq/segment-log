use super::support::{bounded, fixtures::*};
use crate::storage::{
    Op, Purpose,
    sim::{Crash, Event, Fault},
};
use crate::*;
use std::{io, path::Path, sync::Arc};

fn try_open(sim: &Sim) -> Result<SegmentLog, OpenError> {
    SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone()))
}

/// Given creation or recovery of an acknowledged log
/// When a directory sync, parent sync, recovery rewrite or SyncAll fails, including partial writes and ENOSPC
/// Then open returns Io without a handle or a sync retry and releases the lock
/// And after reboot for metadata failure, a later successful open and power loss preserve acknowledged records
/// Cases: OPEN-FAULTS, OPEN-DIR, ORACLE-1, ORACLE-5
#[test]
fn failed_open_releases_its_lock_and_can_recover_without_sync_retries() {
    bounded(|| {
        for existing in [false, true] {
            let sim = Sim::new(210);
            if existing {
                let log = open(&sim);
                append(&log, b"s").wait().unwrap();
                drop(log);
            }
            let (seed, before) = sim.trace();
            drop(open(&sim));
            let after = sim.trace().1;
            let targets: Vec<_> = after[before.len()..]
                .iter()
                .filter_map(|event| {
                    let Event::Call(call, _, _) = event else {
                        return None;
                    };
                    (matches!(call.op, Op::SyncDir | Op::SyncAll)
                        || (call.purpose == Purpose::Recovery && matches!(call.op, Op::Write(..))))
                    .then_some(call.clone())
                })
                .collect();
            for target in targets {
                let faults = if matches!(target.op, Op::Write(..)) {
                    vec![
                        Fault::Partial(1),
                        Fault::NoSpace,
                        Fault::error(io::ErrorKind::Other).unwrap(),
                    ]
                } else {
                    vec![Fault::NoSpace, Fault::error(io::ErrorKind::Other).unwrap()]
                };
                for fault in faults {
                    let replay = Sim::replay(seed, &before);
                    replay.inject(target.clone(), fault);
                    assert!(
                        matches!(try_open(&replay), Err(OpenError::Io(_))),
                        "{target:?}"
                    );
                    let trace = replay.trace().1;
                    assert_eq!(
                        trace[before.len()..]
                            .iter()
                            .filter(|event| matches!(event, Event::Call(c, _, _) if c == &target))
                            .count(),
                        1,
                        "failed calls must not be retried"
                    );
                    assert!(
                        trace[before.len()..]
                            .iter()
                            .any(|event| matches!(event, Event::Unlock(_))),
                        "failed open must release its lock"
                    );
                    if target.op == Op::SyncDir {
                        assert!(
                            matches!(try_open(&replay), Err(OpenError::Io(_))),
                            "metadata failure remains fatal until reboot"
                        );
                        replay.crash(Crash::PowerLoss, 0);
                    }
                    let log = open(&replay);
                    if existing {
                        assert_record(&log);
                    } else {
                        assert_eq!(log.watermark(), 0);
                        append(&log, b"s").wait().unwrap();
                    }
                    drop(log);
                    replay.crash(Crash::PowerLoss, 0);
                    assert_record(&open(&replay));
                }
            }
        }
    });
}
