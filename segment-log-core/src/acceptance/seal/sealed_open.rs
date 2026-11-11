use super::*;
use crate::{codec, storage::sim::Crash};

fn groups() -> Vec<u8> {
    [
        raw::header(raw::VERSION, 0, 1),
        raw::group(1, 0, &[b"a"]),
        raw::group(1, 1, &[b"b", b"c"]),
    ]
    .concat()
}

fn kept() -> [Record; 3] {
    [
        stored(b"s", None, b"a", 0, 0),
        stored(b"s", None, b"b", 1, 1),
        stored(b"s", None, b"c", 2, 2),
    ]
}

/// The end of each frame of a closing run, relative to the run's start.
fn frame_ends(run: &[u8]) -> Vec<usize> {
    let mut end = 0;
    raw::frames(&[vec![0; 64], run.to_vec()].concat())
        .into_iter()
        .map(|frame| {
            end += frame.len();
            end
        })
        .collect()
}

fn seal_run_at(log_id: &codec::LogId, footer_at: usize, next: u64) -> Vec<u8> {
    let footer = codec::encode_footer(
        log_id,
        1,
        next,
        &codec::Footer {
            key_filter_position: None,
            streams: vec![codec::StreamTail {
                stream: b"s",
                last_revision: 0,
                last_offset: next - 1,
                last_position: 64,
            }],
        },
    )
    .unwrap();
    let seal = codec::encode_seal(log_id, 1, next);
    let locator = codec::encode_locator(log_id, 1, next, footer_at as u64);
    [footer, seal, locator].concat()
}

/// Opens a log whose only segment is `bytes`, and checks that open cut it back to `groups`, that
/// the log stays writable in that segment, and that a power loss after open changes nothing.
fn assert_cut_to(bytes: Vec<u8>, groups: &[u8], label: &str) {
    let sim = Sim::new(811);
    raw::install_segments(&sim, &[(0, bytes)]);
    let log = open_with(&sim, unrolled());
    assert_eq!(raw::segments(&sim), [(0, groups.to_vec())], "{label}");
    assert_records(&log.read(0, 10).unwrap(), &kept());
    drop(log);
    sim.crash(Crash::PowerLoss, 811);
    let log = open_with(&sim, unrolled());
    assert_eq!(raw::segments(&sim), [(0, groups.to_vec())], "{label}");
    let answer = send(&log, vec![record(b"s", None, b"d", Some(2))])
        .wait()
        .unwrap();
    assert_eq!(answer.records[0].offset, 3, "{label}");
    let segments = raw::segments(&sim);
    assert_eq!(segments.len(), 1, "{label}: the segment stays active");
    assert!(segments[0].1.starts_with(groups), "{label}");
}

/// Given a log whose older segment ends in a roll's closing run, and whose newest segment ends in a
/// seal's closing run under a later epoch than its groups
/// When the log opens
/// Then it reads every record, with the watermark and the stream head of the scan
/// And `append` gets `Sealed` for a valid submit, after `Invalid` and `TooLarge` for a malformed
/// and an oversized one
/// And open read no key filter, and released the lock, so a second open of the same directory
/// succeeds while the first handle lives, and is read-only too
/// And no segment file changes
/// When the newest segment ends in its groups instead
/// Then open reads the older segment's key filter, and the next append takes the next offset
///
/// Cases: ADM-PRECEDENCE, ORACLE-13, ORACLE-6, HISTORY
#[test]
fn a_log_that_ends_in_a_seal_run_opens_read_only() {
    bounded(|| {
        let older = raw::rolled(
            [raw::header(raw::VERSION, 0, 1), raw::group(1, 0, &[b"a"])].concat(),
            1,
        );
        let newest = [
            raw::header(raw::VERSION, 1, 1),
            raw::group(1, 1, &[b"b", b"c"]),
        ]
        .concat();
        let sealed = [newest.clone(), raw::seal_run(&newest, 2)].concat();
        let sim = Sim::new(810);
        raw::install_segments(&sim, &[(0, older.clone()), (1, sealed)]);
        let image = raw::segments(&sim);
        let filters = key_filter_reads(&sim);
        let log = open_with(&sim, unrolled());
        assert_eq!(
            key_filter_reads(&sim),
            filters,
            "a sealed log reads no key filter"
        );
        assert_records(&log.read(0, 10).unwrap(), &kept());
        assert_eq!(log.watermark(), 3);
        let head = log.stream_head(b"s").map(|h| (h.offset, h.revision));
        assert_eq!(head, Some((2, 2)));
        assert_sealed(&log);
        let second = open_with(&sim, unrolled());
        assert_sealed(&second);
        assert_records(&second.read(0, 10).unwrap(), &kept());
        drop((log, second));
        assert_eq!(raw::segments(&sim), image);

        let sim = Sim::new(810);
        raw::install_segments(&sim, &[(0, older), (1, newest)]);
        let filters = key_filter_reads(&sim);
        let log = open_with(&sim, unrolled());
        assert!(
            key_filter_reads(&sim) > filters,
            "a writable log loads its key filters"
        );
        let answer = send(&log, vec![record(b"s", None, b"d", Some(2))])
            .wait()
            .unwrap();
        assert_eq!(answer.records[0].offset, 3);
    });
}

/// Given a segment of groups followed by the footer, or the footer and the seal frame, of a seal's
/// closing run, with no locator after them
/// When the log opens
/// Then open cuts the segment back to its groups, because the flush that covered those frames
/// never returned
/// And the log stays writable in that segment, and a power loss after open gives the same log
///
/// Cases: RECOVERY-FIXTURES, ORACLE-5
#[test]
fn seal_frames_without_a_locator_are_truncated() {
    bounded(|| {
        let groups = groups();
        let run = raw::seal_run(&groups, 1);
        let ends = frame_ends(&run);
        assert_eq!(ends.len(), 3);
        for (kept, end) in ends[..2].iter().enumerate() {
            let bytes = [groups.clone(), run[..*end].to_vec()].concat();
            assert_cut_to(bytes, &groups, &format!("{} closing frames", kept + 1));
        }
    });
}

/// Given a segment whose last record's payload ends with a complete seal's closing run under the
/// log's own id, whose locator names the footer inside that payload
/// When the log opens
/// Then the segment stays active, because the forward scan steps over the whole group frame
/// And the record is readable with its payload, and the next append lands after it in that segment
///
/// Cases: RECOVERY-FIXTURES
#[test]
fn a_payload_that_ends_in_a_seal_run_leaves_the_segment_active() {
    bounded(|| {
        let log_id = codec::LogId::new(raw::LOG_ID);
        let prefix = vec![b'p'; 100];
        let payload_at = 64 + 8 + 24 + 16 + b"s".len();
        let run = seal_run_at(&log_id, payload_at + prefix.len(), 1);
        let payload = [prefix, run].concat();
        let segment = [
            raw::header(raw::VERSION, 0, 1),
            raw::group(1, 0, &[&payload]),
        ]
        .concat();
        let tail = &segment[segment.len() - codec::LOCATOR_LEN..];
        assert!(
            matches!(
                codec::decode(&log_id, tail).unwrap().body,
                codec::Body::Locator { .. }
            ),
            "the last bytes are a valid locator"
        );

        let sim = Sim::new(812);
        raw::install_segments(&sim, &[(0, segment.clone())]);
        let log = open_with(&sim, unrolled());
        assert_records(
            &log.read(0, 10).unwrap(),
            &[stored(b"s", None, &payload, 0, 0)],
        );
        let answer = send(&log, vec![record(b"s", None, b"d", Some(0))])
            .wait()
            .unwrap();
        assert_eq!(answer.records[0].offset, 1);
        let segments = raw::segments(&sim);
        assert_eq!(segments.len(), 1);
        assert!(segments[0].1.starts_with(&segment));
    });
}

/// Given a segment of groups and a complete seal's closing run, where a power loss kept the footer
/// and the locator but lost the seal frame, or kept the locator but lost the footer
/// When the log opens
/// Then open cuts the closing run and keeps the segment active, and does not fail with `Corrupt`
/// And the log stays writable in that segment, and a power loss after open gives the same log
///
/// Cases: RECOVERY-FIXTURES, ORACLE-5
#[test]
fn a_seal_run_that_lost_a_frame_is_truncated() {
    bounded(|| {
        let groups = groups();
        let run = raw::seal_run(&groups, 1);
        let ends = frame_ends(&run);
        for (label, lost) in [("seal frame", ends[0]..ends[1]), ("footer", 0..ends[0])] {
            let mut bytes = [groups.clone(), run.clone()].concat();
            bytes[groups.len() + lost.start..groups.len() + lost.end].fill(0);
            assert_cut_to(bytes, &groups, label);
        }
    });
}

/// Given a newest segment that ends in a complete seal's closing run, where an earlier group frame
/// has a bad checksum
/// When the log opens
/// Then open fails with `Corrupt` at that group frame, because the segment is sealed and recovery
/// never truncates one
/// And no file changes
///
/// Cases: RECOVERY-FIXTURES, ORACLE-6
#[test]
fn a_bad_group_before_a_complete_seal_run_fails_open() {
    bounded(|| {
        let groups = groups();
        let sealed = [groups.clone(), raw::seal_run(&groups, 1)].concat();
        let second = 64 + raw::frames(&groups)[0].len();
        for bad in [64, second] {
            let mut bytes = sealed.clone();
            bytes[bad + 40] ^= 1;
            let sim = Sim::new(813);
            raw::install_segments(&sim, &[(0, bytes)]);
            let before = sim.image();
            let result = try_open(&sim);
            assert!(
                matches!(
                    result,
                    Err(OpenError::Corrupt { segment: 0, position }) if position == bad as u64
                ),
                "bad group at {bad}: {:?}",
                result.err()
            );
            assert_eq!(sim.image(), before);
        }
    });
}

/// Given a segment that ends in a complete seal's closing run and has a successor
/// When the log opens
/// Then open fails with `Corrupt` at that segment's footer, because only the final segment of a
/// log can be sealed by a seal, and no file changes
///
/// Cases: RECOVERY-FIXTURES, OPEN-LAYOUT, ORACLE-6
#[test]
fn a_seal_run_before_another_segment_fails_open() {
    bounded(|| {
        let groups = groups();
        let sealed = [groups.clone(), raw::seal_run(&groups, 1)].concat();
        let sim = Sim::new(814);
        raw::install_segments(&sim, &[(0, sealed), (3, raw::header(raw::VERSION, 3, 1))]);
        let before = sim.image();
        let result = try_open(&sim);
        assert!(
            matches!(
                result,
                Err(OpenError::Corrupt { segment: 0, position }) if position == groups.len() as u64
            ),
            "{:?}",
            result.err()
        );
        assert_eq!(sim.image(), before);
    });
}
