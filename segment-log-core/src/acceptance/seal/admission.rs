use super::*;
use crate::storage::sim::{Crash, Event, Fault};

fn assert_poisoned(log: &SegmentLog) {
    let [valid, invalid, large] = refusals(log).map(Result::err);
    assert!(matches!(valid, Some(AppendError::Poisoned)), "{valid:?}");
    assert!(
        matches!(invalid, Some(AppendError::Invalid(_))),
        "{invalid:?}"
    );
    assert!(matches!(large, Some(AppendError::TooLarge)), "{large:?}");
}

/// Given a log with a queue of one, whose group flush is held with a submit in it, and a second
/// submit queued behind that flush
/// When `seal` starts and cannot queue its marker yet
/// Then an append gets `Sealed` although the queue is full, and `Invalid` and `TooLarge` still
/// come first
/// When the held flush is released
/// Then both submits are acknowledged, and `seal` returns `Ok`
///
/// Cases: ADM-PRECEDENCE, ADM-BUDGETS
#[test]
fn a_sealing_log_refuses_with_sealed_before_full() {
    bounded(|| {
        let sim = Sim::new(840);
        let log = Arc::new(open_with(
            &sim,
            Options {
                queue_capacity: 1,
                ..options()
            },
        ));
        let flush = sim.hold(group_flush());
        let held = send(&log, vec![record(b"a", None, b"x", None)]);
        flush.wait();
        let queued = send(&log, vec![record(b"b", None, b"x", None)]);
        let [full, ..] = refusals(&log).map(Result::err);
        assert!(matches!(full, Some(AppendError::Full)), "{full:?}");
        let sealing = seal_on_a_thread(&log);
        loop {
            match log
                .append(submit(vec![record(b"c", None, b"x", None)]))
                .err()
            {
                Some(AppendError::Full) => std::thread::yield_now(),
                Some(AppendError::Sealed) => break,
                other => panic!("{other:?}"),
            }
        }
        assert_sealed(&log);
        flush.release(None);
        assert_eq!(held.wait().unwrap().records[0].offset, 0);
        assert_eq!(queued.wait().unwrap().records[0].offset, 1);
        assert!(sealing.join().unwrap().is_ok());
        assert_sealed(&log);
    });
}

/// Given a log whose group flush failed, so that the writer is poisoned
/// When `seal` is called, and called again
/// Then each call returns `Poisoned`, and an append gets `Poisoned`, not `Sealed`
/// And the acknowledged record stays readable, and the writer writes no closing frame
/// When the log reopens
/// Then it is writable and keeps the acknowledged record
///
/// Cases: ADM-PRECEDENCE, ORACLE-7, ORACLE-1
#[test]
fn a_poisoned_writer_answers_seal_and_appends_with_poisoned() {
    bounded(|| {
        let sim = Sim::new(841);
        let log = open_with(&sim, unrolled());
        send(&log, vec![record(b"s", None, b"a", None)])
            .wait()
            .unwrap();
        sim.inject(group_flush(), Fault::NoSpace);
        let failed = send(&log, vec![record(b"s", None, b"b", Some(0))]).wait();
        assert!(matches!(failed, Err(CommitError::Indeterminate)));
        for _ in 0..2 {
            assert!(matches!(log.seal(), Err(SealError::Poisoned)));
            assert_poisoned(&log);
        }
        assert_records(&log.read(0, 10).unwrap(), &[stored(b"s", None, b"a", 0, 0)]);
        assert_eq!(
            sim.count(Path::new(ANY_SEG), Purpose::Seal, &Op::Append(vec![])),
            0
        );
        drop(log);
        sim.crash(Crash::PowerLoss, 841);
        let log = open_with(&sim, unrolled());
        assert_eq!(log.read(0, 10).unwrap()[0].payload, b"a");
        let next = log.watermark();
        let answer = send(&log, vec![record(b"t", None, b"c", None)])
            .wait()
            .unwrap();
        assert_eq!(answer.records[0].offset, next);
    });
}

/// Given two acknowledged groups
/// When the write of the seal's footer, seal frame or locator fails or panics, or its flush does
/// Then `seal` returns `Poisoned`, a second `seal` does too, and an append gets `Poisoned`
/// And the acknowledged records stay readable, and the writer makes no further storage call that
/// writes or flushes
/// When the log reopens, loses power and reopens again
/// Then both acknowledged records are present
/// And the log is sealed only if all three closing frames were written, as they were when only the
/// flush failed, and otherwise it is writable and the next append takes the next offset
///
/// Cases: ADM-PRECEDENCE, ORACLE-7, ORACLE-1, ORACLE-5, RECOVERY-FIXTURES
#[test]
fn a_failed_or_panicked_seal_poisons_the_writer() {
    bounded(|| {
        let other = || Fault::error(std::io::ErrorKind::Other).unwrap();
        for step in 0..4 {
            let faults = if step < 3 {
                vec![
                    Fault::Partial(1),
                    Fault::Partial(20),
                    Fault::NoSpace,
                    other(),
                    Fault::Panic,
                ]
            } else {
                vec![Fault::NoSpace, other(), Fault::Panic]
            };
            for fault in faults {
                let label = format!("step {step} {fault:?}");
                let sim = Sim::new(842);
                let log = open_with(&sim, unrolled());
                send(&log, vec![record(b"s", Some(b"a"), b"a", None)])
                    .wait()
                    .unwrap();
                send(&log, vec![record(b"s", Some(b"b"), b"b", Some(0))])
                    .wait()
                    .unwrap();
                let groups = raw::segments(&sim)[0].1.clone();
                let run = raw::seal_run(&groups, 1);
                let frames = raw::frames(&[groups.clone(), run].concat())[2..]
                    .iter()
                    .map(|frame| Op::Append(frame.to_vec()))
                    .chain([Op::SyncData])
                    .collect::<Vec<_>>();
                let call = Call::new(raw::segment(0), Purpose::Seal, frames[step].clone());
                sim.inject(call.clone(), fault.clone());
                assert!(matches!(log.seal(), Err(SealError::Poisoned)), "{label}");
                assert!(matches!(log.seal(), Err(SealError::Poisoned)), "{label}");
                assert_poisoned(&log);
                assert_eq!(log.read(0, 10).unwrap().len(), 2, "{label}");
                drop(log);
                let trace = sim.trace().1;
                let failed = trace
                    .iter()
                    .position(|event| matches!(event, Event::Call(c, Some(_), _) if c == &call))
                    .unwrap();
                assert!(
                    !trace[failed + 1..].iter().any(|event| matches!(
                        event,
                        Event::Call(c, _, _) if !matches!(c.op, Op::Read(..))
                    )),
                    "{label}: the poisoned writer called the storage again"
                );

                let log = open_with(&sim, unrolled());
                assert_eq!(log.read(0, 10).unwrap().len(), 2, "{label}");
                drop(log);
                sim.crash(Crash::PowerLoss, 842);
                let log = open_with(&sim, unrolled());
                assert_eq!(log.read(0, 10).unwrap().len(), 2, "{label}");
                let written = step == 3;
                assert_eq!(sealed_on_disk(&sim, 1), written, "{label}");
                if written {
                    assert_sealed(&log);
                } else {
                    let answer = send(&log, vec![record(b"s", None, b"c", Some(1))])
                        .wait()
                        .unwrap();
                    assert_eq!(answer.records[0].offset, 2, "{label}");
                }
            }
        }
    });
}

/// Given a log whose writer thread panicked at a group flush, which an append has observed as
/// `Poisoned`
/// When appends keep arriving while `seal` closes admission
/// Then every append gets `Poisoned`, never `Sealed`, and `seal` returns `Poisoned`
///
/// Cases: ADM-PRECEDENCE
#[test]
fn a_panicked_writer_answers_appends_racing_seal_with_poisoned() {
    bounded(|| {
        for seed in 0..50 {
            let sim = Sim::new(850 + seed);
            let log = std::sync::Arc::new(open_with(&sim, unrolled()));
            sim.inject(group_flush(), Fault::Panic);
            let failed = send(&log, vec![record(b"s", None, b"a", None)]).wait();
            assert!(matches!(failed, Err(CommitError::Indeterminate)));
            while !matches!(
                log.append(Submit {
                    records: vec![record(b"s", None, b"b", None)]
                }),
                Err(AppendError::Poisoned)
            ) {}
            let racer = {
                let log = log.clone();
                std::thread::spawn(move || {
                    for _ in 0..2000 {
                        let answer = log.append(Submit {
                            records: vec![record(b"s", None, b"c", None)],
                        });
                        assert!(
                            matches!(answer, Err(AppendError::Poisoned)),
                            "seed {seed}: {:?}",
                            answer.err()
                        );
                    }
                })
            };
            assert!(matches!(log.seal(), Err(SealError::Poisoned)));
            racer.join().unwrap();
        }
    });
}
