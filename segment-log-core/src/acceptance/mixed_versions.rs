//! ADR 0064, rules 11–14 and Enforcement: a log written at format version N, opened and appended
//! to by a build at N+1. Until a real version 4 exists, version 4 is a test-only bump: the same
//! bytes as version 3 under another `format_version`.
use super::support::{bounded, fixtures::*, oracle, raw};
use crate::engine::{Hooks, Limits};
use crate::storage::sim::Crash;
use crate::*;
use std::{path::Path, sync::Arc};

const NEXT: u16 = 4;
const MODES: [Crash; 3] = [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn];

/// Opens the log as a build that writes `version` and reads 3 up to it.
fn open_at(sim: &Sim, version: u16, roll_bytes: u64) -> Result<SegmentLog, OpenError> {
    let hooks = Hooks {
        limits: Limits {
            format_version: version,
            ..Limits::default()
        },
        ..Hooks::default()
    };
    let options = Options {
        roll_bytes,
        ..options()
    };
    SegmentLog::with_hooks(Path::new("/log"), options, Arc::new(sim.clone()), hooks)
}

fn keyed(key: &[u8]) -> Submit {
    Submit {
        records: vec![NewRecord {
            stream: key.to_vec(),
            expected: ExpectedRevision::Empty,
            key: Some(key.to_vec()),
            payload: key.to_vec(),
        }],
    }
}

fn append(log: &SegmentLog, key: &[u8]) -> Committed {
    log.append(keyed(key)).unwrap().wait().unwrap()
}

fn payloads(log: &SegmentLog) -> Vec<Vec<u8>> {
    let records = log.read(0, 100).unwrap();
    records.into_iter().map(|r| r.payload).collect()
}

fn version(segment: &[u8]) -> u16 {
    u16::from_le_bytes([segment[4], segment[5]])
}

/// Segments 0 (rolled) and 1 (active, one group), both at version 3, with records `a` and `b`.
fn written_at_3(sim: &Sim) -> Vec<(u64, Vec<u8>)> {
    let log = open_at(sim, 3, EVERY_GROUP).unwrap();
    append(&log, b"a");
    append(&log, b"b");
    drop(log);
    let segments = raw::segments(sim);
    assert_eq!(segments.iter().map(|s| s.0).collect::<Vec<_>>(), [0, 1]);
    segments
}

/// A new log at version 3, with no record: one segment that holds only its header.
fn created_at_3(sim: &Sim) -> Vec<(u64, Vec<u8>)> {
    drop(open_at(sim, 3, EVERY_GROUP).unwrap());
    raw::segments(sim)
}

/// A rolled segment and a newest one that holds only its header, both at version 3.
fn header_only_at_3(sim: &Sim) -> Vec<(u64, Vec<u8>)> {
    let first = [
        raw::header(raw::VERSION, 0, 1),
        raw::group(1, 0, &[b"a", b"b"]),
    ]
    .concat();
    let segments = vec![
        (0, raw::rolled(first, 1)),
        (2, raw::header(raw::VERSION, 2, 1)),
    ];
    raw::install_segments(sim, &segments);
    segments
}

/// Given a log written at version 3: a rolled segment, and an active one with a group
/// When a build at version 4 opens it
/// Then the rolled segment keeps its bytes, the active one is closed by a roll's run in its own
/// version under the new writer epoch, and a new segment starts at version 4
/// And appends go to the new segment, a retry of a version 3 record returns its original position,
/// and every record reads back, also after a reopen
/// And a build at version 3 then refuses the log with `UnsupportedVersion(4)` and changes no file
///
/// Cases: FORMAT-MIXED
#[test]
fn a_log_written_at_one_version_is_appended_to_at_the_next() {
    bounded(|| {
        let sim = Sim::new(660);
        let before = written_at_3(&sim);
        let log = open_at(&sim, NEXT, Options::default().roll_bytes).unwrap();
        let after = raw::segments(&sim);
        assert_eq!(after[0], before[0], "a sealed segment is never rewritten");
        let closed = [before[1].1.clone(), raw::roll_run(&before[1].1, 2)].concat();
        assert_eq!(after[1], (1, closed), "closed in its own version");
        assert_eq!(version(&after[1].1), 3);
        let header = raw::with_log_id(raw::header(NEXT, 2, 2), raw::log_id(&sim));
        assert_eq!(after[2], (2, header), "the next segment is version 4");
        assert_eq!(after.len(), 3);

        append(&log, b"c");
        let retry = append(&log, b"a");
        assert!(retry.deduplicated);
        assert_eq!(retry.records[0].offset, 0);
        assert_eq!(payloads(&log), [b"a", b"b", b"c"]);
        drop(log);
        let segments = raw::segments(&sim);
        assert_eq!(
            segments.len(),
            3,
            "appends at version 4 go to the version 4 segment"
        );
        let log = open_at(&sim, NEXT, Options::default().roll_bytes).unwrap();
        assert_eq!(payloads(&log), [b"a", b"b", b"c"]);
        drop(log);
        oracle::sealed_unchanged(&before[..1], &raw::segments(&sim)[..1]);

        let files = raw::segments(&sim);
        let refused = open_at(&sim, 3, EVERY_GROUP);
        assert!(matches!(refused, Err(OpenError::UnsupportedVersion(4))));
        assert_eq!(raw::segments(&sim), files);

        // Read-only across both versions once sealed, and refused by a version 3 reader.
        open_at(&sim, NEXT, EVERY_GROUP).unwrap().seal().unwrap();
        let storage: Arc<dyn crate::storage::Storage> = Arc::new(sim.clone());
        let reader = LogReader::at_version(Path::new("/log"), storage.clone(), NEXT).unwrap();
        let records = reader.read(0, 100).unwrap();
        let read: Vec<_> = records.into_iter().map(|r| r.payload).collect();
        assert_eq!(read, [b"a", b"b", b"c"]);
        let refused = LogReader::at_version(Path::new("/log"), storage, 3);
        assert!(matches!(refused, Err(OpenError::UnsupportedVersion(4))));
    });
}

/// Given a log at version 3 whose newest segment holds only its header, after a rolled segment
/// When a build at version 4 opens it
/// Then the newest header is rewritten in place at version 4, with the same log id, base offset
/// and epoch, and the rolled segment keeps its bytes
/// And the next append goes into that segment
/// Given a new log at version 3 with no record
/// When a build at version 4 opens it
/// Then its only segment's header is rewritten in place at version 4
///
/// Cases: FORMAT-MIXED
#[test]
fn a_header_only_segment_of_the_older_version_is_rewritten_in_place() {
    bounded(|| {
        let sim = Sim::new(661);
        let before = header_only_at_3(&sim);
        let log = open_at(&sim, NEXT, Options::default().roll_bytes).unwrap();
        let after = raw::segments(&sim);
        assert_eq!(after[0], before[0]);
        assert_eq!(after[1], (2, raw::header(NEXT, 2, 1)));
        append(&log, b"c");
        assert_eq!(raw::segments(&sim).len(), 2);
        assert_eq!(payloads(&log), [b"a", b"b", b"c"]);

        let sim = Sim::new(662);
        drop(open_at(&sim, 3, EVERY_GROUP).unwrap());
        let created = raw::segments(&sim);
        assert_eq!(version(&created[0].1), 3);
        let log = open_at(&sim, NEXT, EVERY_GROUP).unwrap();
        let rewritten = raw::segments(&sim);
        assert_eq!(rewritten.len(), 1);
        assert_eq!(version(&rewritten[0].1), NEXT);
        assert_eq!(rewritten[0].1[8..60], created[0].1[8..60]);
        append(&log, b"a");
        assert_eq!(payloads(&log), [b"a"]);
    });
}

/// Given each version 3 log of the scenarios above
/// When a crash cuts the version 4 build's open and first append at each storage call, under
/// each crash mode, and a build at version 4 opens the log again
/// Then the open succeeds with every record acknowledged before it, and at most the new one
/// And every segment before the older active one keeps its bytes, an older active segment with
/// groups keeps them before any closing run, and the newest segment is at version 4
/// And under a kill or a power loss, which keep each sector old or new (V2), the log id never
/// changes; a torn sector write, which V2 rules out, may cost only the header-only segment
/// And a power loss after that open gives the same records
///
/// Cases: FORMAT-MIXED, ORACLE-1, RECOVERY-FIXTURES
#[test]
fn a_crash_at_each_step_of_the_version_change_keeps_every_record() {
    bounded(|| {
        type Setup = fn(&Sim) -> Vec<(u64, Vec<u8>)>;
        // A label, the setup, the segments before the older active one, whether that one holds
        // groups, and the records acknowledged before the version 4 open.
        type Case = (&'static str, Setup, usize, bool, &'static [&'static [u8]]);
        let setups: [Case; 3] = [
            ("close and roll", written_at_3, 1, true, &[b"a", b"b"]),
            ("header rewrite", header_only_at_3, 1, false, &[b"a", b"b"]),
            ("new log", created_at_3, 0, false, &[]),
        ];
        for (label, setup, sealed, groups, acked) in setups {
            let sim = Sim::new(663);
            let before = setup(&sim);
            let start = sim.trace().1.len();
            let log = open_at(&sim, NEXT, Options::default().roll_bytes).unwrap();
            append(&log, b"c");
            drop(log);
            let (seed, trace) = sim.trace();
            for cut in start..=trace.len() {
                for mode in MODES {
                    let replay = Sim::replay(seed, &trace[..cut]);
                    replay.crash(mode, cut as u64);
                    let log = open_at(&replay, NEXT, Options::default().roll_bytes);
                    let label = format!("{label}: cut {cut} {mode:?}");
                    let log = log.unwrap_or_else(|e| panic!("{label}: {e:?}"));
                    let kept = payloads(&log);
                    assert!(
                        kept.iter().zip(acked).all(|(k, a)| k == a) && kept.len() >= acked.len(),
                        "{label}: {kept:?}"
                    );
                    assert!(kept.len() <= acked.len() + 1, "{label}: {kept:?}");
                    let segments = raw::segments(&replay);
                    assert_eq!(segments[..sealed], before[..sealed], "{label}");
                    if groups {
                        let active = &before[sealed];
                        assert_eq!(segments[sealed].0, active.0, "{label}");
                        assert!(segments[sealed].1.starts_with(&active.1), "{label}");
                    }
                    assert_eq!(version(&segments.last().unwrap().1), NEXT, "{label}");
                    // A torn sector write, which V2 rules out, can leave a header-only newest
                    // segment unreadable; open then removes it, and a new log has none left.
                    if mode != Crash::PowerLossTorn {
                        let id = raw::header_fields(&before[0].1).0;
                        assert_eq!(raw::log_id(&replay), id, "{label}");
                    }
                    drop(log);
                    replay.crash(Crash::PowerLoss, cut as u64);
                    let log = open_at(&replay, NEXT, Options::default().roll_bytes).unwrap();
                    assert_eq!(payloads(&log), kept, "{label}");
                }
            }
        }
    });
}

/// Given a log whose version 4 segment is followed by a version 3 segment, which a writer never
/// produces (ADR 0064, rule 12), as after a hand copy
/// When a build at version 4 opens it, with a writer or with `LogReader`
/// Then open fails with `Corrupt` at the older segment's header and changes no file
///
/// Cases: FORMAT-MIXED
#[test]
fn an_older_version_after_a_newer_one_fails_open() {
    bounded(|| {
        let sim = Sim::new(664);
        let first = [raw::header(NEXT, 0, 1), raw::group(1, 0, &[b"a", b"b"])].concat();
        let segments = vec![
            (0, raw::rolled(first, 1)),
            (2, raw::header(raw::VERSION, 2, 1)),
        ];
        raw::install_segments(&sim, &segments);
        let corrupt = |result: Result<_, OpenError>| {
            matches!(
                result,
                Err(OpenError::Corrupt {
                    segment: 2,
                    position: 0
                })
            )
        };
        assert!(corrupt(open_at(&sim, NEXT, EVERY_GROUP).map(drop)));
        assert_eq!(raw::segments(&sim), segments);
        let storage: Arc<dyn crate::storage::Storage> = Arc::new(sim.clone());
        let reader = LogReader::at_version(Path::new("/log"), storage, NEXT);
        assert!(corrupt(reader.map(drop)));
    });
}
