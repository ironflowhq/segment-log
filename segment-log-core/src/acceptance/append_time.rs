//! ADR 0064, rule 2: each group frame carries a wall-clock append time, and the time never
//! decreases within a log, even when the clock steps back.
use super::support::{bounded, fixtures::*, raw};
use crate::storage::sim::{Crash, Event, Outcome};
use crate::storage::{Op, Purpose};
use crate::*;

const MODES: [Crash; 3] = [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn];

fn rolling(sim: &Sim) -> SegmentLog {
    open_with(
        sim,
        Options {
            roll_bytes: EVERY_GROUP,
            ..options()
        },
    )
}

fn append(log: &SegmentLog, payload: &[u8]) {
    log.append(Submit {
        records: vec![NewRecord {
            stream: payload.to_vec(),
            expected: ExpectedRevision::Empty,
            key: None,
            payload: payload.to_vec(),
        }],
    })
    .unwrap()
    .wait()
    .unwrap();
}

/// The append time of every group in the log, in log order.
fn times(sim: &Sim) -> Vec<u64> {
    let segments = raw::segments(sim);
    let log_id = crate::codec::LogId::new(raw::header_fields(&segments[0].1).0);
    segments
        .iter()
        .flat_map(|(_, bytes)| raw::frames(bytes))
        .filter_map(|frame| crate::codec::decode(&log_id, frame).ok()?.into_group())
        .map(|group| group.append_time)
        .collect()
}

/// Given a log whose groups roll after each group
/// When the clock reads 5,000, then steps back to 3,000, then jumps to 7,000, and the log appends
/// a group at each reading
/// Then the groups carry 5,000, 5,000 and 7,000
/// When the log reopens with the clock stepped back to 1,000 and appends again
/// Then that group carries 7,000, the newest time open found
/// When the log rolls into a segment with no group, reopens with the clock at 2,000, and appends
/// Then open takes the newest time from the segment before, and the group still carries 7,000
///
/// Cases: FORMAT-APPEND-TIME
#[test]
fn a_clock_that_steps_back_never_lowers_the_append_time() {
    bounded(|| {
        let sim = Sim::new(640);
        let log = rolling(&sim);
        for (clock, payload) in [(5_000, b"a"), (3_000, b"b"), (7_000, b"c")] {
            sim.set_clock(clock, 0);
            append(&log, payload);
        }
        assert_eq!(times(&sim), [5_000, 5_000, 7_000]);
        drop(log);

        sim.set_clock(1_000, 0);
        let log = rolling(&sim);
        append(&log, b"d");
        assert_eq!(times(&sim), [5_000, 5_000, 7_000, 7_000]);
        drop(log);

        // A reopen rolls the full active segment before the next group goes in, and writes the
        // rolled segment's closing run first. Holding the group's write and killing the process
        // leaves a newest segment with only its header, after a sealed one.
        let log_id = crate::codec::LogId::new(raw::log_id(&sim));
        let group = crate::codec::encode_group(
            &log_id,
            &crate::codec::Group {
                epoch: 3,
                base_offset: 4,
                append_time: 7_000,
                entries: vec![crate::codec::Entry {
                    stream: b"e",
                    key: None,
                    payload: b"e",
                    revision: 0,
                }],
            },
        )
        .unwrap();
        let hold = sim.hold(crate::storage::Call::new(
            ANY_SEG,
            Purpose::Group,
            Op::Append(group),
        ));
        let log = rolling(&sim);
        let pending = log
            .append(Submit {
                records: vec![NewRecord {
                    stream: b"e".to_vec(),
                    expected: ExpectedRevision::Empty,
                    key: None,
                    payload: b"e".to_vec(),
                }],
            })
            .unwrap();
        hold.wait();
        sim.crash(Crash::Kill, 640);
        hold.release(None);
        assert!(pending.wait().is_err());
        drop(log);
        let newest = raw::segments(&sim).pop().unwrap().1;
        assert_eq!(newest.len(), 64, "the newest segment holds only its header");

        sim.set_clock(2_000, 0);
        let log = rolling(&sim);
        append(&log, b"f");
        assert_eq!(times(&sim), [5_000, 5_000, 7_000, 7_000, 7_000]);
    });
}

/// Given the storage trace of groups appended while the clock steps back after each one, rolling
/// after each group
/// When a crash cuts the trace at each storage call, under each crash mode, and the log reopens
/// with the clock far behind every stored time and appends one group
/// Then the append times of the kept groups never decrease, and the new group carries the newest
/// kept time, or the low clock reading in a log with no kept group
///
/// Cases: FORMAT-APPEND-TIME, ORACLE-1
#[test]
fn a_crash_at_each_storage_boundary_keeps_append_times_from_decreasing() {
    bounded(|| {
        let sim = Sim::new(641);
        sim.set_clock(10_000, -1_000);
        let log = rolling(&sim);
        for payload in [b"a", b"b", b"c", b"d"] {
            append(&log, payload);
        }
        drop(log);
        assert_eq!(times(&sim), [10_000; 4]);
        let (seed, trace) = sim.trace();
        let groups = trace
            .iter()
            .filter(
                |e| matches!(e, Event::Call(c, _, Outcome::Ok(_)) if c.purpose == Purpose::Group),
            )
            .count();
        assert!(groups >= 8, "each group is a write and a flush");
        for cut in 0..=trace.len() {
            for mode in MODES {
                let replay = Sim::replay(seed, &trace[..cut]);
                replay.crash(mode, cut as u64);
                replay.set_clock(5, 0);
                let log = rolling(&replay);
                let kept = times(&replay);
                append(&log, b"next");
                let label = format!("cut {cut} {mode:?}");
                let after = times(&replay);
                assert_eq!(after[..kept.len()], kept[..], "{label}");
                assert!(after.windows(2).all(|w| w[0] <= w[1]), "{label}: {after:?}");
                let expected = kept.last().copied().unwrap_or(5);
                assert_eq!(after.last(), Some(&expected), "{label}: {after:?}");
            }
        }
    });
}
