use super::support::{bounded, fixtures::*, raw};
use crate::repair::{Apply, Class, Outcome, PlanKind, apply_with, plan_with, verify_with};
use crate::{storage, *};
use std::{path::Path, sync::Arc};

fn apply(sim: &Sim, at: (u64, u64), drop_acknowledged: bool, backup: &str) -> Outcome {
    let apply = Apply {
        at,
        drop_acknowledged,
        backup,
    };
    apply_with(sim, Path::new("/log"), raw::VERSION, apply).unwrap()
}

/// An unsealed newest segment of three groups of one record each, the second with a flipped byte.
/// Returns the log, the bad group's position, the third group's position and the segment's bytes.
fn damage_before_a_group() -> (Sim, u64, u64, Vec<u8>) {
    let first = raw::group(1, 0, &[b"a"]);
    // Large enough that a seal run fits before the third group.
    let mut second = raw::group(1, 1, &[&[b'b'; 200]]);
    second[40] ^= 0xff;
    let third = raw::group(1, 2, &[&[b'c'; 200]]);
    let bad = (codec::HEADER_LEN + first.len()) as u64;
    let proof = bad + second.len() as u64;
    let bytes = [raw::header(raw::VERSION, 0, 1), first, second, third].concat();
    let sim = Sim::new(1783);
    raw::install_segments(&sim, &[(0, bytes.clone())]);
    (sim, bad, proof, bytes)
}

fn reopen(sim: &Sim) -> Result<SegmentLog, OpenError> {
    SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone()))
}

/// Given three acknowledged groups in an unsealed newest segment
/// When one byte inside the first group frame flips, and the later two frames stay valid
/// Then open fails with `Corrupt` at the first group frame and changes no file
/// And `LogReader` fails with `Corrupt` at the same frame, not `NotSealed`
///
/// Cases: OPEN-DAMAGE-FOLLOWED
#[test]
fn open_refuses_a_bad_frame_with_acknowledged_groups_after_it() {
    bounded(|| {
        let sim = Sim::new(178);
        // All three groups in one unsealed segment, whatever roll size the run sets.
        let unrolled = Options {
            roll_bytes: 1 << 20,
            ..options()
        };
        let log = open_with(&sim, unrolled);
        for i in 0..3u64 {
            let expected = match i {
                0 => ExpectedRevision::Empty,
                _ => ExpectedRevision::At(i - 1),
            };
            let submit = Submit {
                records: vec![NewRecord {
                    stream: b"s".to_vec(),
                    expected,
                    key: Some(format!("k{i}").into_bytes()),
                    payload: format!("record {i}").into_bytes(),
                }],
            };
            let committed = log.append(submit).unwrap().wait().unwrap();
            assert_eq!(committed.records[0].offset, i);
        }
        drop(log);

        let mut segments = raw::segments(&sim);
        let first = codec::HEADER_LEN;
        segments[0].1[first + 40] ^= 0xff;
        raw::install_segments(&sim, &segments);
        let before = sim.image();

        let result = reopen(&sim);
        assert!(
            matches!(
                result,
                Err(OpenError::Corrupt { segment: 0, position }) if position == first as u64
            ),
            "expected Corrupt at 0:{first}, got {:?}",
            result.map(|log| log.watermark())
        );
        assert_eq!(sim.image(), before, "a refused open changes no file");
        // `LogReader` refuses the damage before it would refuse the unsealed log.
        let reader = LogReader::with_storage(Path::new("/log"), Arc::new(sim.clone()));
        assert!(
            matches!(
                reader,
                Err(OpenError::Corrupt { segment: 0, position }) if position == first as u64
            ),
            "expected Corrupt from LogReader, got {:?}",
            reader.err()
        );
        assert_eq!(sim.image(), before);
    });
}

/// Given two acknowledged records in an unsealed segment
/// When a power loss tears the last group, whose payload holds a byte-for-byte copy of the first
/// group frame of this log
/// Then the copy's offset is below the next expected one, so it proves nothing, and open truncates
/// the torn group and keeps both records
///
/// Cases: OPEN-TORN-LOOKALIKE-COPY
#[test]
fn a_lookalike_copy_in_a_torn_payload_still_truncates() {
    bounded(|| {
        let first = raw::group(2, 0, &[b"first", b"second"]);
        let prefix = [raw::header(raw::VERSION, 0, 1), first.clone()].concat();
        let mut torn = raw::group(2, 2, &[&[b"backup:".as_slice(), &first, b":end"].concat()]);
        torn.truncate(torn.len() - 2);
        let sim = Sim::new(1781);
        raw::install(&sim, [prefix.clone(), torn].concat());

        let log = open(&sim);
        assert_eq!(log.watermark(), 2);
        assert_eq!(raw::bytes(&sim), prefix);
    });
}

/// Given a second-newest segment of three groups that lacks its closing run, and a newest segment
/// When one byte inside its second group flips, and its third group stays valid
/// Then open fails with `Corrupt` at the second group, rebuilds no run and changes no file
///
/// Cases: OPEN-DAMAGE-SECOND-NEWEST
#[test]
fn open_refuses_damage_in_a_second_newest_segment_without_its_run() {
    bounded(|| {
        let groups = [
            raw::group(2, 0, &[b"zero"]),
            raw::group(2, 1, &[b"one"]),
            raw::group(2, 2, &[b"two"]),
        ];
        let mut older = [raw::header(raw::VERSION, 0, 1), groups.concat()].concat();
        let cut = codec::HEADER_LEN + groups[0].len();
        older[cut + 40] ^= 0xff;
        let sim = Sim::new(1782);
        raw::install_segments(&sim, &[(0, older), (3, raw::header(raw::VERSION, 3, 2))]);
        let before = sim.image();

        let result = reopen(&sim);
        assert!(
            matches!(
                result,
                Err(OpenError::Corrupt { segment: 0, position }) if position == cut as u64
            ),
            "expected Corrupt at 0:{cut}, got {:?}",
            result.err()
        );
        assert_eq!(sim.image(), before);
    });
}

/// Given an unsealed newest segment whose second group has a flipped byte and whose third is valid
/// When `verify` runs
/// Then it reports class F at the second group, not a torn tail, and changes nothing
///
/// Cases: REPAIR-CLASS-F-VERIFY
#[test]
fn verify_reports_class_f_for_damage_with_a_group_after_it() {
    bounded(|| {
        let (sim, bad, _, _) = damage_before_a_group();
        let before = sim.image();
        let report = verify_with(&sim, Path::new("/log"), raw::VERSION).unwrap();
        let found: Vec<_> = report
            .findings
            .iter()
            .map(|f| (f.class, f.segment, f.position))
            .collect();
        assert_eq!(found, [(Class::F, 0, bad)]);
        assert!(report.damaged());
        assert_eq!(sim.image(), before);
    });
}

/// Given the class F log
/// When `truncate` runs without `--apply`
/// Then the plan is lossy, cuts at the bad group, and drops offsets 1 to 3
/// When `truncate --apply` runs without `--drop-acknowledged`
/// Then it refuses and changes nothing
///
/// Cases: REPAIR-CLASS-F-REFUSED
#[test]
fn a_class_f_plan_is_lossy_and_needs_drop_acknowledged() {
    bounded(|| {
        let (sim, bad, _, _) = damage_before_a_group();
        let before = sim.image();
        let outcome = plan_with(&sim, Path::new("/log"), raw::VERSION).unwrap();
        assert_eq!(outcome.refusal, None);
        let plan = outcome.plan.expect("a class F plan");
        assert_eq!(
            (plan.segment, plan.cut, plan.kind),
            (0, bad, PlanKind::Lossy)
        );
        assert_eq!(plan.dropped_offsets, Some((1, 3)));
        assert!(plan.closing_frames_removed.is_empty());
        let outcome = apply(&sim, (0, bad), false, "repair-1");
        assert!(outcome.refusal.unwrap().contains("--drop-acknowledged"));
        assert!(!outcome.plan.unwrap().applied);
        assert_eq!(sim.image(), before);
    });
}

/// Given the class F log
/// When `truncate --apply --drop-acknowledged` runs at the plan's cut
/// Then the segment ends in a seal's run after the record before the cut
/// And `LogReader` reads that record and none after, and a writable open finds the log sealed, so
/// no dropped offset is assigned again
///
/// Cases: REPAIR-CLASS-F-SEALS
#[test]
fn a_class_f_repair_seals_the_log() {
    bounded(|| {
        let (sim, bad, _, bytes) = damage_before_a_group();
        let outcome = apply(&sim, (0, bad), true, "repair-1");
        assert_eq!(outcome.refusal, None);
        assert!(outcome.plan.unwrap().applied);
        let kept = bytes[..bad as usize].to_vec();
        let run = raw::seal_run(&kept, 2);
        assert_eq!(raw::segments(&sim), [(0, [kept, run].concat())]);
        let reader = LogReader::with_storage(Path::new("/log"), Arc::new(sim.clone())).unwrap();
        let payloads: Vec<_> = reader
            .read(0, 10)
            .unwrap()
            .into_iter()
            .map(|r| r.payload)
            .collect();
        assert_eq!(payloads, [b"a"]);
        let log = reopen(&sim).unwrap();
        assert!(matches!(log.append(submit(b"t")), Err(AppendError::Sealed)));
    });
}

/// Given an unsealed newest segment whose damaged group is too small to hold a seal run before
/// the only group frame after it
/// When `truncate` runs, with or without `--apply`
/// Then it refuses, because the run would overwrite the last proof, and changes nothing
///
/// Cases: REPAIR-CLASS-F-NO-ROOM
#[test]
fn a_class_f_seal_run_that_would_reach_the_last_proof_is_refused() {
    bounded(|| {
        let first = raw::group(1, 0, &[b"a"]);
        let mut second = raw::group(1, 1, &[b"b"]);
        second[40] ^= 0xff;
        let bad = (codec::HEADER_LEN + first.len()) as u64;
        let bytes = [
            raw::header(raw::VERSION, 0, 1),
            first,
            second,
            raw::group(1, 2, &[b"c"]),
        ]
        .concat();
        let sim = Sim::new(1784);
        raw::install_segments(&sim, &[(0, bytes)]);
        let before = sim.image();
        let outcome = plan_with(&sim, Path::new("/log"), raw::VERSION).unwrap();
        assert_eq!(outcome.plan.unwrap().kind, PlanKind::Lossy);
        assert!(
            outcome
                .refusal
                .unwrap()
                .contains("last acknowledged group frame")
        );
        let outcome = apply(&sim, (0, bad), true, "repair-1");
        assert!(outcome.refusal.is_some());
        assert_eq!(sim.image(), before);
    });
}

/// Given a second-newest segment without its closing run, whose second group has a flipped byte
/// and whose third is valid
/// When `verify` and `truncate` run
/// Then `verify` reports class F at the second group, and `truncate` refuses and changes nothing
///
/// Cases: REPAIR-CLASS-F-SECOND-NEWEST
#[test]
fn class_f_in_the_second_newest_segment_is_diagnosed_only() {
    bounded(|| {
        let groups = [
            raw::group(2, 0, &[b"zero"]),
            raw::group(2, 1, &[b"one"]),
            raw::group(2, 2, &[b"two"]),
        ];
        let mut older = [raw::header(raw::VERSION, 0, 1), groups.concat()].concat();
        let cut = (codec::HEADER_LEN + groups[0].len()) as u64;
        older[cut as usize + 40] ^= 0xff;
        let sim = Sim::new(1785);
        raw::install_segments(&sim, &[(0, older), (3, raw::header(raw::VERSION, 3, 2))]);
        let before = sim.image();
        let report = verify_with(&sim, Path::new("/log"), raw::VERSION).unwrap();
        let found: Vec<_> = report
            .findings
            .iter()
            .map(|f| (f.class, f.segment, f.position))
            .collect();
        assert_eq!(found, [(Class::F, 0, cut)]);
        let outcome = plan_with(&sim, Path::new("/log"), raw::VERSION).unwrap();
        assert!(outcome.refusal.unwrap().contains("class F"));
        let outcome = apply(&sim, (0, cut), true, "repair-1");
        assert!(outcome.refusal.is_some());
        assert_eq!(sim.image(), before);
    });
}

/// Given the class F log, and the trace of a `truncate --apply --drop-acknowledged` on it
/// When Kill, PowerLoss or PowerLossTorn cuts that trace at each storage call
/// Then open fails with `Corrupt` at the cut, with every byte before it unchanged, or finds the
/// sealed repaired log; it never finds a writable log, which would assign a dropped offset again
/// And after `Corrupt`, a second `--apply` converges to the same repaired log
///
/// Cases: REPAIR-CLASS-F-CRASH
#[test]
fn a_crash_at_each_step_of_a_class_f_repair_fails_closed_or_finds_it_sealed() {
    bounded(|| {
        let (sim, bad, _, original) = damage_before_a_group();
        let start = sim.trace().1.len();
        apply(&sim, (0, bad), true, "repair-1");
        let repaired = raw::segments(&sim)[0].1.clone();
        let (seed, trace) = sim.trace();
        let (mut closed, mut opened) = (0, 0);
        for step in start..=trace.len() {
            for mode in [
                storage::sim::Crash::Kill,
                storage::sim::Crash::PowerLoss,
                storage::sim::Crash::PowerLossTorn,
            ] {
                for crash_seed in 0..3 {
                    let crashed = Sim::replay(seed, &trace[..step]);
                    crashed.crash(mode, crash_seed);
                    let label = format!("step {step} {mode:?} {crash_seed}");
                    match reopen(&crashed) {
                        Err(OpenError::Corrupt {
                            segment: 0,
                            position,
                        }) => {
                            assert_eq!(position, bad, "{label}");
                            let now = &raw::segments(&crashed)[0].1;
                            assert_eq!(now[..bad as usize], original[..bad as usize], "{label}");
                            closed += 1;
                            let again = apply(&crashed, (0, bad), true, "repair-2");
                            assert!(again.plan.unwrap().applied, "{label}");
                            assert_eq!(raw::segments(&crashed)[0].1, repaired, "{label}");
                        }
                        Ok(log) => {
                            opened += 1;
                            assert_eq!(raw::segments(&crashed)[0].1, repaired, "{label}");
                            let read = log.read(0, 10).unwrap();
                            assert_eq!(read.len(), 1, "{label}");
                            let appended = log.append(submit(b"t"));
                            assert!(matches!(appended, Err(AppendError::Sealed)), "{label}");
                        }
                        Err(other) => panic!("{label}: {other:?}"),
                    }
                }
            }
        }
        assert!(closed > 0 && opened > 0, "{closed} {opened}");
    });
}

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name).map_or(default, |v| v.parse().expect("numeric damage setting"))
}

/// The next value of a splitmix64 sequence, so that one seed rebuilds a case.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Given seeded logs of two to seven acknowledged groups in one unsealed segment
/// When one byte anywhere before the start of the last group frame flips
/// Then open fails with `Corrupt`, or keeps every acknowledged offset and record; it never returns
/// `Ok` with a lower watermark
/// A failure prints the seed, and `Sim` saves its storage trace, which rebuilds the damaged image
///
/// Cases: OPEN-DAMAGE-RANDOM
#[test]
fn one_flipped_byte_before_the_last_group_never_loses_an_acknowledged_record() {
    let seed = setting("SEGMENT_LOG_DAMAGE_SEED", 178);
    let cases = setting("SEGMENT_LOG_DAMAGE_CASES", 64);
    bounded(move || {
        // One segment, whatever roll size the run sets.
        let unrolled = || Options {
            roll_bytes: 1 << 20,
            ..options()
        };
        for case in 0..cases {
            let mut state = seed ^ case.wrapping_mul(0xD6E8_FEB8_6659_FD93);
            let sim = Sim::new(seed.wrapping_add(case));
            let log =
                SegmentLog::with_storage(Path::new("/log"), unrolled(), Arc::new(sim.clone()))
                    .unwrap();
            let groups = 2 + next(&mut state) % 6;
            let mut written = vec![];
            for i in 0..groups {
                let stream = format!("s{i}").into_bytes();
                let payload = vec![i as u8; 1 + (next(&mut state) % 300) as usize];
                let submit = Submit {
                    records: vec![NewRecord {
                        stream,
                        expected: ExpectedRevision::Empty,
                        key: Some(format!("k{i}").into_bytes()),
                        payload: payload.clone(),
                    }],
                };
                log.append(submit).unwrap().wait().unwrap();
                written.push(payload);
            }
            drop(log);
            let mut segments = raw::segments(&sim);
            let segment = &mut segments[0].1;
            let last = segment.len() - raw::frames(segment).last().unwrap().len();
            let at = codec::HEADER_LEN + (next(&mut state) as usize) % (last - codec::HEADER_LEN);
            segment[at] ^= 1 << (next(&mut state) % 8);
            raw::install_segments(&sim, &segments);
            let replay = format!(
                "case {case} byte {at}; replay: SEGMENT_LOG_DAMAGE_SEED={seed} \
                 SEGMENT_LOG_DAMAGE_CASES={cases} cargo test -p segment-log-core --lib \
                 one_flipped_byte"
            );
            match SegmentLog::with_storage(Path::new("/log"), unrolled(), Arc::new(sim.clone())) {
                Err(OpenError::Corrupt { .. }) => (),
                Ok(log) => {
                    assert_eq!(log.watermark(), groups, "{replay}");
                    let read: Vec<_> = log
                        .read(0, 10)
                        .unwrap()
                        .into_iter()
                        .map(|r| r.payload)
                        .collect();
                    assert_eq!(read, written, "{replay}");
                }
                Err(other) => panic!("{replay}: {other:?}"),
            }
        }
    });
}

/// Given a second-newest segment whose closing run the sealer had not yet written (ADR 0061), and
/// a newest segment of three groups whose second has a flipped byte and whose third is valid
/// When `verify` and `truncate` run
/// Then `verify` reports the missing run and class F, and `truncate` refuses and changes nothing,
/// because sealing the newest segment would leave a missing run that open never rebuilds
///
/// Cases: REPAIR-CLASS-F-PENDING-RUN
#[test]
fn class_f_with_a_pending_closing_run_is_refused() {
    bounded(|| {
        let older = [
            raw::header(raw::VERSION, 0, 1),
            raw::group(1, 0, &[b"zero"]),
        ]
        .concat();
        let first = raw::group(2, 1, &[b"a"]);
        let mut second = raw::group(2, 2, &[&[b'b'; 200]]);
        second[40] ^= 0xff;
        let third = raw::group(2, 3, &[&[b'c'; 200]]);
        let cut = (codec::HEADER_LEN + first.len()) as u64;
        let newest = [raw::header(raw::VERSION, 1, 2), first, second, third].concat();
        let sim = Sim::new(1786);
        raw::install_segments(&sim, &[(0, older.clone()), (1, newest)]);
        let before = sim.image();
        let report = verify_with(&sim, Path::new("/log"), raw::VERSION).unwrap();
        let found: Vec<_> = report
            .findings
            .iter()
            .map(|f| (f.class, f.segment, f.position))
            .collect();
        assert_eq!(
            found,
            [(Class::TornTail, 0, older.len() as u64), (Class::F, 1, cut)]
        );
        let outcome = plan_with(&sim, Path::new("/log"), raw::VERSION).unwrap();
        assert!(
            outcome
                .refusal
                .as_deref()
                .unwrap_or("")
                .contains("closing run"),
            "{:?}",
            outcome.refusal
        );
        let outcome = apply(&sim, (1, cut), true, "repair-1");
        assert!(outcome.refusal.is_some());
        assert_eq!(sim.image(), before);
    });
}

/// Given three segments, the oldest sealed with a flipped byte in its second group and in its
/// locator, and a valid group after the damaged one
/// When `verify` runs
/// Then it reports class B at the damaged group, not class F: open checks for a proof only in the
/// newest segment and in a second-newest segment without its run (ADR 0068)
///
/// Cases: REPAIR-CLASS-F-SECOND-NEWEST
#[test]
fn class_f_is_not_reported_for_an_older_sealed_segment() {
    bounded(|| {
        let groups = [
            raw::group(1, 0, &[b"zero"]),
            raw::group(1, 1, &[b"one"]),
            raw::group(1, 2, &[b"two"]),
        ];
        let mut oldest = raw::rolled(
            [raw::header(raw::VERSION, 0, 1), groups.concat()].concat(),
            1,
        );
        let bad = codec::HEADER_LEN + groups[0].len();
        oldest[bad + 40] ^= 0xff;
        let locator = oldest.len() - 1;
        oldest[locator] ^= 0xff;
        let middle = raw::rolled(
            [raw::header(raw::VERSION, 3, 1), raw::group(1, 3, &[b"x"])].concat(),
            1,
        );
        let sim = Sim::new(1787);
        raw::install_segments(
            &sim,
            &[
                (0, oldest),
                (3, middle),
                (4, raw::header(raw::VERSION, 4, 1)),
            ],
        );
        let report = verify_with(&sim, Path::new("/log"), raw::VERSION).unwrap();
        let found: Vec<_> = report
            .findings
            .iter()
            .map(|f| (f.class, f.segment, f.position))
            .collect();
        assert_eq!(found, [(Class::B, 0, bad as u64)]);
    });
}

/// Given an unsealed segment whose second and third groups each hold one record of almost 16 MiB
/// with a flipped byte, and a valid fourth group after them, more than 32 MiB past the damage
/// When the log opens
/// Then the search refills its window and still finds the fourth group, so open fails with
/// `Corrupt` at the second group
///
/// Cases: OPEN-DAMAGE-FOLLOWED
#[test]
fn a_proof_past_the_search_window_is_found() {
    bounded(|| {
        let first = raw::group(1, 0, &[b"a"]);
        let mut damaged = raw::group(1, 1, &[&vec![0xff; codec::FRAME_BODY_LIMIT - 64]]);
        damaged[40] ^= 0xff;
        let cut = (codec::HEADER_LEN + first.len()) as u64;
        let bytes = [
            raw::header(raw::VERSION, 0, 1),
            first,
            damaged.clone(),
            damaged,
            raw::group(1, 3, &[b"c"]),
        ]
        .concat();
        let sim = Sim::new(1788);
        raw::install_segments(&sim, &[(0, bytes)]);
        let result = reopen(&sim);
        assert!(
            matches!(result, Err(OpenError::Corrupt { segment: 0, position }) if position == cut),
            "expected Corrupt at 0:{cut}, got {:?}",
            result.map(|log| log.watermark())
        );
    });
}
