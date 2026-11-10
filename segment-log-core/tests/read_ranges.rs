#[path = "../src/acceptance/support/deadline.rs"]
mod deadline;
#[path = "../src/acceptance/support/roll_size.rs"]
mod roll_size;

use segment_log_core::*;
use std::{
    sync::{
        Barrier,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn append(log: &SegmentLog, start: u64, count: u64) {
    let committed = log
        .append(Submit {
            records: (start..start + count)
                .map(|i| NewRecord {
                    stream: b"s".to_vec(),
                    expected: if i == 0 {
                        ExpectedRevision::Empty
                    } else {
                        ExpectedRevision::At(i - 1)
                    },
                    key: (i % 2 == 0).then(|| i.to_le_bytes().to_vec()),
                    payload: i.to_le_bytes().to_vec(),
                })
                .collect(),
        })
        .unwrap()
        .wait()
        .unwrap();
    assert!(!committed.deduplicated);
    assert_eq!(committed.records.len(), count as usize);
    for (p, i) in committed.records.iter().zip(start..) {
        assert_eq!((p.offset, p.revision), (i, i));
    }
}
fn check(records: &[Record], from: u64, count: usize) {
    assert_eq!(records.len(), count);
    for (record, i) in records.iter().zip(from..) {
        assert_eq!((record.offset, record.revision), (i, i));
        assert_eq!(record.stream, b"s");
        assert_eq!(record.payload, i.to_le_bytes());
        assert_eq!(
            record.key.as_deref(),
            (i % 2 == 0)
                .then_some(i.to_le_bytes())
                .as_ref()
                .map(|key| key.as_slice())
        );
    }
}
fn ranges(log: &SegmentLog) {
    for from in 0..=12 {
        for max in [0, 1, 2, 3, 4, 7, 12, usize::MAX] {
            check(
                &log.read(from, max).unwrap(),
                from,
                max.min(12 - from as usize),
            );
        }
    }
    assert!(log.read(u64::MAX, usize::MAX).unwrap().is_empty());
}

#[test]
fn reads_boundaries_and_snapshot_during_appends() {
    let _deadline = deadline::Deadline::new(Duration::from_secs(30), || {});
    let dir = std::env::temp_dir().join(format!("segment-log-read-ranges-{}", std::process::id()));
    let options = || Options {
        linger: Duration::ZERO,
        roll_bytes: roll_size::roll_bytes(Options::default().roll_bytes),
        ..Options::default()
    };
    let log = SegmentLog::open(&dir, options()).unwrap();
    assert!(log.read(0, usize::MAX).unwrap().is_empty());
    for (start, count) in [(0, 3), (3, 1), (4, 5), (9, 3)] {
        append(&log, start, count);
    }
    ranges(&log);
    drop(log);
    let log = SegmentLog::open(&dir, options()).unwrap();
    ranges(&log);
    let barrier = Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for i in 12..76 {
                barrier.wait();
                append(&log, i, 1);
                barrier.wait();
            }
        });
        for i in 12..76 {
            barrier.wait();
            let records = log.read(0, usize::MAX).unwrap();
            assert!((i as usize..=i as usize + 1).contains(&records.len()));
            check(&records, 0, records.len());
            check(&log.read(3, 4).unwrap(), 3, 4);
            barrier.wait();
        }
    });
    check(&log.read(0, usize::MAX).unwrap(), 0, 76);
    drop(log);
    let log = SegmentLog::open(&dir, options()).unwrap();
    check(&log.read(0, usize::MAX).unwrap(), 0, 76);
    drop(log);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Given a log on real storage and four readers that reread chosen ranges without pause
/// When a caller appends and flushes groups while the readers run
/// Then every read returns its range in log order with each record's own contents,
/// at most `max` records, and nothing past the watermark
/// Cases: READ-BOUNDED
#[test]
fn free_running_readers_get_exact_ranges_while_groups_flush() {
    let _deadline = deadline::Deadline::new(Duration::from_secs(60), || {});
    let dir = std::env::temp_dir().join(format!(
        "segment-log-concurrent-ranges-{}",
        std::process::id()
    ));
    let log = SegmentLog::open(
        &dir,
        Options {
            linger: Duration::ZERO,
            roll_bytes: roll_size::roll_bytes(Options::default().roll_bytes),
            ..Options::default()
        },
    )
    .unwrap();
    append(&log, 0, 8);
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let readers: Vec<_> = (0..4u64)
            .map(|reader| {
                let (log, stop) = (&log, &stop);
                scope.spawn(move || {
                    let mut reads = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        let before = log.watermark();
                        let from = (reads * 2654435761 + reader) % before;
                        let records = log.read(from, 16).unwrap();
                        assert!(records.len() as u64 >= (before - from).min(16));
                        assert!(records.len() <= 16);
                        assert!(from + records.len() as u64 <= log.watermark());
                        check(&records, from, records.len());
                        reads += 1;
                    }
                    reads
                })
            })
            .collect();
        for group in 1..64 {
            append(&log, group * 8, 8);
        }
        stop.store(true, Ordering::Relaxed);
        for reader in readers {
            assert!(reader.join().unwrap() > 0);
        }
    });
    check(&log.read(0, usize::MAX).unwrap(), 0, 512);
    drop(log);
    std::fs::remove_dir_all(dir).unwrap();
}
