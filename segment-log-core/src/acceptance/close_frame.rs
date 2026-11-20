//! ADR 0071: a clean drop writes a close frame, which proves that every group before it was
//! synced, so open no longer truncates a damaged last group as a torn tail.
use super::support::{bounded, fixtures::*, raw};
use crate::engine::Hooks;
use crate::storage::{
    Call, Op, Purpose,
    sim::{Crash, Event, Fault},
};
use crate::*;
use std::{path::Path, sync::Arc};

/// All groups in one unsealed segment, whatever roll size the run sets.
fn unrolled() -> Options {
    Options {
        roll_bytes: 1 << 20,
        ..options()
    }
}

fn reopen(sim: &Sim) -> Result<SegmentLog, OpenError> {
    SegmentLog::with_storage(Path::new("/log"), unrolled(), Arc::new(sim.clone()))
}

fn keyed(i: u64) -> Submit {
    let expected = match i {
        0 => ExpectedRevision::Empty,
        _ => ExpectedRevision::At(i - 1),
    };
    Submit {
        records: vec![NewRecord {
            stream: b"s".to_vec(),
            expected,
            key: Some(format!("k{i}").into_bytes()),
            payload: format!("record {i}").into_bytes(),
        }],
    }
}

/// Appends `n` acknowledged records, one group each, and drops the log cleanly.
fn written(sim: &Sim, n: u64) {
    let log = open_with(sim, unrolled());
    for i in 0..n {
        let committed = log.append(keyed(i)).unwrap().wait().unwrap();
        assert_eq!(committed.records[0].offset, i);
    }
    drop(log);
}

/// The position of the last group frame of the only segment.
fn last_group(segment: &[u8]) -> usize {
    let mut at = codec::HEADER_LEN;
    let mut last = None;
    for frame in raw::frames(segment) {
        if frame[8] == 1 {
            last = Some(at);
        }
        at += frame.len();
    }
    last.expect("a group")
}

/// Given three acknowledged groups in an unsealed segment and a clean drop
/// When one byte inside the last group frame flips
/// Then open fails with `Corrupt` at that frame and changes no file
/// And `LogReader` fails with the same `Corrupt`
///
/// Cases: OPEN-DAMAGE-LAST-GROUP-AFTER-CLOSE
#[test]
fn damage_to_the_last_group_after_a_clean_drop_is_refused() {
    bounded(|| {
        let sim = Sim::new(183);
        written(&sim, 3);
        let mut segments = raw::segments(&sim);
        let at = last_group(&segments[0].1);
        segments[0].1[at + 40] ^= 0xff;
        raw::install_segments(&sim, &segments);
        let before = sim.image();

        let result = reopen(&sim);
        assert!(
            matches!(
                result,
                Err(OpenError::Corrupt { segment: 0, position }) if position == at as u64
            ),
            "expected Corrupt at 0:{at}, got {:?}",
            result.map(|log| log.watermark())
        );
        assert_eq!(sim.image(), before, "a refused open changes no file");
        let reader = LogReader::with_storage(Path::new("/log"), Arc::new(sim.clone()));
        assert!(
            matches!(
                reader,
                Err(OpenError::Corrupt { segment: 0, position }) if position == at as u64
            ),
            "expected Corrupt from LogReader, got {:?}",
            reader.err()
        );
        assert_eq!(sim.image(), before);
    });
}

/// A close frame of the log in `sim`.
fn close(sim: &Sim, epoch: u64, base: u64) -> Vec<u8> {
    let log_id = codec::LogId::new(raw::log_id(sim));
    codec::encode_close(&log_id, epoch, base)
}

fn close_calls(sim: &Sim, op: Op) -> usize {
    sim.count(Path::new(ANY_SEG), Purpose::Close, &op)
}

/// Given a log that appended two records
/// When it is dropped cleanly
/// Then the segment ends with one close frame at the next offset, under the writer's epoch
/// When the log reopens and is dropped again without an append
/// Then it writes no second close frame
/// When the log reopens, appends and is dropped
/// Then the segment ends with a new close frame after the new group
///
/// Cases: DROP-CLOSE-FRAME
#[test]
fn a_clean_drop_writes_one_close_frame_and_an_idle_one_writes_none() {
    bounded(|| {
        let sim = Sim::new(1831);
        written(&sim, 2);
        let after_first = raw::segments(&sim)[0].1.clone();
        assert!(after_first.ends_with(&close(&sim, 1, 2)));
        assert_eq!(close_calls(&sim, Op::SyncData), 1);

        drop(reopen(&sim).unwrap());
        assert_eq!(
            raw::segments(&sim)[0].1,
            after_first,
            "an idle drop writes nothing"
        );
        assert_eq!(close_calls(&sim, Op::Append(vec![])), 1);

        let log = reopen(&sim).unwrap();
        log.append(keyed(2)).unwrap().wait().unwrap();
        drop(log);
        let segment = raw::segments(&sim)[0].1.clone();
        assert!(segment.starts_with(&after_first));
        // The idle session wrote no frame, so this one reuses its epoch.
        assert!(segment.ends_with(&close(&sim, 2, 3)));
        assert_eq!(reopen(&sim).unwrap().watermark(), 3);
    });
}

/// Given a writer that a failed group flush poisoned
/// When the log is dropped
/// Then it writes no close frame, because the failed group may not be durable
///
/// Given a rolled segment whose closing run the sealer fails to flush
/// When the log is dropped
/// Then it writes no close frame, because its last group is not in a sealed segment
///
/// Given a clean drop whose close frame's flush fails
/// When the log is dropped
/// Then it flushes once and never retries (ADR 0004), and the log reopens with every record
/// And that open rewrites and syncs the trailing close frame, not the group before it, because
/// only the close frame's flush is in doubt
///
/// Cases: DROP-CLOSE-FRAME
#[test]
fn a_poisoned_writer_or_a_failed_flush_writes_no_close_frame() {
    bounded(|| {
        let sim = Sim::new(1832);
        let log = open_with(&sim, unrolled());
        log.append(keyed(0)).unwrap().wait().unwrap();
        sim.inject(
            Call::new(ANY_SEG, Purpose::Group, Op::SyncData),
            Fault::NoSpace,
        );
        assert!(log.append(keyed(1)).unwrap().wait().is_err());
        drop(log);
        assert_eq!(close_calls(&sim, Op::Append(vec![])), 0, "poisoned");

        let sim = Sim::new(1833);
        let hooks = Hooks {
            background_seal: true,
            limits: crate::engine::Limits {
                keyed_records: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let log =
            SegmentLog::with_hooks(Path::new("/log"), unrolled(), Arc::new(sim.clone()), hooks)
                .unwrap();
        log.append(keyed(0)).unwrap().wait().unwrap();
        sim.inject(
            Call::new(raw::segment(0), Purpose::Roll, Op::SyncData),
            Fault::NoSpace,
        );
        log.append(keyed(1)).unwrap().wait().unwrap();
        drop(log);
        assert_eq!(
            close_calls(&sim, Op::Append(vec![])),
            0,
            "failed closing run"
        );

        let sim = Sim::new(1834);
        sim.inject(
            Call::new(ANY_SEG, Purpose::Close, Op::SyncData),
            Fault::NoSpace,
        );
        written(&sim, 2);
        assert_eq!(close_calls(&sim, Op::SyncData), 1, "no retry");
        let segment = raw::segments(&sim)[0].1.clone();
        let at = (segment.len() - 32) as u64;
        let log = reopen(&sim).unwrap();
        assert_eq!(log.read(0, 10).unwrap().len(), 2);
        // The frame whose flush failed is the one open rewrites and syncs (ADR 0015).
        let rewrite = Op::Write(at, close(&sim, 1, 2));
        let rewritten = sim.trace().1.iter().any(|event| {
            matches!(event, Event::Call(call, _, _)
                if call.purpose == Purpose::Recovery && call.op == rewrite)
        });
        assert!(rewritten, "open rewrites the trailing close frame");
    });
}

/// Given two acknowledged groups after a crash, so with no close frame
/// When the last group is torn
/// Then open truncates it as a torn tail and keeps the first, as before ADR 0071
///
/// Cases: OPEN-TORN-LAST-GROUP-NO-CLOSE
#[test]
fn a_torn_last_group_without_a_close_frame_is_truncated() {
    bounded(|| {
        let prefix = [raw::header(raw::VERSION, 0, 1), raw::group(1, 0, &[b"a"])].concat();
        let mut torn = raw::group(1, 1, &[b"b"]);
        torn[40] ^= 0xff;
        let sim = Sim::new(1835);
        raw::install(&sim, [prefix.clone(), torn].concat());
        let log = reopen(&sim).unwrap();
        assert_eq!(log.watermark(), 1);
        assert_eq!(raw::bytes(&sim), prefix);
    });
}

/// Given two acknowledged groups and a close frame
/// When the close frame is torn
/// Then open truncates only the close frame and keeps both groups
///
/// Cases: OPEN-TORN-CLOSE-FRAME
#[test]
fn a_torn_close_frame_is_truncated_and_every_group_kept() {
    bounded(|| {
        let groups = [
            raw::header(raw::VERSION, 0, 1),
            raw::group(1, 0, &[b"a"]),
            raw::group(1, 1, &[b"b"]),
        ]
        .concat();
        let close = raw::close(1, 2);
        for torn in [close[..20].to_vec(), {
            let mut bad = close.clone();
            bad[20] ^= 1;
            bad
        }] {
            let sim = Sim::new(1836);
            raw::install(&sim, [groups.clone(), torn].concat());
            let log = reopen(&sim).unwrap();
            assert_eq!(log.watermark(), 2);
            assert_eq!(raw::bytes(&sim), groups);
        }
    });
}

/// Given an earlier session's close frame, then a later session's group
/// When that group is damaged and nothing follows it
/// Then the old close frame proves nothing, because its offset is not past the damaged group's,
/// and open truncates the group
///
/// Cases: OPEN-OLD-CLOSE-FRAME
#[test]
fn an_earlier_sessions_close_frame_proves_nothing() {
    bounded(|| {
        let kept = [
            raw::header(raw::VERSION, 0, 1),
            raw::group(1, 0, &[b"a"]),
            raw::close(1, 1),
        ]
        .concat();
        let mut damaged = raw::group(2, 1, &[b"b"]);
        damaged[40] ^= 0xff;
        let sim = Sim::new(1837);
        raw::install(&sim, [kept.clone(), damaged].concat());
        let log = reopen(&sim).unwrap();
        assert_eq!(log.watermark(), 1);
        assert_eq!(raw::bytes(&sim), kept);
    });
}

/// Given a version 3 segment that holds a frame of kind 7 after its groups, which no writer makes
/// When a build at version 4 opens it
/// Then the frame is not valid in that segment's version: open truncates it as a torn tail, and
/// closes the segment in version 3 before it rolls (ADR 0064, rule 12)
/// And with a valid group after it, open fails with `Corrupt` at the frame
///
/// Cases: FORMAT-MIXED
#[test]
fn a_kind_7_frame_in_a_version_3_segment_is_not_valid() {
    bounded(|| {
        let groups = [raw::header(3, 0, 1), raw::group(1, 0, &[b"a"])].concat();
        let sim = Sim::new(1838);
        raw::install(&sim, [groups.clone(), raw::close(1, 1)].concat());
        let log = reopen(&sim).unwrap();
        assert_eq!(log.watermark(), 1);
        assert_eq!(raw::segments(&sim)[0].1, raw::rolled(groups.clone(), 2));

        let sim = Sim::new(1839);
        let later = raw::group(1, 1, &[b"b"]);
        raw::install(&sim, [groups.clone(), raw::close(1, 1), later].concat());
        let before = sim.image();
        let cut = groups.len() as u64;
        assert!(matches!(
            reopen(&sim),
            Err(OpenError::Corrupt { segment: 0, position }) if position == cut
        ));
        assert_eq!(sim.image(), before);
    });
}

/// Given two acknowledged groups
/// When a kill or a power loss overtakes the drop's close frame write or its flush
/// Then the log reopens with both records, a torn close frame being a torn tail
///
/// Cases: DROP-CLOSE-FRAME
#[test]
fn a_crash_at_each_step_of_a_drop_keeps_every_record() {
    bounded(|| {
        for step in [Op::Append(vec![]), Op::SyncData] {
            for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
                for seed in 0..4 {
                    let label = format!("{step:?} {mode:?} seed {seed}");
                    let sim = Sim::new(1840 + seed);
                    let log = open_with(&sim, unrolled());
                    for i in 0..2 {
                        log.append(keyed(i)).unwrap().wait().unwrap();
                    }
                    let step = match &step {
                        Op::Append(_) => Op::Append(close(&sim, 1, 2)),
                        other => other.clone(),
                    };
                    let hold = sim.hold(Call::new(ANY_SEG, Purpose::Close, step));
                    let dropping = std::thread::spawn(move || drop(log));
                    hold.wait();
                    sim.crash(mode, seed);
                    hold.release(None);
                    dropping.join().unwrap();

                    let log = reopen(&sim).unwrap();
                    let read = log.read(0, 10).unwrap();
                    assert_eq!(read.len(), 2, "{label}");
                }
            }
        }
    });
}

fn apply(sim: &Sim, at: (u64, u64), backup: &str) -> repair::Outcome {
    let apply = repair::Apply {
        at,
        drop_acknowledged: true,
        backup,
    };
    repair::apply_with(sim, Path::new("/log"), raw::VERSION, apply).unwrap()
}

/// An unsealed segment of two groups and the close frame of a clean drop, whose second group,
/// with a payload of `payload` bytes, has a flipped byte. Returns the log, the bad group's
/// position and the segment's bytes.
fn damage_before_a_close_frame(payload: usize) -> (Sim, u64, Vec<u8>) {
    let first = raw::group(1, 0, &[b"a"]);
    let mut second = raw::group(1, 1, &[&vec![b'b'; payload]]);
    second[40] ^= 0xff;
    let bad = (codec::HEADER_LEN + first.len()) as u64;
    let bytes = [
        raw::header(raw::VERSION, 0, 1),
        first,
        second,
        raw::close(1, 2),
    ]
    .concat();
    let sim = Sim::new(1850 + payload as u64);
    raw::install_segments(&sim, &[(0, bytes.clone())]);
    (sim, bad, bytes)
}

fn assert_sealed_with_one_record(sim: &Sim, label: &str) {
    let reader = LogReader::with_storage(Path::new("/log"), Arc::new(sim.clone())).unwrap();
    let payloads: Vec<_> = reader
        .read(0, 10)
        .unwrap()
        .into_iter()
        .map(|r| r.payload)
        .collect();
    assert_eq!(payloads, [b"a"], "{label}");
    let log = reopen(sim).unwrap();
    assert!(
        matches!(log.append(keyed(9)), Err(AppendError::Sealed)),
        "{label}"
    );
}

/// Given an unsealed segment whose last group is damaged and whose only proof is the clean drop's
/// close frame after it, with room for a seal run before the close frame
/// When `verify` and `truncate --apply --drop-acknowledged` run
/// Then `verify` reports class F, the plan drops offset 1 and moves no proof, and the log ends
/// sealed after the record before the cut
///
/// Cases: REPAIR-CLASS-F-CLOSE-FITS
#[test]
fn a_class_f_repair_with_a_close_frame_proof_that_has_room_seals_the_log() {
    bounded(|| {
        let (sim, bad, bytes) = damage_before_a_close_frame(300);
        let report = repair::verify_with(&sim, Path::new("/log"), raw::VERSION).unwrap();
        let found: Vec<_> = report
            .findings
            .iter()
            .map(|f| (f.class, f.segment, f.position))
            .collect();
        assert_eq!(found, [(repair::Class::F, 0, bad)]);
        let outcome = apply(&sim, (0, bad), "repair-1");
        assert_eq!(outcome.refusal, None);
        let plan = outcome.plan.unwrap();
        assert_eq!(plan.dropped_offsets, Some((1, 2)));
        assert_eq!(plan.moved_proof, None);
        assert!(plan.applied);
        let kept = bytes[..bad as usize].to_vec();
        let run = raw::seal_run(&kept, 2);
        assert_eq!(raw::segments(&sim), [(0, [kept, run].concat())]);
        assert_sealed_with_one_record(&sim, "fits");
    });
}

/// Given the same log, but a damaged group too small to hold a seal run before the close frame
/// When `truncate --apply --drop-acknowledged` runs
/// Then the plan first copies the close frame past where the seal run will end, and the log ends
/// sealed after the record before the cut, instead of refusing as with a group-frame proof
///
/// Cases: REPAIR-CLASS-F-CLOSE-MOVED
#[test]
fn a_class_f_repair_moves_a_close_frame_proof_past_its_seal_run() {
    bounded(|| {
        let (sim, bad, bytes) = damage_before_a_close_frame(1);
        let close_at = (bytes.len() - 32) as u64;
        let plan = repair::plan_with(&sim, Path::new("/log"), raw::VERSION).unwrap();
        assert_eq!(plan.refusal, None);
        let kept = bytes[..bad as usize].to_vec();
        let run = raw::seal_run(&kept, 2);
        let run_end = bad + run.len() as u64;
        assert!(
            run_end > close_at,
            "the run must not fit before the close frame"
        );
        assert_eq!(plan.plan.unwrap().moved_proof, Some((close_at, run_end)));
        let outcome = apply(&sim, (0, bad), "repair-1");
        assert_eq!(outcome.refusal, None);
        assert!(outcome.plan.unwrap().applied);
        assert_eq!(raw::segments(&sim), [(0, [kept, run].concat())]);
        assert_sealed_with_one_record(&sim, "moved");
    });
}

/// Given the logs of the two scenarios above, and the trace of `truncate --apply
/// --drop-acknowledged` on each
/// When Kill, PowerLoss or PowerLossTorn cuts that trace at each storage call
/// Then open fails with `Corrupt` at the cut, or finds the sealed repaired log; it never finds a
/// writable log, which would assign a dropped offset again
/// And after `Corrupt`, a second `--apply` converges to the same repaired log
///
/// Cases: REPAIR-CLASS-F-CLOSE-CRASH
#[test]
fn a_crash_at_each_step_of_a_close_frame_repair_fails_closed_or_finds_it_sealed() {
    bounded(|| {
        for payload in [1, 300] {
            let (sim, bad, original) = damage_before_a_close_frame(payload);
            let start = sim.trace().1.len();
            apply(&sim, (0, bad), "repair-1");
            let repaired = raw::segments(&sim)[0].1.clone();
            let (seed, trace) = sim.trace();
            let (mut closed, mut opened) = (0, 0);
            for step in start..=trace.len() {
                for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
                    for crash_seed in 0..3 {
                        let crashed = Sim::replay(seed, &trace[..step]);
                        crashed.crash(mode, crash_seed);
                        let label = format!("payload {payload} step {step} {mode:?} {crash_seed}");
                        match reopen(&crashed) {
                            Err(OpenError::Corrupt {
                                segment: 0,
                                position,
                            }) => {
                                assert_eq!(position, bad, "{label}");
                                let now = &raw::segments(&crashed)[0].1;
                                assert_eq!(
                                    now[..bad as usize],
                                    original[..bad as usize],
                                    "{label}"
                                );
                                closed += 1;
                                let again = apply(&crashed, (0, bad), "repair-2");
                                assert!(again.plan.unwrap().applied, "{label}");
                                assert_eq!(raw::segments(&crashed)[0].1, repaired, "{label}");
                            }
                            Ok(log) => {
                                opened += 1;
                                drop(log);
                                assert_eq!(raw::segments(&crashed)[0].1, repaired, "{label}");
                                assert_sealed_with_one_record(&crashed, &label);
                            }
                            Err(other) => panic!("{label}: {other:?}"),
                        }
                    }
                }
            }
            assert!(closed > 0 && opened > 0, "{payload}: {closed} {opened}");
        }
    });
}

/// Given a segment that holds two sessions' groups, each followed by its close frame, and that a
/// later session rolled
/// When a reopened log reads the sealed segment, which it walks frame by frame
/// Then it steps over both close frames and returns every record
/// When a byte of the close frame between the groups flips
/// Then that read fails with `Corrupt` at the close frame, and reads of the next segment succeed
///
/// Cases: DROP-CLOSE-FRAME, READ-BOUNDED
#[test]
fn reads_step_over_close_frames_in_a_sealed_segment() {
    bounded(|| {
        let sim = Sim::new(1860);
        written(&sim, 2);
        let between = raw::segments(&sim)[0].1.len() - 32;
        let log = reopen(&sim).unwrap();
        log.append(keyed(2)).unwrap().wait().unwrap();
        drop(log);
        let rolling = Options {
            roll_bytes: EVERY_GROUP,
            ..options()
        };
        let log = open_with(&sim, rolling);
        log.append(keyed(3)).unwrap().wait().unwrap();
        drop(log);
        assert_eq!(raw::segments(&sim).len(), 2, "segment 0 rolled");

        let payloads = |log: &SegmentLog, from| -> Result<Vec<Vec<u8>>, ReadError> {
            Ok(log.read(from, 10)?.into_iter().map(|r| r.payload).collect())
        };
        let log = reopen(&sim).unwrap();
        let expected: Vec<Vec<u8>> = (0..4).map(|i| format!("record {i}").into_bytes()).collect();
        assert_eq!(payloads(&log, 0).unwrap(), expected);
        drop(log);

        let mut segments = raw::segments(&sim);
        // The CRC byte: only the checksum check catches it.
        segments[0].1[between + 4] ^= 1;
        raw::install_segments(&sim, &segments);
        let log = reopen(&sim).unwrap();
        assert!(
            matches!(
                payloads(&log, 0),
                Err(ReadError::Corrupt { segment: 0, position }) if position == between as u64
            ),
            "{:?}",
            payloads(&log, 0)
        );
        assert_eq!(payloads(&log, 3).unwrap(), expected[3..]);
    });
}

/// Given a new log that never appended, so its only segment holds only its header
/// When it is dropped cleanly
/// Then the segment ends with a close frame at offset 0, so drop has one path
/// And the log reopens empty and appends at offset 0
///
/// Cases: DROP-CLOSE-FRAME
#[test]
fn a_drop_with_no_group_still_writes_a_close_frame() {
    bounded(|| {
        let sim = Sim::new(1870);
        drop(open_with(&sim, unrolled()));
        let segment = raw::segments(&sim)[0].1.clone();
        assert_eq!(segment.len(), codec::HEADER_LEN + 32);
        assert_eq!(segment[codec::HEADER_LEN..], close(&sim, 1, 0));
        let log = reopen(&sim).unwrap();
        assert_eq!(log.watermark(), 0);
        let committed = log.append(keyed(0)).unwrap().wait().unwrap();
        assert_eq!(committed.records[0].offset, 0);
    });
}
