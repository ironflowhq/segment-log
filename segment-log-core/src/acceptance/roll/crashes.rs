use super::*;
use crate::acceptance::support::bounded;
use crate::storage::{
    Call, Op, Purpose,
    sim::{Crash, Event, Fault, Outcome},
};

const MODES: [Crash; 3] = [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn];

fn on_s(payload: &[u8], revision: Option<u64>) -> NewRecord {
    record(b"s", None, payload, revision)
}

/// Two acknowledged groups: `a` in segment 0 and `b` in segment 1, which has reached the roll size.
fn two_groups(sim: &Sim) -> SegmentLog {
    let log = open_rolling(sim, EVERY_GROUP);
    send(&log, vec![on_s(b"a", None)]).wait().unwrap();
    send(&log, vec![on_s(b"b", Some(0))]).wait().unwrap();
    assert_eq!(bases(sim), [0, 1]);
    log
}

/// The storage calls of the roll into segment 2, in order.
fn roll_steps(sim: &Sim) -> [Call; 4] {
    let (log_id, _, _) = raw::header_fields(&raw::segments(sim)[0].1);
    let header = raw::with_log_id(raw::header(0, 2, 1), log_id);
    [
        Call::new(raw::segment(2), Purpose::Roll, Op::Create),
        Call::new(raw::segment(2), Purpose::Roll, Op::Write(0, header)),
        Call::new(raw::segment(2), Purpose::Roll, Op::SyncData),
        Call::new("/log", Purpose::Roll, Op::SyncDir),
    ]
}

/// The write and the flush of the group `[c, d]`, the first one in segment 2.
fn first_group_steps() -> [Call; 2] {
    let frame = raw::group(1, 2, &[b"c", b"d"]);
    [
        Call::new(raw::segment(2), Purpose::Group, Op::Append(frame)),
        Call::new(raw::segment(2), Purpose::Group, Op::SyncData),
    ]
}

fn payloads(log: &SegmentLog) -> Vec<Vec<u8>> {
    log.read(0, 100)
        .unwrap()
        .into_iter()
        .map(|record| {
            assert_eq!(record.offset, record.revision);
            record.payload
        })
        .collect()
}

/// Every segment file is a header and whole frames after a recovery.
fn assert_whole_segments(sim: &Sim) {
    for (base, bytes) in raw::segments(sim) {
        assert_eq!(raw::header_fields(&bytes).1, base);
        raw::frames(&bytes);
    }
}

/// Given two acknowledged groups, with the active segment at the roll size
/// When a two-record submit starts a roll, and a crash overtakes the roll's create, header write,
/// Data flush or directory flush, or the write or the flush of the first group in the new segment
/// Then the submit gets `Poisoned` for a roll step and `Indeterminate` for a group step
/// And the submit queued behind it gets `Poisoned`
/// When the log reopens
/// Then both acknowledged records are present, and the submit is there in full or not at all
/// And the sealed segment keeps its bytes, and every file is a header and whole frames
/// And a power loss after that open gives the same log, and the next append takes the next offset
///
/// Cases: OPEN-LAYOUT, RECOVERY-FIXTURES, ORACLE-1, ORACLE-2, ORACLE-3, ORACLE-5, ORACLE-6, ORACLE-10
#[test]
fn a_crash_at_each_step_of_a_roll_keeps_every_acknowledged_record() {
    bounded(|| {
        for step in 0..6 {
            for mode in MODES {
                for seed in 0..4 {
                    let sim = Sim::new(320);
                    let log = two_groups(&sim);
                    let sealed = raw::segments(&sim);
                    let call = roll_steps(&sim)
                        .into_iter()
                        .chain(first_group_steps())
                        .nth(step)
                        .unwrap();
                    let hold = sim.hold(call);
                    let rolling = send(&log, vec![on_s(b"c", Some(1)), on_s(b"d", Some(2))]);
                    hold.wait();
                    let queued = send(&log, vec![on_s(b"e", Some(3))]);
                    assert!(rolling.probe().is_none());
                    assert_eq!(log.watermark(), 2);
                    sim.crash(mode, seed);
                    hold.release(None);
                    if step < 4 {
                        assert!(matches!(rolling.wait(), Err(CommitError::Poisoned)));
                    } else {
                        assert!(matches!(rolling.wait(), Err(CommitError::Indeterminate)));
                    }
                    assert!(matches!(queued.wait(), Err(CommitError::Poisoned)));
                    drop(log);

                    let log = open_rolling(&sim, EVERY_GROUP);
                    let kept = payloads(&log);
                    let label = format!("step {step} {mode:?} seed {seed}");
                    if step < 5 {
                        assert_eq!(kept, [b"a", b"b"], "{label}");
                    } else if mode == Crash::Kill {
                        assert_eq!(kept, [b"a", b"b", b"c", b"d"], "{label}");
                    } else {
                        assert!(
                            kept == [b"a", b"b"] || kept == [b"a", b"b", b"c", b"d"],
                            "{label}: {kept:?}"
                        );
                    }
                    assert_eq!(raw::segments(&sim)[..2], sealed, "{label}");
                    assert_whole_segments(&sim);
                    drop(log);

                    sim.crash(Crash::PowerLoss, seed);
                    let log = open_rolling(&sim, EVERY_GROUP);
                    assert_eq!(payloads(&log), kept, "{label}");
                    let next = kept.len() as u64;
                    let answer = send(&log, vec![on_s(b"next", Some(next - 1))])
                        .wait()
                        .unwrap();
                    assert_eq!(positions(&answer), [(next, next)], "{label}");
                }
            }
        }
    });
}

/// Given the storage trace of four groups that roll three times
/// When a crash cuts the trace at each storage call, under each crash mode
/// Then the reopened log holds every group whose flush had returned
/// And it ends at a group boundary, with no group after one that is missing
/// And a kill keeps every group whose write had returned
/// And every file is a header and whole frames, and a power loss after the open gives the same log
///
/// Cases: OPEN-LAYOUT, RECOVERY-FIXTURES, ORACLE-1, ORACLE-2, ORACLE-3, ORACLE-5
#[test]
fn every_storage_boundary_of_a_roll_can_crash() {
    bounded(|| {
        let sim = Sim::new(321);
        let log = open_rolling(&sim, EVERY_GROUP);
        let start = sim.trace().1.len();
        send(&log, vec![on_s(b"a", None)]).wait().unwrap();
        send(&log, vec![on_s(b"b", Some(0))]).wait().unwrap();
        send(&log, vec![on_s(b"c", Some(1)), on_s(b"d", Some(2))])
            .wait()
            .unwrap();
        send(&log, vec![on_s(b"e", Some(3))]).wait().unwrap();
        drop(log);
        assert_eq!(bases(&sim), [0, 1, 2, 4]);
        let all = [b"a", b"b", b"c", b"d", b"e"];
        let ends = [0, 1, 2, 4, 5];
        let (seed, trace) = sim.trace();
        for cut in start..=trace.len() {
            let group_calls = |op: fn(&Op) -> bool| {
                trace[..cut]
                    .iter()
                    .filter(|event| {
                        matches!(event, Event::Call(call, _, Outcome::Ok(_))
                            if call.purpose == Purpose::Group && op(&call.op))
                    })
                    .count()
            };
            let flushed = ends[group_calls(|op| matches!(op, Op::SyncData))];
            let written = ends[group_calls(|op| matches!(op, Op::Append(_)))];
            for mode in MODES {
                let replay = Sim::replay(seed, &trace[..cut]);
                replay.crash(mode, cut as u64);
                let log = open_rolling(&replay, EVERY_GROUP);
                let kept = payloads(&log);
                let label = format!("cut {cut} {mode:?}");
                assert_eq!(kept, all[..kept.len()], "{label}");
                assert!(ends.contains(&kept.len()), "{label}: {kept:?}");
                assert!(flushed <= kept.len() && kept.len() <= written, "{label}");
                if mode == Crash::Kill {
                    assert_eq!(kept.len(), written, "{label}");
                }
                assert_whole_segments(&replay);
                drop(log);
                replay.crash(Crash::PowerLoss, cut as u64);
                assert_eq!(
                    payloads(&open_rolling(&replay, EVERY_GROUP)),
                    kept,
                    "{label}"
                );
            }
        }
    });
}

/// Given two acknowledged groups, with the active segment at the roll size
/// When the roll's create, header write, data flush or directory flush fails
/// Then the submit that started the roll gets `Poisoned`, and so does the submit queued behind it
/// And later appends are refused, the acknowledged records stay readable
/// And the writer makes no further storage call that writes or flushes
/// When the log reopens, loses power and reopens again
/// Then both acknowledged records are present, and the next append takes the next offset
///
/// Cases: ORACLE-7, ORACLE-1, ORACLE-5, RECOVERY-FIXTURES
#[test]
fn a_failed_roll_step_poisons_the_writer_and_a_reopen_recovers() {
    bounded(|| {
        let other = || Fault::error(std::io::ErrorKind::Other).unwrap();
        for step in 0..4 {
            let faults = if step == 1 {
                vec![
                    Fault::Partial(1),
                    Fault::Partial(64),
                    Fault::NoSpace,
                    other(),
                ]
            } else {
                vec![Fault::NoSpace, other()]
            };
            for fault in faults {
                let label = format!("step {step} {fault:?}");
                let sim = Sim::new(322);
                let log = two_groups(&sim);
                let sealed = raw::segments(&sim);
                let call = roll_steps(&sim).into_iter().nth(step).unwrap();
                let hold = sim.hold(call.clone());
                let rolling = send(&log, vec![on_s(b"c", Some(1))]);
                hold.wait();
                let queued = send(&log, vec![on_s(b"d", Some(2))]);
                hold.release(Some(fault));
                assert!(
                    matches!(rolling.wait(), Err(CommitError::Poisoned)),
                    "{label}"
                );
                assert!(
                    matches!(queued.wait(), Err(CommitError::Poisoned)),
                    "{label}"
                );
                assert!(matches!(
                    log.append(Submit {
                        records: vec![on_s(b"later", Some(1))]
                    }),
                    Err(AppendError::Poisoned)
                ));
                assert_eq!(payloads(&log), [b"a", b"b"], "{label}");
                assert_eq!(log.watermark(), 2);
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

                if step == 3 {
                    assert!(
                        matches!(
                            SegmentLog::with_storage(
                                Path::new("/log"),
                                options(),
                                Arc::new(sim.clone())
                            ),
                            Err(OpenError::Io(_))
                        ),
                        "a failed directory flush stays fatal until a reboot"
                    );
                    sim.crash(Crash::PowerLoss, 1);
                }
                let log = open_rolling(&sim, EVERY_GROUP);
                assert_eq!(payloads(&log), [b"a", b"b"], "{label}");
                assert_eq!(raw::segments(&sim)[..2], sealed, "{label}");
                assert_whole_segments(&sim);
                drop(log);
                sim.crash(Crash::PowerLoss, 2);
                let log = open_rolling(&sim, EVERY_GROUP);
                assert_eq!(payloads(&log), [b"a", b"b"], "{label}");
                let answer = send(&log, vec![on_s(b"c", Some(1))]).wait().unwrap();
                assert_eq!(positions(&answer), [(2, 2)], "{label}");
            }
        }
    });
}
