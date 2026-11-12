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

const MANY_SEGMENTS_CHILD: &str = "a_log_of_many_segments_runs_under_a_small_open_file_limit";

fn write_and_reopen_many_segments(dir: &std::path::Path) {
    let options = || Options {
        linger: Duration::ZERO,
        roll_bytes: 1,
        ..Options::default()
    };
    let count = 100;
    let log = SegmentLog::open(dir, options()).unwrap();
    for i in 0..count {
        let committed = log
            .append(Submit {
                records: vec![NewRecord {
                    stream: format!("s{i}").into_bytes(),
                    expected: ExpectedRevision::Empty,
                    key: None,
                    payload: vec![7],
                }],
            })
            .unwrap_or_else(|e| panic!("append {i} refused: {e:?}"))
            .wait()
            .unwrap_or_else(|e| panic!("append {i} failed: {e:?}"));
        assert_eq!(committed.records[0].offset, i);
    }
    assert_eq!(log.read(0, 1000).unwrap().len(), count as usize);
    drop(log);
    let log = SegmentLog::open(dir, options()).unwrap();
    assert_eq!(log.watermark(), count);
    assert_eq!(log.read(0, 1000).unwrap().len(), count as usize);
    let segments = std::fs::read_dir(dir)
        .unwrap()
        .filter(|e| e.as_ref().unwrap().path().extension() == Some("seg".as_ref()))
        .count();
    assert!(segments >= count as usize, "{segments} segments");
}

/// Given a process whose open-file limit is far below the number of segments it writes
/// When it appends one record per segment, reads them all, drops the log and reopens it
/// Then every append succeeds and both reads return every record
///
/// Cases: ORACLE-1, OPEN-LAYOUT, HISTORY
#[test]
fn a_log_of_many_segments_runs_under_a_small_open_file_limit() {
    if let Some(dir) = std::env::var_os("SEGMENT_LOG_MANY_SEGMENTS_DIR") {
        write_and_reopen_many_segments(std::path::Path::new(&dir));
        return;
    }
    let dir = std::env::temp_dir().join(format!("segment-log-many-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(r#"ulimit -n 32 && exec "$0" --exact "$1" --nocapture --test-threads=1"#)
        .arg(std::env::current_exe().unwrap())
        .arg(MANY_SEGMENTS_CHILD)
        .env("SEGMENT_LOG_MANY_SEGMENTS_DIR", dir.join("log"))
        .status()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(status.success(), "child under a 32-file limit: {status}");
}

fn record(i: u64) -> NewRecord {
    NewRecord {
        stream: format!("s{}", i % 2).into_bytes(),
        expected: if i < 2 {
            ExpectedRevision::Empty
        } else {
            ExpectedRevision::At(i / 2 - 1)
        },
        key: Some(format!("k{i}").into_bytes()),
        payload: i.to_le_bytes().to_vec(),
    }
}

/// Given a log on real storage in a fresh directory, whose records span several segments
/// When it is sealed and dropped, and `LogReader::open` opens the directory
/// Then the reader gives the watermark, the records and the stream heads that the sealed log gave
/// And `SegmentLog::open` on the sealed log refuses an append with `Sealed`
/// And `LogReader::open` on a log that was dropped without a seal fails with `NotSealed`
///
/// Cases: READER, HISTORY
#[test]
fn a_sealed_log_reads_back_through_a_log_reader() {
    let _deadline = deadline::Deadline::new(Duration::from_secs(10), || {});
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = std::panic::catch_unwind(|| {
            let root =
                std::env::temp_dir().join(format!("segment-log-sealed-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir(&root).unwrap();
            let options = || Options {
                linger: Duration::ZERO,
                roll_bytes: 1,
                ..Options::default()
            };
            let log = SegmentLog::open(root.join("sealed"), options()).unwrap();
            for i in 0..6 {
                let answer = log
                    .append(Submit {
                        records: vec![record(i)],
                    })
                    .unwrap()
                    .wait()
                    .unwrap();
                assert_eq!(answer.records[0].offset, i);
            }
            log.seal().unwrap();
            let sealed = (
                log.watermark(),
                format!("{:?}", log.read(0, 100).unwrap()),
                format!("{:?}", [log.stream_head(b"s0"), log.stream_head(b"s1")]),
            );
            drop(log);

            let reader = LogReader::open(root.join("sealed")).unwrap();
            let read = (
                reader.watermark(),
                format!("{:?}", reader.read(0, 100).unwrap()),
                format!(
                    "{:?}",
                    [reader.stream_head(b"s0"), reader.stream_head(b"s1")]
                ),
            );
            assert_eq!(read, sealed);
            assert_eq!(read.0, 6);
            let reopened = SegmentLog::open(root.join("sealed"), options()).unwrap();
            assert!(matches!(
                reopened.append(Submit {
                    records: vec![record(6)],
                }),
                Err(AppendError::Sealed)
            ));

            let unsealed = SegmentLog::open(root.join("unsealed"), options()).unwrap();
            unsealed
                .append(Submit {
                    records: vec![record(0)],
                })
                .unwrap()
                .wait()
                .unwrap();
            drop(unsealed);
            assert!(matches!(
                LogReader::open(root.join("unsealed")),
                Err(OpenError::NotSealed)
            ));
            std::fs::remove_dir_all(root).unwrap();
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
