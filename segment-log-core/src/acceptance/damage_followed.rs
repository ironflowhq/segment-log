use super::support::{bounded, fixtures::*, raw};
use crate::*;
use std::{path::Path, sync::Arc};

fn reopen(sim: &Sim) -> Result<SegmentLog, OpenError> {
    SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone()))
}

/// Given three acknowledged groups in an unsealed newest segment
/// When one byte inside the first group frame flips, and the later two frames stay valid
/// Then open fails with `Corrupt` at the first group frame and changes no file
///
/// Cases: OPEN-DAMAGE-FOLLOWED
#[test]
fn open_refuses_a_bad_frame_with_acknowledged_groups_after_it() {
    bounded(|| {
        let sim = Sim::new(178);
        let log = open(&sim);
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
