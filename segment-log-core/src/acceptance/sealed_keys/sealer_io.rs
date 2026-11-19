//! The sealer, not the writer, writes and flushes a rolled segment's closing run (ADR 0067).
use super::*;
use crate::storage::{
    Call, Op,
    sim::{Crash, Fault},
};
use std::time::{Duration, Instant};

/// One keyed record per segment, so that only a keyed submit rolls, and unkeyed groups commit
/// into the new segment while the old one's closing run is still being written.
fn open_sealing(sim: &Sim) -> SegmentLog {
    SegmentLog::with_hooks(
        Path::new("/log"),
        Options {
            roll_bytes: crate::codec::ROLL_BYTES_LIMIT,
            ..options()
        },
        Arc::new(sim.clone()),
        Hooks {
            background_seal: true,
            limits: crate::engine::Limits {
                keyed_records: 1,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap()
}

fn unkeyed(n: u8) -> NewRecord {
    NewRecord {
        stream: vec![b'u', n],
        key: None,
        payload: vec![n],
        expected: ExpectedRevision::Empty,
    }
}

fn closing_flush() -> Call {
    Call::new(raw::segment(0), Purpose::Roll, Op::SyncData)
}

/// The answer to `pending`, which must come without anyone releasing a held call.
fn answered(pending: &crate::Pending) -> Result<Committed, CommitError> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(answer) = pending.probe() {
            return answer;
        }
        assert!(
            Instant::now() < deadline,
            "the group waited for the closing run"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Rolls segment 0 with its closing run's first flush held, then commits two unkeyed groups.
fn roll_with_held_flush(sim: &Sim, log: &SegmentLog) -> crate::storage::sim::Hold {
    commit(log, vec![keyed(b"k0")]);
    let held = sim.hold(closing_flush());
    assert_eq!(offsets(&commit(log, vec![keyed(b"k1")])), [1]);
    held.wait();
    for n in 0..2 {
        let pending = log
            .append(Submit {
                records: vec![unkeyed(n)],
            })
            .unwrap();
        assert_eq!(offsets(&answered(&pending).unwrap()), [2 + u64::from(n)]);
    }
    held
}

/// Given a log that rolled, and a closing run whose flush the storage holds
/// When unkeyed submits arrive
/// Then their groups commit while the flush is held, because the sealer, not the writer, waits
/// for it
/// And once it is released, the rolled segment ends in the closing run its groups produce
///
/// Cases: ROLL-BOUNDED
#[test]
fn groups_commit_while_the_sealer_flushes_a_closing_run() {
    bounded(|| {
        let sim = Sim::new(380);
        let log = open_sealing(&sim);
        let held = roll_with_held_flush(&sim, &log);
        held.release(None);
        drop(log);
        let segments = raw::segments(&sim);
        let (groups, run) = raw::split_run(&segments[0].1);
        assert_eq!(run, raw::roll_run(groups, 1));
        let log = open_sealing(&sim);
        assert_eq!(log.watermark(), 4);
        assert_retry(&log, &[b"k0", b"k1"], &[0, 1]);
    });
}

/// Given a log that rolled, and groups committed while the sealer's flush was held
/// When that flush fails
/// Then a later submit gets `Poisoned`, and no flush is retried
/// And after a reopen every acknowledged record is there, and the rolled segment is sealed
///
/// Cases: ORACLE-7, ROLL-BOUNDED
#[test]
fn a_failed_closing_run_flush_poisons_the_writer() {
    bounded(|| {
        let sim = Sim::new(381);
        let log = open_sealing(&sim);
        let held = roll_with_held_flush(&sim, &log);
        held.release(Some(Fault::error(std::io::ErrorKind::Other).unwrap()));
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut n = 2;
        loop {
            match log.append(Submit {
                records: vec![unkeyed(n)],
            }) {
                Err(AppendError::Poisoned) => break,
                Ok(pending) => match pending.wait() {
                    Err(CommitError::Poisoned) => break,
                    Ok(_) => n += 1,
                    Err(other) => panic!("{other:?}"),
                },
                Err(other) => panic!("{other:?}"),
            }
            assert!(
                Instant::now() < deadline,
                "the writer never saw the failure"
            );
        }
        let acked = 2 + u64::from(n);
        assert_eq!(
            sim.count(Path::new(&raw::segment(0)), Purpose::Roll, &Op::SyncData),
            1
        );
        drop(log);
        let log = open_sealing(&sim);
        assert_eq!(log.watermark(), acked);
        assert_retry(&log, &[b"k0", b"k1"], &[0, 1]);
        let segments = raw::segments(&sim);
        let (groups, run) = raw::split_run(&segments[0].1);
        assert_eq!(run, raw::roll_run(groups, 1));
    });
}

/// Given groups committed into the new segment while the sealer writes the old one's closing run
/// When the process or the power goes at the sealer's first flush, or between its two flushes
/// Then a reopen keeps every acknowledged record and rebuilds the closing run its groups produce
///
/// Cases: KEY-FRAMES-CRASH, ORACLE-6, KEY-LIFETIME
#[test]
fn a_crash_while_the_sealer_flushes_keeps_every_ack() {
    bounded(|| {
        for second in [false, true] {
            for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
                let label = format!("second flush {second}, {mode:?}");
                let sim = Sim::new(382);
                let log = open_sealing(&sim);
                let mut held = roll_with_held_flush(&sim, &log);
                if second {
                    let next = sim.hold(closing_flush());
                    held.release(None);
                    next.wait();
                    held = next;
                }
                sim.crash(mode, 382);
                held.release(None);
                drop(log);
                let log = open_sealing(&sim);
                assert_eq!(log.watermark(), 4, "{label}");
                assert_retry(&log, &[b"k0", b"k1"], &[0, 1]);
                drop(log);
                let segments = raw::segments(&sim);
                let (groups, run) = raw::split_run(&segments[0].1);
                assert_eq!(run, raw::roll_run(groups, 1), "{label}");
            }
        }
    });
}

/// Given a sealer whose flush of a rolled segment the storage holds
/// When a group flush fails, so that the writer is poisoned, and the log is dropped
/// Then the drop waits for the sealer, so no write to the log's files outlives the log and its
/// directory lock
/// And after a reopen every acknowledged record is there, and the rolled segment is sealed
///
/// Cases: ORACLE-7, KEY-FRAMES-CRASH
#[test]
fn a_dropped_poisoned_log_waits_for_its_sealer() {
    bounded(|| {
        let sim = Sim::new(383);
        let log = open_sealing(&sim);
        let held = roll_with_held_flush(&sim, &log);
        sim.inject(
            Call::new(ANY_SEG, Purpose::Group, Op::SyncData),
            Fault::error(std::io::ErrorKind::Other).unwrap(),
        );
        let failed = log
            .append(Submit {
                records: vec![unkeyed(9)],
            })
            .unwrap();
        assert!(matches!(failed.wait(), Err(CommitError::Indeterminate)));
        let (dropped, done) = std::sync::mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(log);
            dropped.send(()).unwrap();
        });
        assert!(
            done.recv_timeout(Duration::from_millis(200)).is_err(),
            "the log closed while its sealer was still writing"
        );
        held.release(None);
        done.recv_timeout(Duration::from_secs(5)).unwrap();
        dropper.join().unwrap();
        let log = open_sealing(&sim);
        assert!(log.watermark() >= 4);
        assert_retry(&log, &[b"k0", b"k1"], &[0, 1]);
        drop(log);
        let segments = raw::segments(&sim);
        let (groups, run) = raw::split_run(&segments[0].1);
        assert_eq!(run, raw::roll_run(groups, 1));
    });
}
