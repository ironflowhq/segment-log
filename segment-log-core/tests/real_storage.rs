#[path = "../src/acceptance/support/deadline.rs"]
mod deadline;

use segment_log_core::*;
use std::{sync::mpsc, time::Duration};

fn assert_record(log: &SegmentLog) {
    let records = log.read(0, 100).unwrap();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!((record.offset, record.revision), (0, 0));
    assert_eq!(record.stream, b"s");
    assert_eq!(record.key.as_deref(), Some(b"key".as_slice()));
    assert_eq!(record.payload, [0, 255, 7]);
    assert_eq!(log.watermark(), 1);
    let head = log.stream_head(b"s").unwrap();
    assert_eq!((head.offset, head.revision), (0, 0));
}

/// Given the public production constructor and a fresh temporary directory
/// When a caller appends, reads, drops and reopens the log
/// Then the same full durable record is returned by the real storage engine
/// Cases: ORACLE-1, READ-BOUNDED, OPEN-DIR
#[test]
fn public_open_runs_the_same_engine_on_real_storage() {
    let _deadline = deadline::Deadline::new(Duration::from_secs(10), || {});
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = std::panic::catch_unwind(|| {
            let dir =
                std::env::temp_dir().join(format!("segment-log-slice1-{}", std::process::id()));
            std::fs::create_dir(&dir).unwrap();
            let options = || Options {
                linger: Duration::ZERO,
                ..Options::default()
            };
            let log = SegmentLog::open(dir.join("log"), options()).unwrap();
            let committed = log
                .append(Submit {
                    records: vec![NewRecord {
                        stream: b"s".to_vec(),
                        expected: ExpectedRevision::Empty,
                        key: Some(b"key".to_vec()),
                        payload: vec![0, 255, 7],
                    }],
                })
                .unwrap()
                .wait()
                .unwrap();
            assert_eq!(committed.records.len(), 1);
            assert_eq!(
                (committed.records[0].offset, committed.records[0].revision),
                (0, 0)
            );
            assert!(!committed.deduplicated);
            assert_record(&log);
            drop(log);
            let log = SegmentLog::open(dir.join("log"), options()).unwrap();
            assert_record(&log);
            drop(log);
            std::fs::remove_dir_all(dir).unwrap();
        });
        sender.send(result).unwrap();
    });
    let result = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("scenario watchdog");
    worker.join().unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
