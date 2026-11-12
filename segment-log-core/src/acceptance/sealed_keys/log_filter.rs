use super::*;
use crate::{
    engine::LogFilterMode,
    storage::{Call, Op, sim::Fault},
};
use std::{collections::BTreeMap, io};

/// Segment sizes that triple, so that the log key filter outgrows its room and is rebuilt more
/// than once while the history is written.
const SEGMENT_KEYS: [usize; 6] = [1, 3, 9, 27, 81, 243];

#[derive(Debug, PartialEq)]
enum Answer {
    Ack(bool, Vec<u64>),
    Refused(String),
}

fn answer(log: &SegmentLog, records: Vec<NewRecord>) -> Answer {
    match log.append(Submit { records }).unwrap().wait() {
        Ok(committed) => Answer::Ack(committed.deduplicated, offsets(&committed)),
        Err(error) => Answer::Refused(format!("{error:?}")),
    }
}

fn key(segment: usize, i: usize) -> Vec<u8> {
    format!("old-{segment}-{i}").into_bytes()
}

fn hooks(mode: LogFilterMode, key_hashes: BTreeMap<Vec<u8>, u64>) -> Hooks {
    Hooks {
        key_hashes,
        log_filter: mode,
        ..Hooks::default()
    }
}

fn unkeyed(log: &SegmentLog) -> NewRecord {
    NewRecord {
        stream: b"plain".to_vec(),
        key: None,
        payload: vec![],
        expected: log
            .stream_head(b"plain")
            .map_or(ExpectedRevision::Empty, |h| {
                ExpectedRevision::At(h.revision)
            }),
    }
}

/// Writes one sealed segment of keyed records for each entry of `sizes`. A last unkeyed record
/// rolls the last of them, so every keyed record ends in a sealed segment.
fn write_history(sim: &Sim, hooks: &Hooks, sizes: &[usize]) -> SegmentLog {
    let log = open_hooked(sim, hooks);
    for (segment, &n) in sizes.iter().enumerate() {
        commit(&log, (0..n).map(|i| keyed(&key(segment, i))).collect());
    }
    commit(&log, vec![unkeyed(&log)]);
    log
}

/// Where the builder's first read of a sealed segment's key index lands, as the builder makes it.
fn first_index_read(segment: (u64, &[u8])) -> Call {
    let (base, bytes) = segment;
    let (groups, run) = raw::split_run(bytes);
    let filter = u32::from_le_bytes(run[..4].try_into().unwrap()) as usize + 8;
    let pages_at = groups.len() + filter + 8 + 24;
    Call::new(
        raw::segment(base),
        Purpose::LogFilter,
        Op::Read(pages_at as u64, 4096),
    )
}

/// Given a history whose sealed segments hold 1, 3, 9, 27, 81 and 243 keyed records, so that the
/// log key filter is rebuilt larger as it grows
/// And new keys that share a hash with keys in old segments
/// When the same submits go to that log with the log key filter built inline, built on its own
/// thread, and turned off, before and after a reopen
/// Then every answer is the same: retries of keys in every segment return their original
/// positions, a changed payload is a conflict, and a new key with a shared hash appends
/// And with the filter, it covers every sealed segment with at least 8 bits for each sealed key
///
/// Cases: LOG-FILTER-ANSWERS, LOG-FILTER-GROWTH, KEY-COLLISION
#[test]
fn the_log_key_filter_never_changes_an_answer() {
    bounded(|| {
        let shared: BTreeMap<Vec<u8>, u64> = SEGMENT_KEYS
            .iter()
            .enumerate()
            .flat_map(|(segment, &n)| {
                let hash = 0x5eed_0000_0000_0000 | segment as u64;
                [
                    (key(segment, n - 1), hash),
                    (format!("new-{segment}").into_bytes(), hash),
                ]
            })
            .collect();
        let probes = |log: &SegmentLog| -> Vec<Answer> {
            let mut answers = Vec::new();
            for (segment, &n) in SEGMENT_KEYS.iter().enumerate() {
                answers.push(answer(log, vec![keyed(&key(segment, 0))]));
                answers.push(answer(log, vec![keyed(&key(segment, n - 1))]));
                let mut changed = keyed(&key(segment, n / 2));
                changed.payload.push(b'!');
                answers.push(answer(log, vec![changed]));
                answers.push(answer(
                    log,
                    vec![keyed(format!("new-{segment}").as_bytes())],
                ));
            }
            answers.push(answer(log, vec![keyed(b"fresh")]));
            answers
        };
        let sealed_keys = SEGMENT_KEYS.iter().sum::<usize>() as u64;
        let run = |mode| {
            let sim = Sim::new(380);
            let hooks = hooks(mode, shared.clone());
            let log = write_history(&sim, &hooks, &SEGMENT_KEYS);
            let memory = log.key_memory();
            assert_eq!(memory.sealed_keys, sealed_keys, "{mode:?}");
            if mode == LogFilterMode::Inline {
                assert_eq!(memory.log_filter_covered, SEGMENT_KEYS.len());
                assert!(memory.log_filter_bytes as u64 >= sealed_keys);
            }
            let first = probes(&log);
            drop(log);
            let log = open_hooked(&sim, &hooks);
            let memory = log.key_memory();
            if mode == LogFilterMode::Inline {
                assert!(memory.log_filter_covered > SEGMENT_KEYS.len());
                assert!(memory.log_filter_bytes as u64 >= memory.sealed_keys);
            }
            (first, probes(&log))
        };
        let off = run(LogFilterMode::Off);
        assert!(off.0.contains(&Answer::Ack(true, vec![0])));
        assert!(
            off.0
                .contains(&Answer::Refused("KeyConflict { index: 0 }".into()))
        );
        assert!(off.0.iter().any(|a| matches!(a, Answer::Ack(false, _))));
        assert_eq!(run(LogFilterMode::Inline), off, "inline build");
        assert_eq!(run(LogFilterMode::Background), off, "background build");
    });
}

/// Given four sealed segments
/// When the log reopens and the log key filter's build fails on segment 1, with an I/O error or
/// with a damaged key index page
/// Then the writer is not poisoned
/// And the filter covers only segment 0, the segments before the failure
/// And retries of keys in every segment the writer can read return their original positions,
/// and a new key appends
///
/// Cases: LOG-FILTER-FAILS
#[test]
fn a_failed_build_costs_speed_and_never_an_answer() {
    let faults = [
        Fault::error(io::ErrorKind::Other).unwrap(),
        Fault::Corrupt(100, 0x01),
    ];
    for fault in faults {
        bounded(move || {
            let sim = Sim::new(390);
            let hooks = hooks(LogFilterMode::Inline, BTreeMap::new());
            drop(write_history(&sim, &hooks, &[3, 3, 3, 3]));
            let segments = raw::segments(&sim);
            let (base, bytes) = &segments[1];
            sim.inject(first_index_read((*base, bytes)), fault.clone());
            let log = open_hooked(&sim, &hooks);
            assert_eq!(log.key_memory().log_filter_covered, 1, "{fault:?}");
            let watermark = log.watermark();
            for segment in 0..4 {
                assert_retry(&log, &[&key(segment, 0)], &[3 * segment as u64]);
            }
            assert_eq!(
                answer(&log, vec![keyed(b"fresh")]),
                Answer::Ack(false, vec![watermark]),
                "{fault:?}"
            );
        });
    }
}

/// Given four sealed segments
/// When the log reopens with the log key filter built on its own thread
/// Then appends go on while it builds, and the filter comes to cover all four segments
/// And a retry of a key in each segment returns its original position
/// And seal stops the builder and returns
///
/// Cases: LOG-FILTER-BACKGROUND
#[test]
fn the_background_build_comes_to_cover_every_sealed_segment() {
    bounded(|| {
        let sim = Sim::new(395);
        let hooks = hooks(LogFilterMode::Background, BTreeMap::new());
        drop(write_history(&sim, &hooks, &[3, 3, 3, 3]));
        let log =
            SegmentLog::with_hooks(Path::new("/log"), options(), Arc::new(sim.clone()), hooks)
                .unwrap();
        let mut appends = 0;
        while log.key_memory().log_filter_covered < 4 {
            commit(&log, vec![unkeyed(&log)]);
            appends += 1;
            std::thread::yield_now();
        }
        for segment in 0..4 {
            assert_retry(&log, &[&key(segment, 0)], &[3 * segment as u64]);
        }
        println!("LOG-FILTER-BACKGROUND: covered after {appends} unkeyed appends");
        log.seal().unwrap();
    });
}
