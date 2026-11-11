//! Seal and the read-only open of a sealed log.
use super::support::{bounded, fixtures::*, raw};
use crate::storage::{Op, Purpose};
use crate::*;
use std::{path::Path, sync::Arc};

mod sealed_open;

/// No roll, so that a scenario can name the segment that each frame lands in.
fn unrolled() -> Options {
    Options {
        roll_bytes: Options::default().roll_bytes,
        ..options()
    }
}

fn try_open(sim: &Sim) -> Result<SegmentLog, OpenError> {
    SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone()))
}

fn record(stream: &[u8], key: Option<&[u8]>, payload: &[u8], revision: Option<u64>) -> NewRecord {
    NewRecord {
        stream: stream.to_vec(),
        key: key.map(<[u8]>::to_vec),
        payload: payload.to_vec(),
        expected: revision.map_or(ExpectedRevision::Empty, ExpectedRevision::At),
    }
}

fn submit(records: Vec<NewRecord>) -> Submit {
    Submit { records }
}

fn send(log: &SegmentLog, records: Vec<NewRecord>) -> Pending {
    log.append(submit(records)).unwrap()
}

/// A valid, a malformed and an oversized submit, which a log refuses in that order of checks.
fn refusals(log: &SegmentLog) -> [Result<Pending, AppendError>; 3] {
    let mut large = record(b"s", None, b"", None);
    large.payload = vec![0; Options::default().group_byte_cap];
    [
        log.append(submit(vec![record(b"s", Some(b"new"), b"x", None)])),
        log.append(submit(vec![record(b"", None, b"x", None)])),
        log.append(submit(vec![large])),
    ]
}

fn assert_sealed(log: &SegmentLog) {
    let [valid, invalid, large] = refusals(log).map(Result::err);
    assert!(matches!(valid, Some(AppendError::Sealed)), "{valid:?}");
    assert!(
        matches!(invalid, Some(AppendError::Invalid(_))),
        "{invalid:?}"
    );
    assert!(matches!(large, Some(AppendError::TooLarge)), "{large:?}");
}

fn key_filter_reads(sim: &Sim) -> usize {
    sim.count(Path::new(ANY_SEG), Purpose::KeyFilter, &Op::Read(0, 0))
}
