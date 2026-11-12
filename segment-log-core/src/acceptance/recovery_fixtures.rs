use super::support::{bounded, fixtures::*, raw};
use crate::storage::{Call, Op, Purpose, Storage};
use crate::*;
use std::{path::Path, sync::Arc};

/// Given independently assembled valid groups followed by an invalid suffix and another valid group
/// When recovery encounters zeros, length past EOF, bad CRC, lower epoch or wrong base
/// Then only the contiguous prefix is published and the retained file is exactly that prefix
/// Cases: RECOVERY-FIXTURES, ORACLE-2, ORACLE-3, ORACLE-5
#[test]
fn handmade_invalid_tails_stop_at_the_first_bad_frame() {
    bounded(|| {
        let prefix = [
            raw::header(raw::VERSION, 0, 1),
            raw::group(2, 0, &[b"first", b"second"]),
        ]
        .concat();
        let mut bad_crc = raw::group(2, 2, &[b"bad"]);
        bad_crc[4] ^= 1;
        let mut past_eof = vec![0; 8];
        past_eof[..4].copy_from_slice(&10000u32.to_le_bytes());
        for tail in [
            vec![0; 600],
            past_eof,
            bad_crc,
            raw::group(1, 2, &[b"old"]),
            raw::group(2, 3, &[b"gap"]),
        ] {
            let sim = Sim::new(201);
            raw::install(
                &sim,
                [prefix.clone(), tail, raw::group(3, 2, &[b"must not skip"])].concat(),
            );
            let log = open(&sim);
            let records = log.read(0, 10).unwrap();
            assert_eq!(log.watermark(), 2);
            assert_eq!(
                records
                    .iter()
                    .map(|r| r.payload.as_slice())
                    .collect::<Vec<_>>(),
                [b"first".as_slice(), b"second".as_slice()]
            );
            assert_eq!(
                records
                    .iter()
                    .map(|r| (r.offset, r.revision))
                    .collect::<Vec<_>>(),
                [(0, 0), (1, 1)]
            );
            assert_eq!(raw::bytes(&sim), prefix);
            drop(log);
            sim.crash(crate::storage::sim::Crash::PowerLoss, 1);
            assert_eq!(open(&sim).watermark(), 2);
        }
    });
}

/// Given valid groups followed by a frame of a closing kind, an unknown kind, or another log's
/// group, each with a checksum that is right for its writer, and then a valid group
/// When recovery scans the active segment
/// Then the frame ends the valid prefix, because this scan reads only group frames
/// And recovery truncates there and keeps the records before it
/// Cases: RECOVERY-FIXTURES, ORACLE-2, ORACLE-5
#[test]
fn frames_that_are_not_groups_of_this_log_end_the_valid_prefix() {
    bounded(|| {
        let prefix = [
            raw::header(raw::VERSION, 0, 1),
            raw::group(2, 0, &[b"first", b"second"]),
        ]
        .concat();
        let header = |kind: u8, count: u32| {
            let mut body = vec![kind, 0, 0, 0];
            body.extend(2u64.to_le_bytes());
            body.extend(2u64.to_le_bytes());
            body.extend(count.to_le_bytes());
            body
        };
        let mut locator = header(4, 0);
        locator.extend(64u64.to_le_bytes());
        let mut footer = header(2, 0);
        footer.extend(0u64.to_le_bytes());
        let id = raw::LOG_ID;
        for tail in [
            raw::frame(&id, &header(3, 0)),
            raw::frame(&id, &locator),
            raw::frame(&id, &footer),
            raw::frame(&id, &header(5, 0)),
            raw::frame(&id, &header(6, 0)),
            raw::frame(&id, &header(7, 0)),
            raw::group_in(&[9; 16], 2, 2, &[b"another log"]),
        ] {
            let sim = Sim::new(205);
            raw::install(
                &sim,
                [prefix.clone(), tail, raw::group(3, 2, &[b"must not skip"])].concat(),
            );
            let log = open(&sim);
            assert_eq!(log.watermark(), 2);
            assert_eq!(raw::bytes(&sim), prefix);
        }
    });
}

/// Given a segment longer than 64 bytes with a damaged header, or a valid header with a mismatched filename
/// When open validates the header
/// Then it refuses without deleting or changing any bytes
/// Cases: RECOVERY-FIXTURES
#[test]
fn invalid_complete_headers_are_never_repaired() {
    bounded(|| {
        let mut bad_crc = raw::header(raw::VERSION, 0, 1);
        bad_crc[60] ^= 1;
        let mut bad_magic = raw::header(raw::VERSION, 0, 1);
        bad_magic[0] ^= 1;
        for (header, extra, version) in [
            (bad_crc, 17, None),
            (bad_magic, 17, None),
            (raw::header(raw::VERSION, 1, 1), 17, None),
            (raw::header(0, 0, 1), 0, Some(0)),
            (raw::header(0, 0, 1), 17, Some(0)),
            (
                raw::header(raw::VERSION + 1, 0, 1),
                0,
                Some(raw::VERSION + 1),
            ),
        ] {
            let sim = Sim::new(202);
            raw::install(&sim, [header.clone(), vec![0; extra]].concat());
            let image = sim.image();
            let result =
                SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone()));
            match version {
                Some(version) => assert!(
                    matches!(result, Err(OpenError::UnsupportedVersion(found)) if found == version)
                ),
                None => assert!(matches!(result, Err(OpenError::Corrupt { .. }))),
            }
            assert_eq!(sim.image(), image);
        }
    });
}

/// Given an interrupted creation with a short header or a 64 byte header whose CRC is bad
/// When open cleans up the incomplete creation
/// Then a fresh empty log can acknowledge a record that survives power loss
/// Cases: RECOVERY-FIXTURES, OPEN-DIR, ORACLE-1
#[test]
fn incomplete_headers_can_be_recreated() {
    bounded(|| {
        for len in [0, 1, 63, 64] {
            let sim = Sim::new(203);
            raw::install(&sim, vec![0; len]);
            let log = open(&sim);
            assert_eq!(log.watermark(), 0);
            append(&log, b"s").wait().unwrap();
            drop(log);
            sim.crash(crate::storage::sim::Crash::PowerLoss, 5);
            assert_record(&open(&sim));
        }
    });
}

/// Given one readable multi-record group spanning several pages after a failed data flush
/// When recovery publishes them and power is lost immediately
/// Then every kept record remains readable with its original offset and bytes
/// Cases: ORACLE-5, RECOVERY-FIXTURES
#[test]
fn recovery_rewrites_a_readable_group_after_failed_writeback() {
    bounded(|| {
        let sim = Sim::new(204);
        drop(open(&sim));
        let payload = vec![0xa5; 5000];
        let suffix = raw::group_in(&raw::log_id(&sim), 1, 0, &[&payload, b"last"]);
        sim.call(Call::new(SEG, Purpose::Group, Op::Append(suffix)))
            .unwrap();
        sim.inject(
            Call::new(SEG, Purpose::Group, Op::SyncData),
            crate::storage::sim::Fault::NoSpace,
        );
        assert!(
            sim.call(Call::new(SEG, Purpose::Group, Op::SyncData))
                .is_err()
        );
        let log = open(&sim);
        assert_eq!(log.watermark(), 2);
        drop(log);
        sim.crash(crate::storage::sim::Crash::PowerLoss, 0);
        let log = open(&sim);
        assert_eq!(
            log.watermark(),
            2,
            "all readable records must be made durable before publication"
        );
        let records = log.read(0, 10).unwrap();
        assert_eq!(records[0].payload, payload);
        assert_eq!(records[1].payload, b"last");
    });
}
