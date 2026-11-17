//! The operator repair tool's `truncate` (ADR 0065): the plan, the backup, the cut, and a crash at
//! every storage call of `--apply`.
use super::repair::{VERSION, lookalike};
use super::support::{bounded, fixtures::*, raw};
use crate::repair::{Apply, Error, Outcome, PlanKind, apply_with, plan_with};
use crate::storage::{
    Call, Op, Purpose, Storage, Value,
    sim::{Crash, Event, Fault},
};
use crate::{codec, *};
use std::{path::Path, sync::Arc};

const MODES: [Crash; 3] = [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn];

fn plan(sim: &Sim) -> Outcome {
    plan_with(sim, Path::new("/log"), VERSION).unwrap()
}

fn apply(sim: &Sim, at: (u64, u64), drop_acknowledged: bool, backup: &str) -> Outcome {
    try_apply(sim, at, drop_acknowledged, backup).unwrap()
}

fn try_apply(
    sim: &Sim,
    at: (u64, u64),
    drop_acknowledged: bool,
    backup: &str,
) -> Result<Outcome, Error> {
    let apply = Apply {
        at,
        drop_acknowledged,
        backup,
    };
    apply_with(sim, Path::new("/log"), VERSION, apply)
}

fn try_open(sim: &Sim) -> Result<SegmentLog, OpenError> {
    SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone()))
}

fn payloads(records: Vec<Record>) -> Vec<Vec<u8>> {
    records.into_iter().map(|r| r.payload).collect()
}

fn file(sim: &Sim, path: &str) -> Vec<u8> {
    let read = Call::new(path, Purpose::Open, Op::Read(0, usize::MAX));
    let Ok(Value::Bytes(bytes)) = sim.call(read) else {
        panic!("read {path}")
    };
    bytes
}

/// A newest segment that ends in a complete closing run, a roll's or a seal's, after three groups
/// of one record each, the second with a bad checksum. Returns the log, the bad group's position
/// and the segment's bytes.
fn bad_group_before_run(roll: bool) -> (Sim, u64, Vec<u8>) {
    let first = raw::group(1, 0, &[b"a"]);
    // Large enough that the seal run fits before the old footer.
    let second = raw::group(1, 1, &[&[b'b'; 200]]);
    let third = raw::group(1, 2, &[&[b'c'; 200]]);
    let groups = [
        raw::header(raw::VERSION, 0, 1),
        first.clone(),
        second,
        third,
    ]
    .concat();
    let mut bytes = if roll {
        raw::rolled(groups, 1)
    } else {
        let run = raw::seal_run(&groups, 1);
        [groups, run].concat()
    };
    let bad = 64 + first.len();
    bytes[bad + 40] ^= 1;
    let sim = Sim::new(660);
    raw::install_segments(&sim, &[(0, bytes.clone())]);
    assert!(matches!(
        try_open(&sim),
        Err(OpenError::Corrupt { segment: 0, position }) if position == bad as u64
    ));
    (sim, bad as u64, bytes)
}

/// Given the newest segment ends in a torn group whose payload holds the log's own closing run, so
/// open fails with `Corrupt`, and the footer in that payload names no position at or after the
/// torn group
/// When `truncate` runs without `--apply`
/// Then the plan is a torn tail cut where open stopped, dropping no acknowledged offset, and
/// nothing changes
/// When `truncate --apply` runs with that cut
/// Then open succeeds, keeps the acknowledged record, and the next append takes the next offset
///
/// Cases: REPAIR-TORN-LOOKALIKE
#[test]
fn a_torn_tail_behind_a_lookalike_run_is_truncated() {
    bounded(|| {
        let (sim, torn_at) = lookalike();
        let before = sim.image();
        let outcome = plan(&sim);
        assert_eq!(outcome.refusal, None);
        let p = outcome.plan.unwrap();
        assert_eq!((p.segment, p.cut, p.kind), (0, torn_at, PlanKind::TornTail));
        assert_eq!(p.dropped_offsets, None);
        assert_eq!(
            p.closing_frames_removed,
            ["key-filter", "key-index", "footer", "locator"]
        );
        assert!(!p.applied);
        assert_eq!(sim.image(), before);

        let outcome = apply(&sim, (0, torn_at), false, "repair-1");
        assert_eq!(outcome.refusal, None);
        assert!(outcome.plan.unwrap().applied);
        let log = try_open(&sim).unwrap();
        assert_eq!(payloads(log.read(0, 10).unwrap()), [b"a"]);
        let committed = append(&log, b"t").wait().unwrap();
        assert_eq!(committed.records[0].offset, 1);
    });
}

/// Given a newest segment that ends in a complete closing run, with a bad group frame before two
/// valid ones
/// When `truncate` runs
/// Then the plan is lossy and names the dropped offsets 1 to 3
/// When `truncate --apply` runs without `--drop-acknowledged`
/// Then it refuses and changes nothing
///
/// Cases: REPAIR-LOSSY-REFUSED
#[test]
fn a_lossy_plan_needs_drop_acknowledged() {
    bounded(|| {
        for roll in [false, true] {
            let (sim, bad, _) = bad_group_before_run(roll);
            let before = sim.image();
            let p = plan(&sim).plan.unwrap();
            assert_eq!((p.cut, p.kind), (bad, PlanKind::Lossy));
            assert_eq!(p.dropped_offsets, Some((1, 3)));
            let outcome = apply(&sim, (0, bad), false, "repair-1");
            assert!(outcome.refusal.unwrap().contains("--drop-acknowledged"));
            assert!(!outcome.plan.unwrap().applied);
            assert_eq!(sim.image(), before);
        }
    });
}

/// Given the same log, ending in a seal's run or a roll's run
/// When `truncate --apply --drop-acknowledged` runs
/// Then the segment ends in a seal's run after the record before the cut
/// And `LogReader` reads that record and none after it, and a writable open finds the log sealed,
/// so no dropped offset is assigned again
///
/// Cases: REPAIR-LOSSY-SEALS
#[test]
fn a_lossy_repair_seals_the_log() {
    bounded(|| {
        for roll in [false, true] {
            let (sim, bad, bytes) = bad_group_before_run(roll);
            let outcome = apply(&sim, (0, bad), true, "repair-1");
            assert_eq!(outcome.refusal, None);
            assert!(outcome.plan.unwrap().applied);
            let kept = bytes[..bad as usize].to_vec();
            let run = raw::seal_run(&kept, 2);
            assert_eq!(raw::segments(&sim), [(0, [kept, run].concat())]);
            let reader = LogReader::with_storage(Path::new("/log"), Arc::new(sim.clone())).unwrap();
            assert_eq!(payloads(reader.read(0, 10).unwrap()), [b"a"]);
            assert_eq!(reader.watermark(), 1);
            let log = try_open(&sim).unwrap();
            assert!(matches!(log.append(submit(b"t")), Err(AppendError::Sealed)));
        }
    });
}

/// Given a newest segment whose scan stops at 20 bytes of damage, before a seal's run whose footer
/// names that position, so the plan is lossy and its seal run would reach the old footer
/// When `truncate` runs, with or without `--apply`
/// Then it refuses, and changes nothing
///
/// Cases: REPAIR-LOSSY-NO-ROOM
#[test]
fn a_seal_run_that_would_reach_the_old_locator_is_refused() {
    bounded(|| {
        let log_id = codec::LogId::new(raw::LOG_ID);
        let kept = [raw::header(raw::VERSION, 0, 1), raw::group(1, 0, &[b"a"])].concat();
        let cut = kept.len() as u64;
        let footer = codec::encode_footer(
            &log_id,
            1,
            2,
            &codec::Footer {
                key_filter_position: None,
                streams: vec![codec::StreamTail {
                    stream: b"s",
                    last_revision: 1,
                    last_offset: 1,
                    last_position: cut,
                }],
            },
        )
        .unwrap();
        let footer_at = cut + 20;
        let seal = codec::encode_seal(&log_id, 1, 2);
        let locator = codec::encode_locator(&log_id, 1, 2, footer_at);
        let bytes = [kept, vec![3; 20], footer, seal, locator].concat();
        let sim = Sim::new(661);
        raw::install_segments(&sim, &[(0, bytes)]);
        let before = sim.image();
        let outcome = plan(&sim);
        assert_eq!(outcome.plan.unwrap().kind, PlanKind::Lossy);
        assert!(outcome.refusal.unwrap().contains("old footer"));
        let outcome = apply(&sim, (0, cut), true, "repair-1");
        assert!(outcome.refusal.is_some());
        assert_eq!(sim.image(), before);
    });
}

/// Given a class A log
/// When `truncate --apply` runs with an `--at` that differs from the plan's cut
/// Then it refuses and changes nothing
/// When the damage moves after the dry run, and `--apply` repeats the dry run's cut
/// Then it refuses and changes nothing
///
/// Cases: REPAIR-PLAN-MISMATCH
#[test]
fn apply_refuses_a_cut_that_differs_from_the_plan() {
    bounded(|| {
        let (sim, torn_at) = lookalike();
        let before = sim.image();
        let outcome = apply(&sim, (0, torn_at + 1), false, "repair-1");
        assert!(outcome.refusal.unwrap().contains("differs from the plan"));
        assert_eq!(sim.image(), before);

        let (sim, bad, mut bytes) = bad_group_before_run(false);
        assert_eq!(plan(&sim).plan.unwrap().cut, bad);
        bytes[64 + 40] ^= 1;
        let write = Op::Write(64 + 40, vec![bytes[64 + 40]]);
        sim.call(Call::new(raw::segment(0), Purpose::Open, write))
            .unwrap();
        let before = sim.image();
        let outcome = apply(&sim, (0, bad), true, "repair-1");
        assert!(outcome.refusal.unwrap().contains("differs from the plan"));
        assert_eq!(sim.image(), before);
    });
}

/// Given a class A log
/// When `truncate --apply` runs
/// Then it copies the newest segment byte for byte into the backup directory, and the copy, its
/// directory entry and the log directory are durable before the first change to the segment
/// And open ignores the backup directory
/// When an `--apply` fails at the cut, after its backup, and a second one runs
/// Then the second makes its own backup and keeps the first
///
/// Cases: REPAIR-BACKUP
#[test]
fn apply_backs_up_the_segment_before_changing_it() {
    bounded(|| {
        let (sim, torn_at) = lookalike();
        let original = raw::segments(&sim)[0].1.clone();
        let start = sim.trace().1.len();
        apply(&sim, (0, torn_at), false, "repair-1");
        assert_eq!(
            file(&sim, "/log/repair-1/00000000000000000000.seg"),
            original
        );
        let calls: Vec<Call> = sim.trace().1[start..]
            .iter()
            .filter_map(|e| match e {
                Event::Call(call, _, _) if call.purpose == Purpose::Repair => Some(call.clone()),
                _ => None,
            })
            .collect();
        let index = |path: &str, op: Op| {
            calls
                .iter()
                .position(|c| c.path == Path::new(path) && c.op == op)
                .unwrap()
        };
        let cut = index(raw::segment(0).as_str(), Op::Truncate(torn_at));
        assert!(index("/log/repair-1/00000000000000000000.seg", Op::SyncAll) < cut);
        assert!(index("/log/repair-1", Op::SyncDir) < cut);
        assert!(index("/log", Op::SyncDir) < cut);
        let log = try_open(&sim).unwrap();
        assert_eq!(log.watermark(), 1);
        drop(log);

        let (sim, torn_at) = lookalike();
        let original = raw::segments(&sim)[0].1.clone();
        let truncate = Call::new(raw::segment(0), Purpose::Repair, Op::Truncate(torn_at));
        sim.inject(truncate, Fault::error(std::io::ErrorKind::Other).unwrap());
        assert!(matches!(
            try_apply(&sim, (0, torn_at), false, "repair-1"),
            Err(Error::Io(_))
        ));
        apply(&sim, (0, torn_at), false, "repair-2");
        for backup in ["repair-1", "repair-2"] {
            let path = format!("/log/{backup}/00000000000000000000.seg");
            assert_eq!(file(&sim, &path), original, "{backup}");
        }
    });
}

/// Given a writer that holds `LOCK`
/// When `truncate` runs, with or without `--apply`
/// Then it fails with `Locked` and changes nothing
///
/// Cases: REPAIR-LOCKED
#[test]
fn truncate_fails_while_another_process_holds_the_lock() {
    bounded(|| {
        let sim = Sim::new(662);
        let log = open(&sim);
        let before = sim.image();
        let planned = plan_with(&sim, Path::new("/log"), VERSION);
        assert!(matches!(planned, Err(Error::Locked)), "{planned:?}");
        assert!(matches!(
            try_apply(&sim, (0, 64), true, "repair-1"),
            Err(Error::Locked)
        ));
        assert_eq!(sim.image(), before);
        drop(log);
    });
}

/// Given a class A log with a torn-tail plan, and with a lossy plan before a seal's run or a roll's
/// run
/// When a crash comes after each storage call of `truncate --apply`, as a process kill, a power
/// loss or a torn power loss
/// Then open fails with `Corrupt` at the cut, with every byte before the cut unchanged, or open
/// finds the repaired log: writable after a torn-tail cut, sealed after a lossy one
/// And open never finds a writable log that lacks a dropped offset
/// And after `Corrupt`, a second `--apply` repairs the log the same way
///
/// Cases: REPAIR-CRASH
#[test]
fn a_crash_at_each_step_of_apply_fails_closed_or_finds_the_repaired_log() {
    bounded(|| {
        for (lossy, roll) in [(false, false), (true, false), (true, true)] {
            let (sim, cut, original) = if lossy {
                bad_group_before_run(roll)
            } else {
                let (sim, cut) = lookalike();
                let bytes = raw::segments(&sim)[0].1.clone();
                (sim, cut, bytes)
            };
            let start = sim.trace().1.len();
            apply(&sim, (0, cut), lossy, "repair-1");
            let repaired = raw::segments(&sim)[0].1.clone();
            let (seed, trace) = sim.trace();
            let (mut closed, mut opened) = (0, 0);
            for step in start..=trace.len() {
                for mode in MODES {
                    for crash_seed in 0..3 {
                        let crashed = Sim::replay(seed, &trace[..step]);
                        crashed.crash(mode, crash_seed);
                        let label =
                            format!("lossy {lossy} roll {roll} step {step} {mode:?} {crash_seed}");
                        match try_open(&crashed) {
                            Err(OpenError::Corrupt {
                                segment: 0,
                                position,
                            }) => {
                                assert_eq!(position, cut, "{label}");
                                // Bytes before the cut never change.
                                let now = &raw::segments(&crashed)[0].1;
                                assert_eq!(
                                    now[..cut as usize],
                                    original[..cut as usize],
                                    "{label}"
                                );
                                closed += 1;
                                let again = apply(&crashed, (0, cut), lossy, "repair-2");
                                assert!(again.plan.unwrap().applied, "{label}");
                                assert_eq!(raw::segments(&crashed)[0].1, repaired, "{label}");
                            }
                            Ok(log) => {
                                opened += 1;
                                assert_eq!(raw::segments(&crashed)[0].1, repaired, "{label}");
                                assert_eq!(payloads(log.read(0, 10).unwrap()), [b"a"], "{label}");
                                let appended = log.append(submit(b"t"));
                                assert_eq!(appended.is_ok(), !lossy, "{label}");
                            }
                            Err(other) => panic!("{label}: {other:?}"),
                        }
                    }
                }
            }
            assert!(
                closed > 0 && opened > 0,
                "lossy {lossy} roll {roll}: {closed} {opened}"
            );
        }
    });
}
