//! The operator repair tool (ADR 0065): `verify` reports, and never changes a segment.
//! `truncate` is in [`super::repair_truncate`].
use super::closing_runs::closing_run_at;
use super::support::{bounded, fixtures::*, raw};
use crate::repair::{Apply, Class, Error, Report, apply_with, plan_with, verify_with};
use crate::storage::{Call, Op, Purpose, Storage, sim::Event, sim::Outcome};
use crate::{codec, *};
use std::path::Path;

pub(super) const VERSION: u16 = codec::FORMAT_VERSION;

fn verify(sim: &Sim) -> Report {
    verify_with(sim, Path::new("/log"), VERSION).unwrap()
}

/// Calls that changed the image: a successful create, write, truncate, removal or sync.
pub(super) fn changes(sim: &Sim) -> usize {
    sim.trace()
        .1
        .iter()
        .filter(|event| {
            matches!(event, Event::Call(call, _, Outcome::Ok(_))
                if !matches!(call.op, Op::Read(..) | Op::Open | Op::List | Op::Random(_)))
        })
        .count()
}

/// Runs `verify` and checks that it changed nothing.
fn verify_unchanged(sim: &Sim) -> Report {
    let (before, calls) = (sim.image(), changes(sim));
    let report = verify(sim);
    assert_eq!(sim.image(), before);
    assert_eq!(changes(sim), calls, "verify changed the storage");
    report
}

pub(super) fn classes(report: &Report) -> Vec<(Class, u64, u64)> {
    report
        .findings
        .iter()
        .map(|f| (f.class, f.segment, f.position))
        .collect()
}

fn keyed(n: usize) -> Submit {
    let key = format!("key-{n}").into_bytes();
    Submit {
        records: vec![NewRecord {
            stream: format!("s-{}", n % 3).into_bytes(),
            expected: ExpectedRevision::Empty,
            key: Some(key.clone()),
            payload: key,
        }],
    }
}

/// A log of keyed records in several segments: each segment past the first sealed by a roll, and
/// the newest active.
pub(super) fn rolled_log(sim: &Sim) -> Vec<(u64, Vec<u8>)> {
    let options = Options {
        roll_bytes: 150,
        ..options()
    };
    let log = open_with(sim, options);
    for n in 0..8 {
        let mut submit = keyed(n);
        let head = log.stream_head(&submit.records[0].stream);
        submit.records[0].expected = head.map_or(ExpectedRevision::Empty, |h| {
            ExpectedRevision::At(h.revision)
        });
        log.append(submit).unwrap().wait().unwrap();
    }
    drop(log);
    let segments = raw::segments(sim);
    assert!(segments.len() >= 3, "the fixture rolls: {}", segments.len());
    segments
}

/// The position and kind byte of each frame of a segment image.
pub(super) fn kinds(segment: &[u8]) -> Vec<(usize, u8)> {
    let mut at = 64;
    raw::frames(segment)
        .into_iter()
        .map(|frame| {
            let out = (at, frame[8]);
            at += frame.len();
            out
        })
        .collect()
}

pub(super) fn position_of(segment: &[u8], kind: u8) -> usize {
    kinds(segment)
        .into_iter()
        .find(|&(_, k)| k == kind)
        .map(|(at, _)| at)
        .unwrap()
}

/// Given a healthy log of several segments, a sealed log, and a log whose newest segment ends in a
/// torn tail that open would truncate
/// When `verify` runs
/// Then it finds no damage, and no storage call changes anything
/// And for the torn tail it reports where open would truncate
///
/// Cases: REPAIR-VERIFY-CLEAN
#[test]
fn verify_finds_no_damage_in_a_healthy_log() {
    bounded(|| {
        let sim = Sim::new(650);
        rolled_log(&sim);
        let report = verify_unchanged(&sim);
        assert_eq!(classes(&report), []);
        assert!(report.frames.is_empty());

        let sim = Sim::new(651);
        let log = open(&sim);
        append(&log, b"s").wait().unwrap();
        log.seal().unwrap();
        drop(log);
        assert_eq!(classes(&verify_unchanged(&sim)), []);

        let sim = Sim::new(652);
        let log = open(&sim);
        append(&log, b"s").wait().unwrap();
        drop(log);
        let (path, at, frame) = raw::last_frame(&sim);
        let end = at + frame.len() as u64;
        sim.call(Call::new(&path, Purpose::Open, Op::Write(end, vec![9; 50])))
            .unwrap();
        let report = verify_unchanged(&sim);
        let base = raw::segments(&sim).last().unwrap().0;
        assert_eq!(classes(&report), [(Class::TornTail, base, end)]);
        assert!(!report.damaged());
    });
}

/// Given a log whose oldest segment has damage that open does not read: a bad group frame (B), a
/// closing run that passes its checks but differs from its groups (C), or a bad key index frame (D)
/// When `verify` runs
/// Then it names the class, the segment and the frame, and changes nothing
///
/// Given a log with a missing segment, a segment of another log, a bad header in an older segment,
/// or a header of a format version this build does not read (E)
/// When `verify` runs
/// Then it names the segment, and changes nothing
/// And `truncate` refuses every class B–E, and changes nothing
///
/// Given the newest segment ends in a torn group whose payload holds the log's own closing run, so
/// open fails with `Corrupt` (A)
/// When `verify` runs
/// Then it reports class A where open stops, and the frames around it
///
/// Cases: REPAIR-VERIFY-CLASSES
#[test]
fn verify_names_each_class_of_damage() {
    bounded(|| {
        let healthy = rolled_log(&Sim::new(653));
        let (first, second) = (healthy[0].1.clone(), healthy[1].0);
        let damaged = |label: &str, segments: Vec<(u64, Vec<u8>)>| {
            let sim = Sim::new(654);
            raw::install_segments(&sim, &segments);
            let report = verify_unchanged(&sim);
            assert!(report.damaged(), "{label}: {:?}", report.findings);
            let before = sim.image();
            let outcome = plan_with(&sim, Path::new("/log"), VERSION).unwrap();
            assert!(
                outcome.plan.is_none() && outcome.refusal.is_some(),
                "{label}"
            );
            let apply = Apply {
                at: (0, 64),
                drop_acknowledged: true,
                backup: "repair-1",
            };
            let outcome = apply_with(&sim, Path::new("/log"), VERSION, apply).unwrap();
            assert!(outcome.refusal.is_some(), "{label}");
            assert_eq!(sim.image(), before, "{label}");
            classes(&report)
        };
        let with_first = |bytes: Vec<u8>| {
            let mut segments = healthy.clone();
            segments[0].1 = bytes;
            segments
        };

        let mut bytes = first.clone();
        bytes[64 + 40] ^= 1;
        assert_eq!(damaged("B", with_first(bytes)), [(Class::B, 0, 64)]);

        let filter_at = position_of(&first, 5);
        let footer_at = position_of(&first, 2);
        let mut bytes = first.clone();
        // The first footer entry's last revision, with the frame CRC recomputed.
        bytes[footer_at + 8 + 32 + 4] ^= 1;
        let length = bytes.len() - codec::LOCATOR_LEN - footer_at;
        raw::rechecksum(
            &raw::header_fields(&first).0,
            &mut bytes[footer_at..footer_at + length],
        );
        assert_eq!(
            damaged("C", with_first(bytes)),
            [(Class::C, 0, filter_at as u64)]
        );

        let index_at = position_of(&first, 6);
        let mut bytes = first.clone();
        bytes[index_at + 8 + 24 + 20] ^= 1;
        assert_eq!(
            damaged("D", with_first(bytes)),
            [(Class::D, 0, index_at as u64)]
        );

        let mut missing = healthy.clone();
        missing.remove(1);
        let third = missing[1].0;
        assert_eq!(damaged("gap", missing), [(Class::E, third, 0)]);

        let mut other = healthy.clone();
        let header = raw::with_log_id(other[1].1[..64].to_vec(), [9; 16]);
        other[1].1[..64].copy_from_slice(&header);
        assert_eq!(damaged("log id", other), [(Class::E, second, 0)]);

        let mut bytes = first.clone();
        bytes[30] ^= 1;
        assert_eq!(damaged("header", with_first(bytes)), [(Class::E, 0, 0)]);

        let mut bytes = first.clone();
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        let checksum = raw::crc(&bytes[..60]);
        bytes[60..64].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(damaged("version", with_first(bytes)), [(Class::E, 0, 0)]);

        let (sim, torn_at) = lookalike();
        let report = verify_unchanged(&sim);
        assert_eq!(classes(&report), [(Class::A, 0, torn_at)]);
        let around: Vec<_> = report
            .frames
            .iter()
            .map(|f| (f.position, f.kind, f.valid))
            .collect();
        let locator_at = raw::segments(&sim)[0].1.len() - codec::LOCATOR_LEN;
        assert_eq!(around[0], (64, "group", true));
        assert_eq!(around[1], (torn_at, "group", false));
        assert_eq!(around[3], (locator_at as u64, "locator", true));
        assert_eq!(around[2].1, "footer");
    });
}

/// The trigger of issue #1: a torn group whose payload ends in a closing run under the log's own
/// id. Returns the log and the torn group's position.
pub(super) fn lookalike() -> (Sim, u64) {
    let kept = [raw::header(raw::VERSION, 0, 1), raw::group(1, 0, &[b"a"])].concat();
    let torn_at = kept.len();
    let prefix = vec![b'p'; 3 * 4096];
    let filter_at = torn_at + 8 + 32 + 16 + b"s".len() + prefix.len();
    let run = closing_run_at(&codec::LogId::new(raw::LOG_ID), filter_at, 2);
    let mut torn = raw::group(1, 1, &[&[prefix, run].concat()]);
    torn[4096..2 * 4096].fill(0);
    let sim = Sim::new(655);
    raw::install_segments(&sim, &[(0, [kept, torn].concat())]);
    assert!(matches!(
        SegmentLog::with_storage(Path::new("/log"), options(), std::sync::Arc::new(sim.clone())),
        Err(OpenError::Corrupt { segment: 0, position }) if position == torn_at as u64
    ));
    (sim, torn_at as u64)
}

/// Given a writer that holds `LOCK`
/// When `verify` runs
/// Then it fails with `Locked` and changes nothing
///
/// Cases: REPAIR-LOCKED
#[test]
fn verify_fails_while_another_process_holds_the_lock() {
    bounded(|| {
        let sim = Sim::new(656);
        let log = open(&sim);
        let before = sim.image();
        let result = verify_with(&sim, Path::new("/log"), VERSION);
        assert!(matches!(result, Err(Error::Locked)), "{result:?}");
        assert_eq!(sim.image(), before);
        drop(log);
    });
}
