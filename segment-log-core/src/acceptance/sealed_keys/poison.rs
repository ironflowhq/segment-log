use super::*;
use crate::{
    codec,
    storage::{
        Call, Op, Storage,
        sim::{Crash, Event, Fault},
    },
};

fn segment_zero(sim: &Sim) -> Vec<u8> {
    raw::segments(sim).remove(0).1
}

fn rewrite(sim: &Sim, bytes: Vec<u8>) {
    let path = raw::segment(0);
    sim.call(Call::new(&path, Purpose::Open, Op::Truncate(0)))
        .unwrap();
    sim.call(Call::new(&path, Purpose::Open, Op::Write(0, bytes)))
        .unwrap();
}

struct Reads {
    page: (usize, Call),
    envelope: Call,
    frame: (usize, Call),
}

fn reads_of(segment: &[u8]) -> Reads {
    let (groups, run) = raw::split_run(segment);
    let frame = groups.len() - 64;
    let filter = u32::from_le_bytes(run[..4].try_into().unwrap()) as usize + 8;
    let page = groups.len() + filter + 8 + 24;
    let read =
        |purpose, at: usize, len| Call::new(raw::segment(0), purpose, Op::Read(at as u64, len));
    Reads {
        page: (page, read(Purpose::KeyIndex, page, 4096)),
        envelope: read(Purpose::Lookup, 64, 8),
        frame: (frame, read(Purpose::Lookup, 64, frame)),
    }
}

enum Damage {
    Fault(Fault),
    Bytes(Vec<u8>),
}

fn poisoned_lookup(target: &Call, damage: impl Fn(&[u8]) -> Damage, label: &str) {
    for buffered in [false, true] {
        let label = format!("{label}, buffered {buffered}");
        let sim = Sim::new(370);
        let hooks = Hooks::default();
        let log = open_hooked(&sim, &hooks);
        commit(&log, vec![keyed(b"z")]);
        commit(&log, vec![keyed(b"a")]);
        let intact = segment_zero(&sim);
        let (new, retry, read) = if buffered {
            let flush = sim.hold(Call::new(ANY_SEG, Purpose::Group, Op::SyncData));
            let previous = log.append(Submit {
                records: vec![keyed(b"previous")],
            });
            flush.wait();
            let new = log.append(Submit {
                records: vec![keyed(b"new")],
            });
            let read = sim.hold(target.clone());
            let retry = log.append(Submit {
                records: vec![keyed(b"z")],
            });
            flush.release(None);
            previous.unwrap().wait().unwrap();
            (Some(new.unwrap()), retry.unwrap(), read)
        } else {
            let read = sim.hold(target.clone());
            let retry = log.append(Submit {
                records: vec![keyed(b"z")],
            });
            (None, retry.unwrap(), read)
        };
        read.wait();
        let queued = log
            .append(Submit {
                records: vec![keyed(b"queued")],
            })
            .unwrap();
        let fault = match damage(&intact) {
            Damage::Fault(fault) => Some(fault),
            Damage::Bytes(bytes) => {
                rewrite(&sim, bytes);
                None
            }
        };
        let released = sim.trace().1.len();
        read.release(fault);
        assert!(
            matches!(retry.wait(), Err(CommitError::Indeterminate)),
            "{label}"
        );
        if let Some(new) = new {
            assert!(
                matches!(new.wait(), Err(CommitError::Indeterminate)),
                "{label}"
            );
        }
        assert!(
            matches!(queued.wait(), Err(CommitError::Poisoned)),
            "{label}"
        );
        assert!(matches!(
            log.append(Submit {
                records: vec![keyed(b"later")]
            }),
            Err(AppendError::Poisoned)
        ));
        let kept = log.watermark();
        drop(log);
        assert!(
            !sim.trace().1[released..].iter().any(|event| matches!(event,
                Event::Call(call, _, _) if !matches!(call.op, Op::Read(..)))),
            "{label}: the poisoned writer wrote or flushed"
        );

        rewrite(&sim, intact.clone());
        drop(open_hooked(&sim, &hooks));
        sim.crash(Crash::PowerLoss, 370);
        let log = open_hooked(&sim, &hooks);
        assert_eq!(log.watermark(), kept, "{label}");
        assert_retry(&log, &[b"z", b"a"], &[0, 1]);
    }
}

/// Given a key `z` in a sealed segment
/// When the read of its key index page fails with an I/O error, returns nothing, returns a page
/// with a bad checksum, or returns a page with a valid checksum and a wrong entry count
/// Then the retry of `z` gets `Indeterminate`, and so does a new submit buffered before it in its
/// group
/// And a queued submit and every later append get `Poisoned`, and no write or flush follows
/// And a reopen, a power loss and another reopen keep every record, and the retry then matches
///
/// Cases: POISON-LOOKUP, ORACLE-7
#[test]
fn a_failed_key_index_page_read_poisons_the_writer() {
    bounded(|| {
        let sim = Sim::new(370);
        let log = open_hooked(&sim, &Hooks::default());
        commit(&log, vec![keyed(b"z")]);
        commit(&log, vec![keyed(b"a")]);
        drop(log);
        let Reads {
            page: (page, target),
            ..
        } = reads_of(&segment_zero(&sim));
        let other = || Fault::error(std::io::ErrorKind::Other).unwrap();
        poisoned_lookup(&target, |_| Damage::Fault(other()), "read error");
        poisoned_lookup(&target, |_| Damage::Fault(Fault::Eof), "short read");
        poisoned_lookup(
            &target,
            |_| Damage::Fault(Fault::Corrupt(100, 1)),
            "bad page CRC",
        );
        poisoned_lookup(
            &target,
            |intact| {
                let mut bytes = intact.to_vec();
                bytes[page + 4080] = 2;
                let checksum = raw::crc(&bytes[page..page + 4092]);
                bytes[page + 4092..page + 4096].copy_from_slice(&checksum.to_le_bytes());
                Damage::Bytes(bytes)
            },
            "wrong entry count",
        );
    });
}

/// Given a key `z` in a sealed segment, whose key index names its group frame
/// When the read of that frame's envelope or of the whole frame fails with an I/O error, returns
/// nothing or returns bytes with a bad checksum, or the frame has a valid checksum over a malformed
/// body
/// Then the retry of `z` gets `Indeterminate`, and so does a new submit buffered before it
/// And a queued submit and every later append get `Poisoned`, and no write or flush follows
/// And a reopen, a power loss and another reopen keep every record, and the retry then matches
///
/// Cases: POISON-LOOKUP, ORACLE-7
#[test]
fn a_failed_sealed_frame_read_poisons_the_writer() {
    bounded(|| {
        let sim = Sim::new(370);
        let log = open_hooked(&sim, &Hooks::default());
        commit(&log, vec![keyed(b"z")]);
        commit(&log, vec![keyed(b"a")]);
        drop(log);
        let Reads {
            envelope,
            frame: (frame, target),
            ..
        } = reads_of(&segment_zero(&sim));
        let other = || Fault::error(std::io::ErrorKind::Other).unwrap();
        for (read, label) in [(&envelope, "envelope"), (&target, "frame")] {
            poisoned_lookup(
                read,
                |_| Damage::Fault(other()),
                &format!("{label} read error"),
            );
            poisoned_lookup(
                read,
                |_| Damage::Fault(Fault::Eof),
                &format!("{label} short read"),
            );
        }
        poisoned_lookup(
            &envelope,
            |_| Damage::Fault(Fault::Corrupt(0, 0xff)),
            "bad length",
        );
        poisoned_lookup(
            &target,
            |_| Damage::Fault(Fault::Corrupt(8, 1)),
            "bad frame CRC",
        );
        poisoned_lookup(
            &target,
            |intact| {
                let mut bytes = intact.to_vec();
                bytes[64 + 32] = 0;
                let (log_id, _, _) = raw::header_fields(intact);
                raw::rechecksum(&log_id, &mut bytes[64..64 + frame]);
                Damage::Bytes(bytes)
            },
            "malformed group",
        );
    });
}

/// Given a sealed segment whose key index entry for `z` names its key filter frame, a position
/// inside its group frame, or a record past the end of that frame
/// When a caller retries `z`
/// Then the retry gets `Indeterminate`, a queued submit and every later append get `Poisoned`, and
/// no write or flush follows
/// And after a reopen the retry fails the same way, because the damage stays: never a key miss
///
/// Cases: POISON-LOOKUP, ORACLE-7
#[test]
fn a_key_index_entry_that_names_no_group_frame_poisons_the_writer() {
    bounded(|| {
        let log_id = codec::LogId::new(raw::LOG_ID);
        let z = keyed(b"z");
        let group = codec::encode_group(
            &log_id,
            &codec::Group {
                epoch: 1,
                base_offset: 0,
                entries: vec![codec::Entry {
                    stream: &z.stream,
                    key: z.key.as_deref(),
                    payload: &z.payload,
                    revision: 0,
                }],
            },
        )
        .unwrap();
        let groups = [raw::header(raw::VERSION, 0, 1), group].concat();
        let filter_at = groups.len();
        for (label, position, record) in [
            ("the key filter", filter_at, 0),
            ("inside the group frame", 65, 0),
            ("a record past the frame's count", 64, 1),
        ] {
            let mut entries = [codec::KeyEntry {
                hash: log_id.key_hash(b"z"),
                position: position as u32,
                record,
            }];
            let keys =
                codec::encode_key_frames(&log_id, 1, 1, filter_at as u64, &mut entries).unwrap();
            let footer = codec::encode_footer(
                &log_id,
                1,
                1,
                &codec::Footer {
                    key_filter_position: std::num::NonZeroU64::new(filter_at as u64),
                    streams: vec![codec::StreamTail {
                        stream: &z.stream,
                        last_revision: 0,
                        last_offset: 0,
                        last_position: 64,
                    }],
                },
            )
            .unwrap();
            let pages_at = filter_at + keys.filter.len() + 8 + 24;
            let footer_at = filter_at + keys.filter.len() + keys.index.len();
            let locator = codec::encode_locator(&log_id, 1, 1, footer_at as u64);
            let sealed = [groups.clone(), keys.filter, keys.index, footer, locator].concat();
            let sim = Sim::new(371);
            raw::install_segments(&sim, &[(0, sealed), (1, raw::header(raw::VERSION, 1, 1))]);
            for reopen in 0..2 {
                let label = format!("{label}, open {reopen}");
                let log = open_hooked(&sim, &Hooks::default());
                let page = sim.hold(Call::new(
                    raw::segment(0),
                    Purpose::KeyIndex,
                    Op::Read(pages_at as u64, 4096),
                ));
                let retry = log
                    .append(Submit {
                        records: vec![keyed(b"z")],
                    })
                    .unwrap();
                page.wait();
                let queued = log
                    .append(Submit {
                        records: vec![keyed(b"queued")],
                    })
                    .unwrap();
                let released = sim.trace().1.len();
                page.release(None);
                assert!(
                    matches!(retry.wait(), Err(CommitError::Indeterminate)),
                    "{label}"
                );
                assert!(
                    matches!(queued.wait(), Err(CommitError::Poisoned)),
                    "{label}"
                );
                assert_eq!(log.watermark(), 1, "{label}");
                drop(log);
                assert!(
                    !sim.trace().1[released..].iter().any(|event| matches!(event,
                        Event::Call(call, _, _) if !matches!(call.op, Op::Read(..)))),
                    "{label}: the poisoned writer wrote or flushed"
                );
            }
        }
    });
}
