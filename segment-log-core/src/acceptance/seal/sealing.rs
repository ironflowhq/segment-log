use super::*;
use crate::storage::sim::Fault;
use std::{
    sync::{
        Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn head(log: &SegmentLog, stream: &[u8]) -> Option<(u64, u64)> {
    log.stream_head(stream).map(|h| (h.offset, h.revision))
}

/// Given a log with one acknowledged record, whose next group flush is held with a submit in it,
/// and two submits queued behind that flush
/// When `seal` starts on another thread, and the held flush is released
/// Then all three submits are acknowledged, and an append gets `Sealed`, while the seal's flush is
/// held
/// And `seal` returns `Ok` once that flush succeeds
/// And the segment ends in its groups and then a seal's closing run, whose footer names each
/// stream's last record and names no key filter, and the seal wrote that run and flushed it once
/// And `read`, `watermark` and `stream_head` keep answering, and a second `seal` returns `Ok`
///
/// Cases: ADM-PRECEDENCE, HISTORY, ORACLE-1
#[test]
fn queued_submits_commit_before_the_seal_run() {
    bounded(|| {
        let sim = Sim::new(820);
        let log = Arc::new(open_with(&sim, unrolled()));
        send(&log, vec![record(b"a", None, b"0", None)])
            .wait()
            .unwrap();
        let flush = sim.hold(group_flush());
        let held = send(&log, vec![record(b"b", None, b"1", None)]);
        flush.wait();
        let queued = [
            send(&log, vec![record(b"a", None, b"2", Some(0))]),
            send(&log, vec![record(b"c", None, b"3", None)]),
        ];
        let sealing_flush = sim.hold(seal_flush());
        let sealing = seal_on_a_thread(&log);
        flush.release(None);
        sealing_flush.wait();
        let offsets: Vec<_> = [held]
            .into_iter()
            .chain(queued)
            .map(|pending| pending.wait().unwrap().records[0].offset)
            .collect();
        assert_eq!(offsets, [1, 2, 3]);
        assert_sealed(&log);
        sealing_flush.release(None);
        assert!(sealing.join().unwrap().is_ok());

        let segments = raw::segments(&sim);
        assert_eq!(segments.len(), 1);
        let (groups, run) = raw::split_run(&segments[0].1);
        assert_eq!(run, raw::seal_run(groups, 1));
        assert_eq!(
            sim.count(Path::new(ANY_SEG), Purpose::Seal, &Op::Append(vec![])),
            3
        );
        assert_eq!(
            sim.count(Path::new(ANY_SEG), Purpose::Seal, &Op::SyncData),
            1
        );
        assert_records(
            &log.read(0, 10).unwrap(),
            &[
                stored(b"a", None, b"0", 0, 0),
                stored(b"b", None, b"1", 1, 0),
                stored(b"a", None, b"2", 2, 1),
                stored(b"c", None, b"3", 3, 0),
            ],
        );
        assert_eq!(log.watermark(), 4);
        assert_eq!(
            [b"a", b"b", b"c"].map(|s| head(&log, s)),
            [Some((2, 1)), Some((1, 0)), Some((3, 0))]
        );
        assert!(log.seal().is_ok());
        assert_sealed(&log);
        assert_eq!(raw::segments(&sim), segments);
    });
}

/// Given a log with a linger of one minute
/// When a submit is admitted and `seal` follows at once
/// Then the submit is acknowledged and `seal` returns long before the linger ends, because the seal
/// ends the group's gather
///
/// Cases: GROUP-FORMATION, ADM-PRECEDENCE
#[test]
fn a_seal_does_not_wait_for_the_linger() {
    bounded(|| {
        let sim = Sim::new(821);
        let log = open_with(
            &sim,
            Options {
                linger: Duration::from_secs(60),
                ..options()
            },
        );
        let start = Instant::now();
        let pending = send(&log, vec![record(b"s", None, b"x", None)]);
        assert!(log.seal().is_ok());
        assert_eq!(pending.wait().unwrap().records[0].offset, 0);
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
    });
}

/// Given a new log with no record, and a log whose active segment open created after a crash that
/// followed a roll's flush, so that it has no group frame
/// When the log is sealed
/// Then the active segment holds its header and a seal's closing run whose footer names no stream
/// When the log reopens
/// Then it is read-only, and reads every record before that segment with the same stream head
///
/// Cases: RECOVERY-FIXTURES, HISTORY, KEY-FRAMES-CRASH
#[test]
fn a_segment_with_no_group_frame_seals() {
    bounded(|| {
        let sim = Sim::new(822);
        let log = open_with(&sim, unrolled());
        assert!(log.seal().is_ok());
        assert_eq!(log.watermark(), 0);
        let header = raw::segments(&sim)[0].1[..64].to_vec();
        assert_eq!(
            raw::segments(&sim),
            [(0, [header.clone(), raw::seal_run(&header, 1)].concat())]
        );
        drop(log);
        let log = open_with(&sim, unrolled());
        assert_sealed(&log);
        assert!(log.read(0, 10).unwrap().is_empty());

        let groups = [
            raw::header(raw::VERSION, 0, 1),
            raw::group(1, 0, &[b"a"]),
            raw::group(1, 1, &[b"b", b"c"]),
        ]
        .concat();
        let rolled = raw::rolled(groups, 1);
        let sim = Sim::new(822);
        raw::install_segments(&sim, &[(0, rolled.clone())]);
        let log = open_with(&sim, unrolled());
        assert!(log.seal().is_ok());
        let header = raw::header(raw::VERSION, 3, 2);
        let sealed = [header.clone(), raw::seal_run(&header, 2)].concat();
        assert_eq!(raw::segments(&sim), [(0, rolled), (3, sealed)]);
        drop(log);
        let log = open_with(&sim, unrolled());
        assert_sealed(&log);
        assert_eq!(log.read(0, 10).unwrap().len(), 3);
        assert_eq!(head(&log, b"s"), Some((2, 2)));
    });
}

/// Given a log with an acknowledged record
/// When two threads call `seal` at the same time
/// Then both get `Ok`, and the log holds one seal's closing run, written and flushed once
///
/// Cases: ADM-PRECEDENCE
#[test]
fn concurrent_seals_write_one_closing_run() {
    bounded(|| {
        let sim = Sim::new(823);
        let log = Arc::new(open_with(&sim, unrolled()));
        send(&log, vec![record(b"s", None, b"x", None)])
            .wait()
            .unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let sealers: Vec<_> = (0..2)
            .map(|_| {
                let (log, barrier) = (log.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    log.seal()
                })
            })
            .collect();
        for sealer in sealers {
            assert!(sealer.join().unwrap().is_ok());
        }
        assert!(sealed_on_disk(&sim, 1));
        assert_eq!(
            sim.count(Path::new(ANY_SEG), Purpose::Seal, &Op::Append(vec![])),
            3
        );
        assert_eq!(
            sim.count(Path::new(ANY_SEG), Purpose::Seal, &Op::SyncData),
            1
        );
    });
}

const APPENDERS: usize = 8;

#[derive(Default)]
struct Appended {
    acked: Vec<(u64, Vec<u8>)>,
    unknown: usize,
}

fn append_until_refused(
    log: &SegmentLog,
    name: usize,
    admitted: &AtomicUsize,
    sealed: &AtomicBool,
    fault: bool,
) -> Appended {
    let mut appended = Appended::default();
    for i in 0.. {
        let stream = format!("t{name}-{i}").into_bytes();
        let after_seal = sealed.load(Ordering::Acquire);
        match log.append(submit(vec![record(&stream, None, b"x", None)])) {
            Ok(pending) => {
                assert!(!after_seal, "admitted after seal returned");
                admitted.fetch_add(1, Ordering::AcqRel);
                match pending.wait() {
                    Ok(answer) => appended.acked.push((answer.records[0].offset, stream)),
                    Err(CommitError::Indeterminate | CommitError::Poisoned) if fault => {
                        appended.unknown += 1
                    }
                    Err(other) => panic!("answered {other:?}"),
                }
            }
            Err(AppendError::Sealed) => {
                let again = log.append(submit(vec![record(b"s", None, b"x", None)]));
                match again.err() {
                    Some(AppendError::Sealed) => {}
                    // A failing flush can poison the writer between the two appends, and Poisoned
                    // comes before Sealed.
                    Some(AppendError::Poisoned) if fault => {}
                    other => panic!("refused again with {other:?}"),
                }
                break;
            }
            Err(AppendError::Poisoned) if fault => break,
            Err(other) => panic!("refused with {other:?}"),
        }
    }
    appended
}

fn race(sim: &Sim, fault: bool) -> Appended {
    let log = Arc::new(open(sim));
    let admitted = Arc::new(AtomicUsize::new(0));
    let sealed = Arc::new(AtomicBool::new(false));
    let appenders: Vec<_> = (0..APPENDERS)
        .map(|name| {
            let (log, admitted, sealed) = (log.clone(), admitted.clone(), sealed.clone());
            std::thread::spawn(move || append_until_refused(&log, name, &admitted, &sealed, fault))
        })
        .collect();
    while admitted.load(Ordering::Acquire) < 2 * APPENDERS {
        std::thread::yield_now();
    }
    let flush = sim.hold(group_flush());
    flush.wait();
    let sealing = seal_on_a_thread(&log);
    flush.release(fault.then_some(Fault::NoSpace));
    let result = sealing.join().unwrap();
    sealed.store(true, Ordering::Release);
    assert_eq!(result.is_ok(), !fault, "{result:?}");
    let mut all = Appended::default();
    for appender in appenders {
        let appended = appender.join().unwrap();
        all.acked.extend(appended.acked);
        all.unknown += appended.unknown;
    }
    all.acked.sort();
    all
}

/// Given eight threads that each append, one submit at a time, until they are refused
/// When `seal` starts while a group flush is held, and that flush succeeds
/// Then every admitted submit is acknowledged, every refusal is `Sealed`, no submit is admitted
/// once `seal` has returned, and nothing waits forever
/// And the log reopens read-only with exactly the acknowledged records
/// When that flush fails instead
/// Then `seal` returns `Poisoned`, each admitted submit is acknowledged or gets `Indeterminate` or
/// `Poisoned`, each refusal is `Sealed` or `Poisoned`, and nothing waits forever
/// And the reopened log keeps every acknowledged record
///
/// Cases: ADM-PRECEDENCE, ORACLE-1, ORACLE-2, ORACLE-7
#[test]
fn appends_that_race_seal_are_committed_or_refused_with_sealed() {
    bounded(|| {
        for round in 0..4 {
            for fault in [false, true] {
                let label = format!("round {round} fault {fault}");
                let sim = Sim::new(824 + round);
                let appended = race(&sim, fault);
                let log = open(&sim);
                let records = log.read(0, usize::MAX).unwrap();
                if fault {
                    assert!(appended.unknown > 0, "{label}: the fault hit admitted work");
                } else {
                    assert_sealed(&log);
                    assert_eq!(records.len(), appended.acked.len(), "{label}");
                }
                for (offset, stream) in &appended.acked {
                    let record = records.get(*offset as usize);
                    assert_eq!(record.map(|r| &r.stream), Some(stream), "{label}");
                }
            }
        }
    });
}

/// Given a log that rolled many times, with keyed and unkeyed records, and a stream that appends
/// throughout
/// When it is sealed, dropped and opened again
/// Then the sealed handle and the read-only open give the same watermark, the same head for every
/// stream and the same records as before the seal
/// And the read-only open reads no key filter, and `seal` on it returns `Ok`
/// And no segment file is deleted or changed by the open
///
/// Cases: HISTORY, ORACLE-12, ORACLE-13, ORACLE-6
#[test]
fn a_sealed_log_keeps_its_whole_history() {
    bounded(|| {
        let sim = Sim::new(825);
        let log = open_with(&sim, rolling());
        fill(&log);
        let before = observe_log(&log);
        assert!(log.seal().is_ok());
        assert_eq!(observe_log(&log), before);
        drop(log);
        let segments = raw::segments(&sim);
        assert!(segments.len() > 5, "{} segments", segments.len());

        let filters = key_filter_reads(&sim);
        let log = open(&sim);
        assert_eq!(key_filter_reads(&sim), filters);
        assert_eq!(observe_log(&log), before);
        assert!(log.seal().is_ok());
        assert_sealed(&log);
        drop(log);
        assert_eq!(raw::segments(&sim), segments);
    });
}
