use crate::acceptance::support::{bounded, fixtures::*, raw};
use crate::storage::sim::Crash;
use crate::*;
use std::{path::Path, sync::Arc};

fn try_open(sim: &Sim) -> Result<SegmentLog, OpenError> {
    SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone()))
}

fn groups(base: u64, epoch: u64, payloads: &[&[u8]]) -> Vec<u8> {
    [
        raw::header(raw::VERSION, base, epoch),
        raw::group(epoch, base, payloads),
    ]
    .concat()
}

fn sealed(base: u64, epoch: u64, payloads: &[&[u8]]) -> Vec<u8> {
    raw::rolled(groups(base, epoch, payloads), epoch)
}

fn next(log: &SegmentLog, revision: u64, payload: &[u8]) -> Position {
    log.append(Submit {
        records: vec![NewRecord {
            stream: b"s".to_vec(),
            key: None,
            payload: payload.to_vec(),
            expected: ExpectedRevision::At(revision),
        }],
    })
    .unwrap()
    .wait()
    .unwrap()
    .records
    .remove(0)
}

/// Given three hand-made segments, where the two older ones hold valid groups and a closing run
/// And the newest one ends in half a frame
/// When the log opens
/// Then `read` returns every record across the files in offset order with its revision
/// And the older segments keep their bytes, and the newest one is cut to its valid prefix
/// And a later append takes the next offset under an epoch above every epoch in the files
/// And a power loss after the open gives the same records
///
/// Cases: OPEN-LAYOUT, RECOVERY-FIXTURES, ORACLE-5, ORACLE-6
#[test]
fn recovery_reads_every_record_across_segments() {
    bounded(|| {
        let sim = Sim::new(301);
        let first = raw::rolled(
            [
                raw::header(raw::VERSION, 0, 1),
                raw::group(1, 0, &[b"a", b"b"]),
                raw::group(1, 2, &[b"c"]),
            ]
            .concat(),
            1,
        );
        let second = raw::rolled(
            [raw::header(raw::VERSION, 3, 1), raw::group(2, 3, &[b"d"])].concat(),
            2,
        );
        let kept = [
            raw::header(raw::VERSION, 4, 2),
            raw::group(2, 4, &[b"e", b"f"]),
        ]
        .concat();
        let torn = raw::group(2, 6, &[b"never acknowledged"]);
        raw::install_segments(
            &sim,
            &[
                (0, first.clone()),
                (3, second.clone()),
                (4, [kept.clone(), torn[..torn.len() / 2].to_vec()].concat()),
            ],
        );
        let log = open(&sim);
        let expected: Vec<_> = [b"a", b"b", b"c", b"d", b"e", b"f"]
            .iter()
            .enumerate()
            .map(|(i, payload)| stored(b"s", None, *payload, i as u64, i as u64))
            .collect();
        assert_eq!(log.watermark(), 6);
        assert_records(&log.read(0, 100).unwrap(), &expected);
        assert_records(&log.read(2, 3).unwrap(), &expected[2..5]);
        let head = log.stream_head(b"s").unwrap();
        assert_eq!((head.offset, head.revision), (5, 5));
        assert_eq!(
            raw::segments(&sim),
            [(0, first.clone()), (3, second.clone()), (4, kept)]
        );
        let position = next(&log, 5, b"g");
        assert_eq!((position.offset, position.revision), (6, 6));
        let segments = raw::segments(&sim);
        assert_eq!(segments[..2], [(0, first), (3, second)]);
        assert!(
            segments
                .last()
                .unwrap()
                .1
                .ends_with(&raw::group(3, 6, &[b"g"])),
            "the new frame carries the epoch after the highest one in the files"
        );
        drop(log);
        sim.crash(Crash::PowerLoss, 1);
        let log = open(&sim);
        assert_eq!(log.watermark(), 7);
        assert_records(&log.read(0, 6).unwrap(), &expected);
    });
}

/// Given segments whose base offsets leave a hole, overlap the segment before, or do not start at 0
/// When the log opens
/// Then open fails with `Gap` and names the segment that does not start where the one before it ends
/// And no file changes
///
/// Cases: OPEN-LAYOUT, RECOVERY-FIXTURES
#[test]
fn a_missing_segment_or_a_mismatched_base_is_a_gap() {
    bounded(|| {
        let first = sealed(0, 1, &[b"a", b"b"]);
        for (segments, gap) in [
            (vec![(0, first.clone()), (5, sealed(5, 1, &[b"f"]))], 5),
            (vec![(0, first.clone()), (1, sealed(1, 1, &[b"b"]))], 1),
            (
                vec![(0, first.clone()), (3, raw::header(raw::VERSION, 3, 1))],
                3,
            ),
            (vec![(2, sealed(2, 1, &[b"c"]))], 2),
        ] {
            let sim = Sim::new(302);
            raw::install_segments(&sim, &segments);
            let before = sim.image();
            let result = try_open(&sim);
            assert!(
                matches!(result, Err(OpenError::Gap { segment }) if segment == gap),
                "expected a gap at {gap}: {:?}",
                result.err()
            );
            assert_eq!(sim.image(), before);
        }
    });
}

/// Given a second segment whose header carries another log's id
/// As a segment with a successor, as the newest segment with records, and as a newest segment with only a header
/// When the log opens
/// Then open fails with `LogIdMismatch` and names that segment
/// And it deletes no file and changes no byte
///
/// Cases: RECOVERY-FIXTURES, OPEN-LAYOUT
#[test]
fn a_segment_from_another_log_fails_open_without_deletion() {
    bounded(|| {
        let first = sealed(0, 1, &[b"a", b"b"]);
        let foreign = |bytes: Vec<u8>| {
            let header = raw::with_log_id(bytes[..64].to_vec(), [9; 16]);
            [header, bytes[64..].to_vec()].concat()
        };
        for segments in [
            vec![
                (0, first.clone()),
                (2, foreign(sealed(2, 1, &[b"c"]))),
                (3, sealed(3, 1, &[b"d"])),
            ],
            vec![(0, first.clone()), (2, foreign(sealed(2, 1, &[b"c"])))],
            vec![
                (0, first.clone()),
                (2, foreign(raw::header(raw::VERSION, 2, 1))),
            ],
        ] {
            let sim = Sim::new(303);
            raw::install_segments(&sim, &segments);
            let before = sim.image();
            let result = try_open(&sim);
            assert!(
                matches!(result, Err(OpenError::LogIdMismatch { segment: 2 })),
                "{:?}",
                result.err()
            );
            assert_eq!(sim.image(), before);
        }
    });
}

/// Given a segment with a successor and a valid closing run, where one group frame has a bad
/// checksum, an epoch below the frame before it, a base that does not follow the frame before it,
/// or zeros in place of its bytes
/// When the log opens
/// Then open succeeds, because it reads no group frame of that segment, and changes no file
/// And a read of that segment fails with `ReadError::Corrupt` at that frame's position
/// And a read from the next segment returns its records
///
/// Cases: RECOVERY-FIXTURES, OPEN-LAYOUT, ORACLE-6, ORACLE-13
#[test]
fn a_bad_frame_in_a_sealed_segment_fails_its_read_and_changes_nothing() {
    bounded(|| {
        let valid = groups(0, 2, &[b"a", b"b"]);
        let good = raw::group(2, 2, &[b"c"]);
        let mut bad_crc = good.clone();
        bad_crc[4] ^= 1;
        let sealed_first = raw::rolled([valid.clone(), good.clone()].concat(), 2);
        let at = valid.len();
        for bad in [
            bad_crc,
            raw::group(1, 2, &[b"c"]),
            raw::group(2, 3, &[b"c"]),
            vec![0; good.len()],
        ] {
            let mut damaged = sealed_first.clone();
            damaged[at..at + good.len()].copy_from_slice(&bad);
            let sim = Sim::new(304);
            raw::install_segments(
                &sim,
                &[
                    (0, damaged),
                    (3, sealed(3, 2, &[b"d"])),
                    (4, raw::header(raw::VERSION, 4, 2)),
                ],
            );
            let before = raw::segments(&sim);
            let log = try_open(&sim).unwrap();
            let result = log.read(0, 10);
            assert!(
                matches!(
                    result,
                    Err(ReadError::Corrupt { segment: 0, position })
                        if position == at as u64
                ),
                "{result:?}"
            );
            assert_eq!(log.read(3, 10).unwrap()[0].payload, b"d");
            drop(log);
            assert_eq!(raw::segments(&sim), before);
        }
    });
}

/// Given a segment sealed by a roll and a newer file no longer than a header whose header checksum
/// does not match
/// When the log opens
/// Then the newer file is replaced by a whole header-only segment under the next writer epoch, as
/// the roll would have written it, and the records of the sealed segment are readable
/// And that state survives a power loss, and the next append takes the next offset
///
/// Cases: RECOVERY-FIXTURES, OPEN-LAYOUT
#[test]
fn a_torn_newest_segment_no_longer_than_a_header_is_deleted() {
    bounded(|| {
        let first = sealed(0, 1, &[b"a", b"b"]);
        let mut bad_crc = raw::header(raw::VERSION, 2, 1);
        bad_crc[60] ^= 1;
        let replaced = [(0, first.clone()), (2, raw::header(raw::VERSION, 2, 2))];
        for torn in [vec![], vec![0; 1], vec![0; 63], vec![0; 64], bad_crc] {
            let sim = Sim::new(305);
            raw::install_segments(&sim, &[(0, first.clone()), (2, torn)]);
            let log = open(&sim);
            assert_eq!(log.watermark(), 2);
            assert_eq!(raw::segments(&sim), replaced);
            drop(log);
            sim.crash(Crash::PowerLoss, 2);
            assert_eq!(raw::segments(&sim), replaced);
            let log = open(&sim);
            let payloads: Vec<_> = log
                .read(0, 100)
                .unwrap()
                .into_iter()
                .map(|record| record.payload)
                .collect();
            assert_eq!(payloads, [b"a", b"b"]);
            let position = next(&log, 1, b"c");
            assert_eq!((position.offset, position.revision), (2, 2));
        }
    });
}

/// Given a frame whose epoch is lower than the epoch of the last frame in the segment before it
/// When that frame opens the newest segment
/// Then it ends the log, and recovery cuts it and keeps every record before it
/// And the next append survives a reopen and a power loss, because the writer epoch is above every
/// epoch in the files and not only above the newest segment's
/// When that frame opens a segment that has a successor, whose closing run has the same epoch
/// Then open fails with `Corrupt` at that segment's footer and changes no file
/// When that closing run has the epoch of the segment before
/// Then open succeeds, and a read of that segment fails with `ReadError::Corrupt` at that frame
///
/// Cases: RECOVERY-FIXTURES, ORACLE-1
#[test]
fn a_lower_epoch_than_the_segment_before_ends_the_log() {
    bounded(|| {
        let first = sealed(0, 3, &[b"a", b"b"]);
        let stale = [
            raw::header(raw::VERSION, 2, 1),
            raw::group(2, 2, &[b"stale"]),
        ]
        .concat();
        let sim = Sim::new(306);
        raw::install_segments(&sim, &[(0, first.clone()), (2, stale.clone())]);
        let log = open(&sim);
        assert_eq!(log.watermark(), 2);
        assert_eq!(
            raw::segments(&sim),
            [(0, first.clone()), (2, raw::header(raw::VERSION, 2, 1))]
        );
        let position = next(&log, 1, b"c");
        assert_eq!((position.offset, position.revision), (2, 2));
        drop(log);
        for crash in [None, Some(Crash::PowerLoss)] {
            if let Some(mode) = crash {
                sim.crash(mode, 6);
            }
            let payloads: Vec<_> = open(&sim)
                .read(0, 100)
                .unwrap()
                .into_iter()
                .map(|record| record.payload)
                .collect();
            assert_eq!(payloads, [b"a", b"b", b"c"], "after {crash:?}");
        }

        let footer_at = |segment: &[u8]| {
            let frames = raw::frames(segment);
            (segment.len() - frames[frames.len() - 2..].concat().len()) as u64
        };
        for run_epoch in [2, 3] {
            let sim = Sim::new(306);
            let stale = raw::rolled(stale.clone(), run_epoch);
            raw::install_segments(
                &sim,
                &[
                    (0, first.clone()),
                    (2, stale.clone()),
                    (3, raw::header(raw::VERSION, 3, 3)),
                ],
            );
            let before = sim.image();
            let result = try_open(&sim);
            if run_epoch == 2 {
                assert!(
                    matches!(
                        result,
                        Err(OpenError::Corrupt { segment: 2, position })
                            if position == footer_at(&stale)
                    ),
                    "{:?}",
                    result.err()
                );
                assert_eq!(sim.image(), before);
            } else {
                let log = result.unwrap();
                assert!(matches!(
                    log.read(2, 10),
                    Err(ReadError::Corrupt {
                        segment: 2,
                        position: 64
                    })
                ));
            }
        }
    });
}

/// Given two sealed segments and a newest file that recovery must repair
/// As a segment that ends in half a frame, and as a file shorter than a header
/// When a crash interrupts that recovery at each storage call, under each crash mode
/// Then the next open returns every record of the valid prefix
/// And the sealed segments keep their bytes
///
/// Cases: RECOVERY-FIXTURES, OPEN-LAYOUT, ORACLE-1, ORACLE-5, ORACLE-6
#[test]
fn a_crash_at_each_step_of_a_multi_segment_recovery_keeps_the_valid_prefix() {
    bounded(|| {
        let sealed_segments = [(0, sealed(0, 1, &[b"a", b"b"])), (2, sealed(2, 1, &[b"c"]))];
        let torn = raw::group(1, 4, &[b"never acknowledged"]);
        for (newest, kept) in [
            (
                [groups(3, 1, &[b"d"]), torn[..torn.len() / 2].to_vec()].concat(),
                4,
            ),
            (vec![0xff; 40], 3),
        ] {
            let sim = Sim::new(307);
            raw::install_segments(
                &sim,
                &[sealed_segments.to_vec(), vec![(3, newest)]].concat(),
            );
            let start = sim.trace().1.len();
            drop(open(&sim));
            let (seed, trace) = sim.trace();
            for cut in start..=trace.len() {
                for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
                    let replay = Sim::replay(seed, &trace[..cut]);
                    replay.crash(mode, cut as u64);
                    let log = open(&replay);
                    assert_eq!(log.watermark(), kept, "cut {cut} {mode:?}");
                    let payloads: Vec<_> = log
                        .read(0, 100)
                        .unwrap()
                        .into_iter()
                        .map(|record| record.payload)
                        .collect();
                    assert_eq!(payloads, [b"a", b"b", b"c", b"d"][..kept as usize]);
                    assert_eq!(raw::segments(&replay)[..2], sealed_segments);
                }
            }
        }
    });
}
