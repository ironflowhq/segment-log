use super::*;
use crate::acceptance::support::{bounded, oracle};
use crate::storage::{Call, Op, Purpose};

fn frame_counts(segment: &[u8]) -> Vec<usize> {
    let (log_id, _, _) = raw::header_fields(segment);
    raw::frames(raw::split_run(segment).0)
        .into_iter()
        .map(|frame| oracle::frame_inputs(&log_id, frame).len())
        .collect()
}

/// Given a new log whose roll size every segment with a frame has reached
/// When one submit commits, then two more queue behind a held flush, then a last one commits
/// Then each group starts a file named by the next log offset
/// And each header has that base offset, the id of the log and the writer epoch
/// And every group frame sits wholly in one file, with the queued submits in one frame
/// And each file before the newest ends in its roll's closing run, under the writer epoch
/// And `read` and `stream_head` span the files
/// When the log reopens and appends again
/// Then the new segment's header and the closing run of the segment before it have the next
/// writer epoch
///
/// Cases: GROUP-FORMATION, READ-BOUNDED, HISTORY
#[test]
fn each_roll_starts_a_segment_named_by_the_next_offset() {
    bounded(|| {
        let sim = Sim::new(310);
        let log = open_rolling(&sim, EVERY_GROUP);
        send(&log, vec![record(b"a", None, b"a0", None)])
            .wait()
            .unwrap();
        let flush = sim.hold(Call::new(raw::segment(1), Purpose::Group, Op::SyncData));
        let second = send(
            &log,
            vec![
                record(b"b", None, b"b0", None),
                record(b"b", None, b"b1", Some(0)),
            ],
        );
        flush.wait();
        let third = send(&log, vec![record(b"c", None, b"c0", None)]);
        let fourth = send(
            &log,
            vec![
                record(b"a", None, b"a1", Some(0)),
                record(b"a", None, b"a2", Some(1)),
            ],
        );
        flush.release(None);
        assert_eq!(positions(&second.wait().unwrap()), [(1, 0), (2, 1)]);
        assert_eq!(positions(&third.wait().unwrap()), [(3, 0)]);
        assert_eq!(positions(&fourth.wait().unwrap()), [(4, 1), (5, 2)]);
        send(&log, vec![record(b"d", None, b"d0", None)])
            .wait()
            .unwrap();

        let segments = raw::segments(&sim);
        assert_eq!(bases(&sim), [0, 1, 3, 6]);
        let (log_id, _, _) = raw::header_fields(&segments[0].1);
        for (base, bytes) in &segments {
            assert_eq!(raw::header_fields(bytes), (log_id, *base, 1));
        }
        let counts: Vec<_> = segments
            .iter()
            .map(|(_, bytes)| frame_counts(bytes))
            .collect();
        assert_eq!(counts, [vec![1], vec![2], vec![3], vec![1]]);
        for (index, (_, bytes)) in segments.iter().enumerate() {
            let (groups, run) = raw::split_run(bytes);
            if index + 1 < segments.len() {
                assert_eq!(run, raw::roll_run(groups, 1), "segment {index}");
            } else {
                assert!(run.is_empty(), "the newest segment holds only groups");
            }
        }

        let expected = [
            stored(b"a", None, b"a0", 0, 0),
            stored(b"b", None, b"b0", 1, 0),
            stored(b"b", None, b"b1", 2, 1),
            stored(b"c", None, b"c0", 3, 0),
            stored(b"a", None, b"a1", 4, 1),
            stored(b"a", None, b"a2", 5, 2),
            stored(b"d", None, b"d0", 6, 0),
        ];
        assert_eq!(log.watermark(), 7);
        assert_records(&log.read(0, 100).unwrap(), &expected);
        assert_records(&log.read(2, 3).unwrap(), &expected[2..5]);
        for (stream, head) in [
            (b"a", (5, 2)),
            (b"b", (2, 1)),
            (b"c", (3, 0)),
            (b"d", (6, 0)),
        ] {
            let actual = log.stream_head(stream).unwrap();
            assert_eq!((actual.offset, actual.revision), head);
        }

        drop(log);
        let log = open_rolling(&sim, EVERY_GROUP);
        send(&log, vec![record(b"d", None, b"d1", Some(0))])
            .wait()
            .unwrap();
        let segments = raw::segments(&sim);
        assert_eq!(bases(&sim), [0, 1, 3, 6, 7]);
        assert_eq!(raw::header_fields(&segments[4].1), (log_id, 7, 2));
        let (groups, run) = raw::split_run(&segments[3].1);
        assert_eq!(run, raw::roll_run(groups, 2));
        assert_eq!(log.read(7, 1).unwrap()[0].payload, b"d1");
    });
}

/// Given a roll size that the first group does not reach and the second group passes
/// When three groups commit
/// Then the second group sits wholly in the first segment, which passes the roll size
/// And the third group starts the next segment
/// And a segment exactly at the roll size rolls before the next group
/// And a roll size of 1 byte does not roll a segment that has no record
///
/// Cases: GROUP-FORMATION, OPT-VALIDATION
#[test]
fn a_group_never_crosses_a_roll() {
    bounded(|| {
        let sim = Sim::new(311);
        let one = raw::group(1, 0, &[b"x"]).len() as u64;
        let roll_bytes = 64 + one + 1;
        let log = open_rolling(&sim, roll_bytes);
        send(&log, vec![record(b"s", None, b"x", None)])
            .wait()
            .unwrap();
        send(
            &log,
            vec![
                record(b"s", None, &[7; 300], Some(0)),
                record(b"s", None, &[8; 300], Some(1)),
            ],
        )
        .wait()
        .unwrap();
        send(&log, vec![record(b"s", None, b"z", Some(2))])
            .wait()
            .unwrap();
        let segments = raw::segments(&sim);
        assert_eq!(bases(&sim), [0, 3]);
        assert!(segments[0].1.len() as u64 > roll_bytes);
        assert_eq!(frame_counts(&segments[0].1), [1, 2]);
        assert_eq!(frame_counts(&segments[1].1), [1]);
        assert_eq!(log.read(0, 100).unwrap().len(), 4);

        let sim = Sim::new(311);
        let log = open_rolling(&sim, 64 + one);
        send(&log, vec![record(b"s", None, b"x", None)])
            .wait()
            .unwrap();
        assert_eq!(raw::segments(&sim)[0].1.len() as u64, 64 + one);
        send(&log, vec![record(b"s", None, b"y", Some(0))])
            .wait()
            .unwrap();
        assert_eq!(bases(&sim), [0, 1]);

        let sim = Sim::new(311);
        let log = open_rolling(&sim, 1);
        send(&log, vec![record(b"s", None, b"x", None)])
            .wait()
            .unwrap();
        send(&log, vec![record(b"s", None, b"y", Some(0))])
            .wait()
            .unwrap();
        assert_eq!(bases(&sim), [0, 1]);
    });
}

/// Given a segment that has reached the roll size
/// When submits arrive that append nothing: a deduplicated retry, a key conflict,
/// A mix of a retried and a new record, and a wrong expected revision
/// Then each gets its answer, no segment appears, and the storage sees no write and no flush
/// When a submit with a new record follows
/// Then the log rolls before that record's group
///
/// Cases: GROUP-FORMATION, RETRY-REGROUPED, KEY-CONFLICT, KEY-PARTIAL, REV-EXPECTED
#[test]
fn a_submit_that_appends_nothing_does_not_roll() {
    bounded(|| {
        use crate::storage::sim::Event;
        let sim = Sim::new(314);
        let log = open_rolling(&sim, EVERY_GROUP);
        let original = || record(b"s", Some(b"k"), b"original", None);
        send(&log, vec![original()]).wait().unwrap();
        let before = sim.trace().1.len();

        let retry = send(&log, vec![original()]).wait().unwrap();
        assert!(retry.deduplicated);
        assert_eq!(positions(&retry), [(0, 0)]);
        assert!(matches!(
            send(&log, vec![record(b"s", Some(b"k"), b"other", None)]).wait(),
            Err(CommitError::KeyConflict { index: 0 })
        ));
        assert!(matches!(
            send(&log, vec![original(), record(b"t", None, b"new", None)]).wait(),
            Err(CommitError::PartialRetry)
        ));
        assert!(matches!(
            send(&log, vec![record(b"s", None, b"stale", None)]).wait(),
            Err(CommitError::WrongRevision {
                index: 0,
                actual: Some(0)
            })
        ));
        assert!(
            sim.trace().1[before..].iter().all(|event| matches!(
                event,
                Event::Call(call, _, _) if matches!(call.op, Op::Read(..))
            )),
            "a submit that appends nothing only reads"
        );

        send(&log, vec![record(b"s", None, b"next", Some(0))])
            .wait()
            .unwrap();
        assert_eq!(bases(&sim), [0, 1]);
    });
}

/// Given keyed records that later rolls left in older segments
/// When a caller retries them, alone and regrouped across segments, before and after a reopen
/// Then each retry returns the original positions with `deduplicated: true`
/// And it appends nothing and creates no segment
/// And the same key with another payload is rejected with `KeyConflict`
///
/// Cases: KEY-LIFETIME, RETRY-REGROUPED, KEY-CONFLICT
#[test]
fn a_key_in_an_older_segment_still_deduplicates_a_retry() {
    bounded(|| {
        let sim = Sim::new(312);
        let mut log = open_rolling(&sim, EVERY_GROUP);
        let keyed = |n: u8| record(&[b's', n], Some(&[b'k', n]), &[n; 9], None);
        for n in 0..3 {
            send(&log, vec![keyed(n)]).wait().unwrap();
        }
        assert_eq!(bases(&sim), [0, 1, 2]);
        let sealed = raw::segments(&sim);
        for reopened in [false, true] {
            if reopened {
                drop(log);
                log = open_rolling(&sim, EVERY_GROUP);
            }
            let retry = send(&log, vec![keyed(0)]).wait().unwrap();
            assert!(retry.deduplicated);
            assert_eq!(positions(&retry), [(0, 0)]);
            let regrouped = send(&log, vec![keyed(2), keyed(0), keyed(1)])
                .wait()
                .unwrap();
            assert!(regrouped.deduplicated);
            assert_eq!(positions(&regrouped), [(2, 0), (0, 0), (1, 0)]);
            let mut conflict = keyed(0);
            conflict.payload.push(1);
            assert!(matches!(
                send(&log, vec![conflict]).wait(),
                Err(CommitError::KeyConflict { index: 0 })
            ));
            assert_eq!(log.watermark(), 3);
            assert_eq!(raw::segments(&sim), sealed);
        }
    });
}

/// Given a log that rolls after every group
/// When some streams append once and never again, and one stream appends through every roll
/// And the log reopens and rolls further
/// Then every segment file remains with the bytes it had when its successor appeared
/// And every record of every stream is readable, and each stream head is its last record
///
/// Cases: HISTORY, ORACLE-6, ORACLE-12
#[test]
fn repeated_rolls_keep_every_segment_and_every_stream() {
    bounded(|| {
        let sim = Sim::new(313);
        let mut log = open_rolling(&sim, EVERY_GROUP);
        let mut expected = vec![];
        let mut sealed: Vec<(u64, Vec<u8>)> = vec![];
        for round in 0..8u64 {
            if round == 4 {
                drop(log);
                log = open_rolling(&sim, EVERY_GROUP);
            }
            let once = format!("once-{round}").into_bytes();
            let offset = expected.len() as u64;
            send(
                &log,
                vec![
                    record(&once, Some(&once), b"only", None),
                    record(b"live", None, &round.to_le_bytes(), round.checked_sub(1)),
                ],
            )
            .wait()
            .unwrap();
            expected.push(stored(&once, Some(&once), b"only", offset, 0));
            expected.push(stored(
                b"live",
                None,
                &round.to_le_bytes(),
                offset + 1,
                round,
            ));
            let segments = raw::segments(&sim);
            assert_eq!(segments.len() as u64, round + 1);
            assert_eq!(segments[..sealed.len()], sealed);
            sealed = segments[..segments.len() - 1].to_vec();
        }
        for reopened in [false, true] {
            if reopened {
                drop(log);
                log = open_rolling(&sim, EVERY_GROUP);
            }
            oracle::history(&log, &expected);
            assert_eq!(raw::segments(&sim)[..sealed.len()], sealed);
        }
    });
}
