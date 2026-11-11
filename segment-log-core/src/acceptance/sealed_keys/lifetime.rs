use super::*;
use crate::storage::{
    Call, Op,
    sim::{Crash, Event},
};

fn unkeyed(stream: &[u8]) -> NewRecord {
    NewRecord {
        stream: stream.to_vec(),
        key: None,
        payload: b"plain".to_vec(),
        expected: ExpectedRevision::Empty,
    }
}

/// Given keyed records that rolls left in sealed segments, with a sealed segment between them that
/// holds no keyed record
/// When a caller retries them, alone and regrouped across segments, before and after a reopen
/// Then each retry returns the original positions with `deduplicated: true` and appends nothing
/// And each retry of a sealed key reads a key index page of the segment that holds it, and no
/// retry reads one of the segment that holds no keyed record
/// And the same key with another payload gets `KeyConflict`
/// And the reopen reads the key filter of every sealed segment
///
/// Cases: KEY-LIFETIME, RETRY-REGROUPED, KEY-CONFLICT, ORACLE-12
#[test]
fn a_retry_finds_a_sealed_key_through_its_segments_filter_and_index() {
    bounded(|| {
        let sim = Sim::new(340);
        let hooks = Hooks::default();
        let mut log = open_hooked(&sim, &hooks);
        commit(&log, vec![keyed(b"k0")]);
        commit(&log, vec![unkeyed(b"plain")]);
        commit(&log, vec![keyed(b"k2"), keyed(b"k3")]);
        commit(&log, vec![keyed(b"k4")]);
        let bases: Vec<_> = raw::segments(&sim).iter().map(|(base, _)| *base).collect();
        assert_eq!(bases, [0, 1, 2, 4]);
        for reopened in [false, true] {
            if reopened {
                drop(log);
                log = open_hooked(&sim, &hooks);
                for base in [0, 1, 2] {
                    assert!(
                        reads(&sim, base, Purpose::KeyFilter) > 0,
                        "open reads the key filter of segment {base}"
                    );
                }
            }
            let before = (
                reads(&sim, 0, Purpose::KeyIndex),
                reads(&sim, 2, Purpose::KeyIndex),
            );
            assert_retry(&log, &[b"k0"], &[0]);
            assert!(reads(&sim, 0, Purpose::KeyIndex) > before.0);
            assert_retry(&log, &[b"k3", b"k0", b"k2"], &[3, 0, 2]);
            assert!(reads(&sim, 2, Purpose::KeyIndex) > before.1);
            assert_retry(&log, &[b"k4", b"k2"], &[4, 2]);
            let mut conflict = keyed(b"k2");
            conflict.payload.push(1);
            assert!(matches!(
                log.append(Submit {
                    records: vec![conflict]
                })
                .unwrap()
                .wait(),
                Err(CommitError::KeyConflict { index: 0 })
            ));
            assert_eq!(log.watermark(), 5);
            assert_eq!(reads(&sim, 1, Purpose::KeyIndex), 0);
        }
    });
}

/// Given a sealed segment and an active segment at the roll size, both with keyed records
/// When a new submit starts a roll, and a crash in each mode comes after the roll flushed its
/// closing run and before the next segment exists
/// Then the submit gets `Poisoned`
/// When the log reopens
/// Then open creates the next segment, and a retry of each key in the segment that open found
/// sealed returns its original position through that segment's key index
/// And so does a retry after another reopen and a power loss
///
/// Cases: KEY-FRAMES-CRASH, KEY-LIFETIME, ORACLE-1, ORACLE-4, ORACLE-5, ORACLE-6
#[test]
fn a_key_in_a_segment_that_open_seals_after_a_crash_still_matches() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            let sim = Sim::new(341);
            let hooks = Hooks::default();
            let log = open_hooked(&sim, &hooks);
            commit(&log, vec![keyed(b"k0")]);
            commit(&log, vec![keyed(b"k1"), keyed(b"k2")]);
            let create = sim.hold(Call::new(raw::segment(3), Purpose::Roll, Op::Create));
            let rolling = log.append(Submit {
                records: vec![keyed(b"k3")],
            });
            create.wait();
            sim.crash(mode, 341);
            create.release(None);
            assert!(matches!(
                rolling.unwrap().wait(),
                Err(CommitError::Poisoned)
            ));
            drop(log);
            let trace = sim.trace().1;
            assert!(
                trace.iter().any(|event| matches!(event,
                    Event::Call(call, _, _) if call.purpose == Purpose::Roll
                        && call.op == Op::SyncData
                        && call.path == Path::new(&raw::segment(1)))),
                "the roll flushed its closing run before the crash"
            );

            for crash in [None, Some(Crash::PowerLoss)] {
                if let Some(crash) = crash {
                    sim.crash(crash, 342);
                }
                let log = open_hooked(&sim, &hooks);
                let bases: Vec<_> = raw::segments(&sim).iter().map(|(base, _)| *base).collect();
                assert_eq!(bases, [0, 1, 3], "{mode:?} {crash:?}");
                let before = reads(&sim, 1, Purpose::KeyIndex);
                assert_retry(&log, &[b"k2", b"k0", b"k1"], &[2, 0, 1]);
                assert!(reads(&sim, 1, Purpose::KeyIndex) > before, "{mode:?}");
            }
        }
    });
}
