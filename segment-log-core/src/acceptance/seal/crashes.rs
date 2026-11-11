use super::*;
use crate::storage::sim::{Crash, Fault};

const MODES: [Crash; 3] = [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn];

fn keyed(payload: &[u8], revision: Option<u64>) -> NewRecord {
    record(b"s", Some(payload), payload, revision)
}

fn seal_steps(groups: &[u8]) -> Vec<Call> {
    let sealed = [groups.to_vec(), raw::seal_run(groups, 1)].concat();
    raw::frames(&sealed)
        .into_iter()
        .skip(raw::frames(groups).len())
        .map(|frame| Op::Append(frame.to_vec()))
        .chain([Op::SyncData])
        .map(|op| Call::new(raw::segment(0), Purpose::Seal, op))
        .collect()
}

/// Given two acknowledged keyed groups
/// When `seal` runs and a crash overtakes the write of its footer, its seal frame or its locator,
/// or its flush, or comes after `seal` returned
/// Then `seal` returns `Poisoned`, unless it had returned `Ok`
/// When the log reopens
/// Then both acknowledged records are present at their offsets
/// And the log is sealed only if the segment ends in the whole closing run, which it must once the
/// seal's flush returned, or once a kill kept every write before the flush
/// And otherwise open cut the run, and the log is writable: a retry of each key returns its
/// original position, and the next append takes the next offset
/// And a power loss after that open gives the same log
///
/// Cases: RECOVERY-FIXTURES, ORACLE-1, ORACLE-2, ORACLE-3, ORACLE-4, ORACLE-5, ORACLE-6
#[test]
fn a_crash_at_each_step_of_a_seal_keeps_every_acknowledged_record() {
    bounded(|| {
        for step in 0..=4 {
            for mode in MODES {
                for seed in 0..4 {
                    let label = format!("step {step} {mode:?} seed {seed}");
                    let sim = Sim::new(850);
                    let log = Arc::new(open_with(&sim, unrolled()));
                    send(&log, vec![keyed(b"a", None)]).wait().unwrap();
                    send(&log, vec![keyed(b"b", Some(0))]).wait().unwrap();
                    let groups = raw::segments(&sim)[0].1.clone();
                    if let Some(call) = seal_steps(&groups).get(step) {
                        let hold = sim.hold(call.clone());
                        let sealing = seal_on_a_thread(&log);
                        hold.wait();
                        sim.crash(mode, seed);
                        hold.release(None);
                        let result = sealing.join().unwrap();
                        assert!(matches!(result, Err(SealError::Poisoned)), "{label}");
                    } else {
                        assert!(log.seal().is_ok(), "{label}");
                        sim.crash(mode, seed);
                    }
                    drop(log);

                    let log = open_with(&sim, unrolled());
                    let kept = log.read(0, 10).unwrap();
                    assert_records(
                        &kept,
                        &[
                            stored(b"s", Some(b"a"), b"a", 0, 0),
                            stored(b"s", Some(b"b"), b"b", 1, 1),
                        ],
                    );
                    let sealed = sealed_on_disk(&sim, 1);
                    if step == 4 || (step == 3 && mode == Crash::Kill) {
                        assert!(sealed, "{label}");
                    } else if step < 3 {
                        assert!(!sealed, "{label}");
                    }
                    if sealed {
                        assert_sealed(&log);
                    } else {
                        assert_eq!(raw::segments(&sim), [(0, groups.clone())], "{label}");
                        for (n, payload) in [b"a", b"b"].into_iter().enumerate() {
                            let revision = n.checked_sub(1).map(|r| r as u64);
                            let retry = send(&log, vec![keyed(payload, revision)]).wait().unwrap();
                            assert!(retry.deduplicated, "{label}");
                            assert_eq!(retry.records[0].offset, n as u64, "{label}");
                        }
                    }
                    drop(log);
                    sim.crash(Crash::PowerLoss, seed);
                    let log = open_with(&sim, unrolled());
                    assert_eq!(log.read(0, 10).unwrap().len(), 2, "{label}");
                    assert_eq!(sealed_on_disk(&sim, 1), sealed, "{label}");
                    if !sealed {
                        let answer = send(&log, vec![keyed(b"c", Some(1))]).wait().unwrap();
                        assert_eq!(answer.records[0].offset, 2, "{label}");
                    }
                }
            }
        }
    });
}

/// Given a log whose active segment has no group frame: a new log, and a log whose active segment
/// open created after a crash that followed a roll's flush
/// When the seal's flush fails, or a kill comes during that flush
/// Then `seal` returns `Poisoned`
/// When the log reopens, loses power and reopens again
/// Then it is sealed: an append gets `Sealed`, and it reads every record before that segment
///
/// Cases: RECOVERY-FIXTURES, ORACLE-5
#[test]
fn a_seal_with_no_group_frame_survives_a_failed_flush_or_a_kill() {
    bounded(|| {
        let groups = [raw::header(raw::VERSION, 0, 1), raw::group(1, 0, &[b"a"])].concat();
        for rolled in [false, true] {
            for kill in [false, true] {
                let label = format!("rolled {rolled} kill {kill}");
                let sim = Sim::new(851);
                if rolled {
                    raw::install_segments(&sim, &[(0, raw::rolled(groups.clone(), 1))]);
                }
                let log = Arc::new(open_with(&sim, unrolled()));
                if kill {
                    let hold = sim.hold(seal_flush());
                    let sealing = seal_on_a_thread(&log);
                    hold.wait();
                    sim.crash(Crash::Kill, 851);
                    hold.release(None);
                    assert!(matches!(sealing.join().unwrap(), Err(SealError::Poisoned)));
                } else {
                    sim.inject(seal_flush(), Fault::NoSpace);
                    assert!(matches!(log.seal(), Err(SealError::Poisoned)), "{label}");
                }
                drop(log);
                let log = open_with(&sim, unrolled());
                assert_sealed(&log);
                drop(log);
                sim.crash(Crash::PowerLoss, 851);
                let log = open_with(&sim, unrolled());
                assert_sealed(&log);
                assert_eq!(
                    log.read(0, 10).unwrap().len(),
                    usize::from(rolled),
                    "{label}"
                );
                assert!(sealed_on_disk(&sim, 1 + u64::from(rolled)), "{label}");
            }
        }
    });
}
