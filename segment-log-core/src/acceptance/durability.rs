use super::support::{bounded, fixtures::*};
use crate::storage::{
    Call, Op, Purpose,
    sim::{Crash, Event, Fault},
};
use crate::*;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn lingering(sim: &Sim, linger: Duration) -> SegmentLog {
    SegmentLog::with_storage(
        Path::new("/log"),
        Options {
            linger,
            ..options()
        },
        Arc::new(sim.clone()),
    )
    .unwrap()
}
/// Admits one unkeyed record for each stream, in order.
fn unkeyed(log: &SegmentLog, streams: &[&[u8]]) -> Vec<Pending> {
    streams
        .iter()
        .map(|stream| {
            let mut submit = submit(stream);
            submit.records[0].key = None;
            log.append(submit).unwrap()
        })
        .collect()
}

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

/// Given a one-second linger and a first submit, whose flush is then held
/// When three submits are admitted during the hold and the hold lasts one more linger
/// Then the first submit waited its whole linger alone before its flush started
/// And after the release the three share one flush, which does not wait another linger
/// Cases: GROUP-FORMATION
#[test]
fn linger_runs_from_the_admission_of_a_groups_first_submit() {
    bounded(|| {
        let linger = Duration::from_secs(1);
        let sim = Sim::new(46);
        let log = lingering(&sim, linger);
        let hold = sim.hold(Call::new(SEG, Purpose::Group, Op::SyncData));
        let alone = Instant::now();
        let first = append(&log, b"s");
        hold.wait();
        assert!(alone.elapsed() >= linger);
        let late = unkeyed(&log, &[b"a", b"b", b"c"]);
        // Real time must pass here: the scenario is about a linger that ends during a flush.
        std::thread::sleep(linger);
        let released = Instant::now();
        hold.release(None);
        first.wait().unwrap();
        for pending in late {
            pending.wait().unwrap();
        }
        assert!(
            released.elapsed() < linger,
            "the late group waited another linger after {:?}",
            released.elapsed()
        );
        assert_eq!(sim.count(Path::new(SEG), Purpose::Group, &Op::SyncData), 2);
        assert_eq!(log.watermark(), 4);
    });
}

/// Given three submits that a held flush delayed beyond their linger, so that they form one late group
/// When the flush of that group fails
/// Then all three get Indeterminate, a later submit gets Poisoned and nothing more is written
/// When each storage boundary of the late group is replayed through every crash mode
/// Then reopen keeps the acknowledged first record and all or none of the late group, in order
/// Cases: GROUP-FORMATION, ORACLE-1, ORACLE-2, ORACLE-3, ORACLE-7
#[test]
fn a_group_formed_after_its_linger_ended_fails_and_recovers_whole() {
    bounded(|| {
        let linger = Duration::from_millis(20);
        let sim = Sim::new(47);
        let log = lingering(&sim, linger);
        let flush = Call::new(SEG, Purpose::Group, Op::SyncData);
        let hold = sim.hold(flush.clone());
        let first = append(&log, b"s");
        hold.wait();
        let late = unkeyed(&log, &[b"a", b"b", b"c"]);
        std::thread::sleep(linger);
        sim.inject(flush.clone(), Fault::NoSpace);
        hold.release(None);
        first.wait().unwrap();
        for pending in late {
            assert!(matches!(pending.wait(), Err(CommitError::Indeterminate)));
        }
        assert!(matches!(
            log.append(submit(b"later")),
            Err(AppendError::Poisoned)
        ));
        assert_eq!(log.watermark(), 1);
        let segment = Path::new(SEG);
        assert_eq!(sim.count(segment, Purpose::Group, &Op::Append(vec![])), 2);
        assert_eq!(sim.count(segment, Purpose::Group, &Op::SyncData), 2);
        let (seed, trace) = sim.trace();
        drop(log);
        // The first record is acknowledged, so every replay keeps its flush.
        let acknowledged = trace
            .iter()
            .position(|event| matches!(event, Event::Call(call, None, _) if *call == flush))
            .unwrap()
            + 1;
        for end in acknowledged..=trace.len() {
            for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
                let replay = Sim::replay(seed, &trace[..end]);
                replay.crash(mode, end as u64);
                let recovered = open(&replay);
                let records = recovered.read(0, 5).unwrap();
                assert!(
                    records.len() == 1 || records.len() == 4,
                    "{} records at {end} under {mode:?}",
                    records.len()
                );
                if end == trace.len() && mode == Crash::Kill {
                    assert_eq!(records.len(), 4);
                }
                let streams: [&[u8]; 4] = [b"s", b"a", b"b", b"c"];
                for (offset, record) in records.iter().enumerate() {
                    assert_eq!((record.offset, record.revision), (offset as u64, 0));
                    assert_eq!(record.stream, streams[offset]);
                }
            }
        }
    });
}
