//! A rolled segment whose closing run is not durable yet (ADR 0061).
use super::*;
use crate::storage::{Call, Op};

/// Given a keyed record in the active segment, and a group flush that the storage holds
/// When a new submit rolls that segment, and a retry of its key and a conflicting reuse of it
/// queue behind, so that the writer classifies them before it writes the rolled segment's
/// closing run
/// Then the retry returns the original position with `deduplicated: true`, and the conflict gets
/// `KeyConflict`
/// And both found the key through the pending segment's map and its group frame, never through
/// a key index
///
/// Cases: KEY-LIFETIME, KEY-CONFLICT
#[test]
fn a_retry_of_a_key_whose_segment_waits_for_its_closing_run_matches() {
    bounded(|| {
        let sim = Sim::new(361);
        let log = open_hooked(&sim, &Hooks::default());
        commit(&log, vec![keyed(b"k0")]);
        let flush = sim.hold(Call::new(raw::segment(1), Purpose::Group, Op::SyncData));
        let held = log.append(Submit {
            records: vec![keyed(b"k1")],
        });
        flush.wait();
        let rolling = log.append(Submit {
            records: vec![keyed(b"k2")],
        });
        let retry = log.append(Submit {
            records: vec![keyed(b"k1")],
        });
        let mut reused = keyed(b"k1");
        reused.payload.push(1);
        let conflict = log.append(Submit {
            records: vec![reused],
        });
        flush.release(None);
        assert_eq!(offsets(&held.unwrap().wait().unwrap()), [1]);
        assert_eq!(offsets(&rolling.unwrap().wait().unwrap()), [2]);
        let retry = retry.unwrap().wait().unwrap();
        assert!(retry.deduplicated);
        assert_eq!(offsets(&retry), [1]);
        assert!(matches!(
            conflict.unwrap().wait(),
            Err(CommitError::KeyConflict { index: 0 })
        ));
        assert_eq!(raw::segments(&sim).len(), 3);
        assert!(reads(&sim, 1, Purpose::Lookup) >= 2);
        assert_eq!(reads(&sim, 1, Purpose::KeyIndex), 0);
    });
}

/// Given a log that rolls after every group and builds each closing run off the writer thread
/// When keyed submits and retries of earlier keys commit, so that rolls overtake closing runs
/// that are still being built, and the log is dropped
/// Then every retry returns its original position
/// And every segment but the newest ends in the closing run that its groups produce
/// And after a reopen, a retry of every key still matches
///
/// Cases: ROLL-BOUNDED, KEY-LIFETIME, ORACLE-6
#[test]
fn closing_runs_built_off_the_writer_seal_every_segment() {
    bounded(|| {
        let sim = Sim::new(362);
        let hooks = Hooks {
            background_seal: true,
            ..Hooks::default()
        };
        let keys: Vec<Vec<u8>> = (0..40u8).map(|n| vec![b'k', n]).collect();
        let log = open_hooked(&sim, &hooks);
        for (n, key) in keys.iter().enumerate() {
            assert_eq!(offsets(&commit(&log, vec![keyed(key)])), [n as u64]);
            if n % 3 == 2 {
                assert_retry(
                    &log,
                    &[&keys[n - 1], &keys[n / 3]],
                    &[n as u64 - 1, n as u64 / 3],
                );
            }
        }
        drop(log);
        let segments = raw::segments(&sim);
        assert_eq!(segments.len(), keys.len());
        for (base, bytes) in &segments[..segments.len() - 1] {
            let (groups, run) = raw::split_run(bytes);
            assert_eq!(run, raw::roll_run(groups, 1), "segment {base}");
        }
        let log = open_hooked(&sim, &hooks);
        for (n, key) in keys.iter().enumerate() {
            assert_retry(&log, &[key], &[n as u64]);
        }
    });
}

/// Given a log whose second-newest segment holds only groups, because a crash came before the
/// writer wrote its closing run, and whose newest segment holds only its header
/// When a `LogReader` opens it
/// Then it fails with `NotSealed`, not `Corrupt`, and changes no byte
/// When a writable open follows
/// Then it completes the older segment and reads its records
///
/// Cases: READER, RECOVERY-FIXTURES
#[test]
fn a_reader_finds_a_log_with_a_pending_closing_run_not_sealed() {
    bounded(|| {
        let sim = Sim::new(363);
        let groups = [
            raw::header(raw::VERSION, 0, 1),
            raw::group(1, 0, &[b"kept"]),
        ]
        .concat();
        let newest = (1, raw::header(raw::VERSION, 1, 1));
        raw::install_segments(&sim, &[(0, groups.clone()), newest.clone()]);
        let before = sim.image();
        assert!(matches!(
            LogReader::with_storage(Path::new("/log"), Arc::new(sim.clone())),
            Err(OpenError::NotSealed)
        ));
        assert_eq!(sim.image(), before);
        let log = open_hooked(&sim, &Hooks::default());
        assert_eq!(log.read(0, 10).unwrap()[0].payload, b"kept");
        assert_eq!(raw::segments(&sim), [(0, raw::rolled(groups, 1)), newest]);
    });
}
