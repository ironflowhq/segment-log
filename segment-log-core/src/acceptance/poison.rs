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
        let hold = sim.hold(Call::new(SEG, Purpose::Group, Op::SyncData));
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
            sim.count(Path::new(SEG), Purpose::Group, &Op::Append(vec![])),
            1
        );
        assert_eq!(sim.count(Path::new(SEG), Purpose::Group, &Op::SyncData), 1);
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
                Call::new(SEG, Purpose::Group, Op::Append(first_frame())),
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
            assert_eq!(sim.count(Path::new(SEG), Purpose::Group, &Op::SyncData), 0);
            assert_eq!(
                sim.count(Path::new(SEG), Purpose::Group, &Op::Append(vec![])),
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
