//! The rules that format.md states for ADR 0064, rule 3, which the code and ADR 0054 already
//! depend on.
use super::support::{bounded, fixtures::*, raw};
use crate::codec;
use crate::*;

const STREAMS: [&[u8]; 5] = [b"b", &[0x80], b"ab", b"a", b"B"];

/// One group with a record on each of `STREAMS`, then one more group, so segment 0 rolls.
fn rolled_segment(sim: &Sim) -> Vec<u8> {
    let log = open_with(
        sim,
        Options {
            roll_bytes: EVERY_GROUP,
            ..options()
        },
    );
    let records = |streams: &[&[u8]]| Submit {
        records: streams
            .iter()
            .map(|stream| NewRecord {
                stream: stream.to_vec(),
                expected: ExpectedRevision::Empty,
                key: None,
                payload: b"p".to_vec(),
            })
            .collect(),
    };
    log.append(records(&STREAMS)).unwrap().wait().unwrap();
    log.append(records(&[b"z"])).unwrap().wait().unwrap();
    drop(log);
    raw::segments(sim).remove(0).1
}

fn log_id(segment: &[u8]) -> codec::LogId {
    codec::LogId::new(raw::header_fields(segment).0)
}

fn decoded<'a>(log: &codec::LogId, frame: &'a [u8]) -> codec::Frame<'a> {
    codec::decode(log, frame).unwrap()
}

/// Installs `groups` followed by `run` as the only segment and opens the log. Returns the bytes
/// open kept, or its error, after checking that a failed open changed no file.
fn open_segment(sim: &Sim, groups: &[u8], run: &[u8]) -> Result<Vec<u8>, OpenError> {
    let segment = [groups, run].concat();
    raw::install_segments(sim, &[(0, segment.clone())]);
    let opened = SegmentLog::with_storage(
        std::path::Path::new("/log"),
        options(),
        std::sync::Arc::new(sim.clone()),
    );
    let kept = raw::segments(sim).remove(0).1;
    match opened {
        Ok(_) => Ok(kept),
        Err(error) => {
            assert_eq!(kept, segment, "a failed open changes no file");
            Err(error)
        }
    }
}

fn is_corrupt_at(opened: Result<Vec<u8>, OpenError>, groups: &[u8]) -> bool {
    matches!(opened, Err(OpenError::Corrupt { segment: 0, position }) if position == groups.len() as u64)
}

/// Given a segment whose group holds records on streams `b`, `0x80`, `ab`, `a` and `B`
/// When the writer rolls it
/// Then its footer names the streams in unsigned byte order: `B`, `a`, `ab`, `b`, `0x80`
/// When the newest segment ends in a roll's run whose footer names the same streams in another
/// order, with every CRC valid
/// Then the run is not the run the groups give, so open fails with `Corrupt` where it starts
///
/// Cases: FORMAT-RULES, RECOVERY-FIXTURES
#[test]
fn footer_entries_are_sorted_by_unsigned_stream_bytes() {
    bounded(|| {
        let sim = Sim::new(650);
        let segment = rolled_segment(&sim);
        let log = log_id(&segment);
        let (groups, run) = raw::split_run(&segment);
        let frames = raw::frames(&[&[0; 64][..], run].concat())
            .into_iter()
            .map(<[u8]>::to_vec)
            .collect::<Vec<_>>();
        let codec::Body::Footer(footer) = decoded(&log, &frames[2]).body else {
            panic!("a roll's third closing frame is its footer")
        };
        let order: Vec<&[u8]> = footer.streams.iter().map(|tail| tail.stream).collect();
        assert_eq!(order, [&b"B"[..], b"a", b"ab", b"b", &[0x80]]);

        let footer_frame = decoded(&log, &frames[2]);
        let mut reversed = footer.streams;
        reversed.reverse();
        let unsorted = codec::encode_footer(
            &log,
            footer_frame.epoch,
            footer_frame.base_offset,
            &codec::Footer {
                key_filter_position: footer.key_filter_position,
                streams: reversed,
            },
        )
        .unwrap();
        assert_eq!(unsorted.len(), frames[2].len());
        let run = [&frames[0][..], &frames[1], &unsorted, &frames[3]].concat();
        let sim = Sim::new(651);
        assert!(open_segment(&sim, groups, &frames.concat()).is_ok());
        assert!(is_corrupt_at(open_segment(&sim, groups, &run), groups));
    });
}

/// Given a newest segment that ends in a complete closing run, of a roll or of a seal
/// When every frame of the run has the run's epoch
/// Then open keeps the run
/// When the run's first frames have one epoch and its last frames a higher one, with every CRC
/// valid and no frame below the frame before it
/// Then the run is not the run the groups give, so open fails with `Corrupt` where it starts
///
/// Cases: FORMAT-RULES, RECOVERY-FIXTURES
#[test]
fn every_frame_of_a_closing_run_carries_the_run_epoch() {
    bounded(|| {
        let groups = [
            raw::header(raw::VERSION, 0, 1),
            raw::group(1, 0, &[b"a", b"b"]),
        ]
        .concat();
        let split = |run: &[u8]| -> Vec<Vec<u8>> {
            raw::frames(&[&[0; 64][..], run].concat())
                .into_iter()
                .map(<[u8]>::to_vec)
                .collect()
        };
        for (kind, low, high) in [
            ("roll", raw::roll_run(&groups, 1), raw::roll_run(&groups, 2)),
            ("seal", raw::seal_run(&groups, 1), raw::seal_run(&groups, 2)),
        ] {
            let sim = Sim::new(652);
            assert!(
                open_segment(&sim, &groups, &high).is_ok(),
                "{kind}: a run with one epoch"
            );
            let (low, high) = (split(&low), split(&high));
            for cut in 1..low.len() {
                let mixed = [&low[..cut], &high[cut..]].concat().concat();
                let sim = Sim::new(653);
                assert!(
                    is_corrupt_at(open_segment(&sim, &groups, &mixed), &groups),
                    "{kind}: epoch 1 for the first {cut} frames, then epoch 2"
                );
            }
        }
    });
}

/// Given a newest segment whose header has epoch 3
/// When its first group frame has epoch 2, with a valid CRC
/// Then the frame ends the log, and open truncates the segment to its header
///
/// Cases: FORMAT-RULES, RECOVERY-FIXTURES
#[test]
fn a_frame_below_its_segment_header_epoch_ends_the_log() {
    bounded(|| {
        let header = raw::header(raw::VERSION, 0, 3);
        let sim = Sim::new(654);
        let kept = open_segment(&sim, &header, &raw::group(2, 0, &[b"a"]));
        assert_eq!(kept.unwrap(), header);
        let sim = Sim::new(655);
        let at_header = raw::group(3, 0, &[b"a"]);
        assert_eq!(
            open_segment(&sim, &header, &at_header).unwrap(),
            [&header[..], &at_header].concat()
        );
    });
}

/// Given the limits and constants of format.md
/// When the build's format version is 3
/// Then each has its version 3 value: a change to one is a new format version (ADR 0064, rule 3)
///
/// Cases: FORMAT-RULES
#[test]
fn every_limit_belongs_to_format_version_3() {
    assert_eq!(codec::FORMAT_VERSION, 3);
    assert_eq!(
        (
            codec::FRAME_BODY_LIMIT,
            codec::GROUP_BODY_LIMIT,
            codec::ROLL_BYTES_LIMIT,
            codec::KEYED_RECORD_LIMIT,
            codec::STREAM_LEN_MAX,
            codec::KEY_LEN_MAX,
        ),
        (
            16 * 1024 * 1024,
            8 * 1024 * 1024,
            4000 * 1024 * 1024,
            1_044_225,
            1024,
            256,
        ),
        "the limits table"
    );
    assert_eq!(
        (
            codec::HEADER_LEN,
            codec::BODY_HEADER_LEN,
            codec::GROUP_HEADER_LEN,
            codec::FOOTER_ENTRY_LEN,
            codec::LOCATOR_LEN,
            codec::PAGE_SIZE,
            codec::PAGE_ENTRIES,
        ),
        (64, 24, 32, 28, 40, 4096, 255),
        "the layout constants"
    );
    // The filter: 16 bits for each key, 11 probes, 64-byte blocks. Format.md's check value pins
    // the salts and the block choice.
    let (block, mut bits) = codec::filter_probes(0xa129ca6149be45e5, 3);
    bits.sort_unstable();
    assert_eq!(block, 1);
    assert_eq!(bits, [27, 32, 93, 97, 144, 171, 364, 386, 393, 426, 434]);
}
