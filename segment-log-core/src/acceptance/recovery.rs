use super::support::{bounded, fixtures::*};
use crate::storage;
use crate::storage::{Call, Op, Purpose};
use crate::*;
use std::{path::Path, sync::Arc};

/// Given an acknowledged record on simulated storage
/// When Kill, PowerLoss or PowerLossTorn interrupts the process and it reopens
/// Then the full record and original position survive, including another power loss after open
/// Cases: ORACLE-1, ORACLE-5, ORACLE-9
#[test]
fn acknowledged_record_survives_each_crash_mode() {
    bounded(|| {
        for mode in [
            storage::sim::Crash::Kill,
            storage::sim::Crash::PowerLoss,
            storage::sim::Crash::PowerLossTorn,
        ] {
            let sim = Sim::new(3);
            let log = open(&sim);
            append(&log, b"s").wait().unwrap();
            sim.crash(mode, 9);
            drop(log);
            let reopened = open(&sim);
            assert_record(&reopened);
            sim.crash(storage::sim::Crash::PowerLoss, 10);
            drop(reopened);
            assert_record(&open(&sim));
        }
    });
}

/// Given a group whose write completed but whose flush has not returned
/// When each simulated crash mode interrupts the held flush
/// Then the original caller gets no success and the group was never published
/// And reopen exposes either the complete record or nothing, with Kill retaining it
/// Cases: ORACLE-2, ORACLE-8, ORACLE-9
#[test]
fn crash_during_a_flush_has_an_unknown_but_atomic_outcome() {
    bounded(|| {
        for mode in [
            storage::sim::Crash::Kill,
            storage::sim::Crash::PowerLoss,
            storage::sim::Crash::PowerLossTorn,
        ] {
            for seed in 0..8 {
                let sim = Sim::new(12);
                let log = open(&sim);
                let hold = sim.hold(Call::new(SEG, Purpose::Group, Op::SyncData));
                let pending = append(&log, b"s");
                hold.wait();
                assert!(pending.probe().is_none());
                assert_eq!(log.watermark(), 0);
                assert!(log.read(0, 100).unwrap().is_empty());
                assert!(log.stream_head(b"s").is_none());
                sim.crash(mode, seed);
                hold.release(None);
                assert!(matches!(pending.wait(), Err(CommitError::Indeterminate)));
                drop(log);
                let reopened = open(&sim);
                if mode == storage::sim::Crash::Kill || reopened.watermark() != 0 {
                    assert_record(&reopened);
                } else {
                    assert!(reopened.read(0, 100).unwrap().is_empty());
                }
            }
        }
    });
}

/// Given traces of creation and recovery of an acknowledged record
/// When a crash interrupts each storage boundary under each simulated crash mode
/// Then creation can reopen empty and recovery preserves the acknowledged record
/// Cases: OPEN-DIR, ORACLE-1, ORACLE-5
#[test]
fn every_creation_and_recovery_boundary_can_crash() {
    bounded(|| {
        let sim = Sim::new(13);
        let log = open(&sim);
        let (seed, creation) = sim.trace();
        append(&log, b"s").wait().unwrap();
        drop(log);
        let before = sim.trace().1.len();
        let reopened = open(&sim);
        let (_, recovery) = sim.trace();
        drop(reopened);
        for (trace, start, kept) in [(&creation, 0, false), (&recovery, before, true)] {
            for end in start..=trace.len() {
                for mode in [
                    storage::sim::Crash::Kill,
                    storage::sim::Crash::PowerLoss,
                    storage::sim::Crash::PowerLossTorn,
                ] {
                    let replay = Sim::replay(seed, &trace[..end]);
                    replay.crash(mode, end as u64);
                    let log = open(&replay);
                    if kept {
                        assert_record(&log);
                    } else {
                        assert_eq!(log.watermark(), 0);
                    }
                }
            }
        }
    });
}

/// Given a valid header with an unsupported format version
/// When the log is opened
/// Then it returns UnsupportedVersion and leaves the segment unchanged
/// Cases: RECOVERY-FIXTURES
#[test]
fn unsupported_header_version_is_reported_without_repair() {
    bounded(|| {
        use crate::storage::Storage;
        let sim = Sim::new(14);
        drop(open(&sim));
        let mut bytes = crate::codec::encode_header(&crate::codec::Header {
            log_id: [7; 16],
            base_offset: 0,
            epoch: 1,
        });
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        let crc = crate::codec::crc32c(&bytes[..60]);
        bytes[60..].copy_from_slice(&crc.to_le_bytes());
        sim.call(Call::new(
            SEG,
            Purpose::Recovery,
            Op::Write(0, bytes.to_vec()),
        ))
        .unwrap();
        let image = sim.image();
        assert!(matches!(
            SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone())),
            Err(OpenError::UnsupportedVersion(1))
        ));
        assert_eq!(sim.image(), image);
    });
}
