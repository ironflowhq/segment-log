use super::support::{bounded, fixtures::*};
use crate::storage::{Call, Op, Purpose};
use crate::*;
use std::{path::Path, sync::Arc};

/// Given an empty log whose next group flush is held after open
/// When a caller submits one record and the flush has started
/// Then no answer, record, watermark advance or durable stream head is visible
/// When the flush succeeds
/// Then offset 0 revision 0 and every record field are visible after exactly one group flush
/// Cases: ORACLE-8, GROUP-FORMATION, READ-BOUNDED
#[test]
fn append_is_invisible_until_its_group_flush_succeeds() {
    bounded(|| {
        let sim = Sim::new(1);
        let log = open(&sim);
        let hold = sim.hold(Call::new(SEG, Purpose::Group, Op::SyncData));
        let pending = append(&log, b"s");
        hold.wait();
        assert!(pending.probe().is_none());
        assert!(log.read(0, 100).unwrap().is_empty());
        assert_eq!(log.watermark(), 0);
        assert!(log.stream_head(b"s").is_none());
        hold.release(None);
        let answer = pending.wait().unwrap();
        assert!(!answer.deduplicated);
        assert_eq!(answer.records.len(), 1);
        assert_eq!(
            (answer.records[0].offset, answer.records[0].revision),
            (0, 0)
        );
        assert_record(&log);
        assert_eq!(sim.count(Path::new(SEG), Purpose::Group, &Op::SyncData), 1);
    });
}

/// Given a durable record
/// When read requests zero records or starts at or beyond the watermark
/// Then it returns no records, while ordinary limits bound the result
/// Cases: READ-BOUNDED
#[test]
fn read_boundaries_return_empty() {
    bounded(|| {
        let sim = Sim::new(5);
        let log = open(&sim);
        append(&log, b"s").wait().unwrap();
        assert!(log.read(0, 0).unwrap().is_empty());
        assert!(log.read(1, 10).unwrap().is_empty());
        assert!(log.read(u64::MAX, 10).unwrap().is_empty());
        assert_eq!(log.read(0, 1).unwrap().len(), 1);
    });
}

/// Given zero linger and a held first flush
/// When two later submits queue before that flush is released
/// Then they share one subsequent flush if their combined body fits the cap
/// And a cap that fits only one submit gives two subsequent flushes without splitting
/// Cases: GROUP-FORMATION, READ-BOUNDED
#[test]
fn zero_linger_drains_queued_submits_up_to_the_cap() {
    bounded(|| {
        for (cap, flushes) in [(1024, 2), (47, 3)] {
            let sim = Sim::new(6);
            let log = SegmentLog::with_storage(
                Path::new("/log"),
                Options {
                    group_byte_cap: cap,
                    ..options()
                },
                Arc::new(sim.clone()),
            )
            .unwrap();
            let hold = sim.hold(Call::new(SEG, Purpose::Group, Op::SyncData));
            let first = append(&log, b"s");
            hold.wait();
            let mut a = submit(b"a");
            a.records[0].key = None;
            let mut b = submit(b"b");
            b.records[0].key = None;
            let second = log.append(a).unwrap();
            let third = log.append(b).unwrap();
            hold.release(None);
            first.wait().unwrap();
            second.wait().unwrap();
            third.wait().unwrap();
            assert_eq!(log.watermark(), 3);
            assert_eq!(
                sim.count(Path::new(SEG), Purpose::Group, &Op::SyncData),
                flushes
            );
            let records = log.read(1, 1).unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].stream, b"a");
        }
    });
}
