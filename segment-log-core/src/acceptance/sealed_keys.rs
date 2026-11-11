use super::support::{bounded, fixtures::*, raw};
use crate::{engine::Hooks, storage::Purpose, *};
use std::{path::Path, sync::Arc};

mod collisions;
mod lifetime;
mod memory;
mod poison;

const EVERY_GROUP: u64 = 65;

fn rolling() -> Options {
    Options {
        roll_bytes: EVERY_GROUP,
        ..options()
    }
}

fn open_hooked(sim: &Sim, hooks: &Hooks) -> SegmentLog {
    let before = raw::segments(sim);
    let log = SegmentLog::with_hooks(
        Path::new("/log"),
        rolling(),
        Arc::new(sim.clone()),
        hooks.clone(),
    )
    .unwrap();
    super::support::oracle::sealed_unchanged(&before, &raw::segments(sim));
    log
}

fn keyed(key: &[u8]) -> NewRecord {
    NewRecord {
        stream: [b"stream-", key].concat(),
        key: Some(key.to_vec()),
        payload: key.to_vec(),
        expected: ExpectedRevision::Empty,
    }
}

fn commit(log: &SegmentLog, records: Vec<NewRecord>) -> Committed {
    log.append(Submit { records }).unwrap().wait().unwrap()
}

fn offsets(answer: &Committed) -> Vec<u64> {
    answer.records.iter().map(|p| p.offset).collect()
}

fn assert_retry(log: &SegmentLog, keys: &[&[u8]], expected: &[u64]) {
    let watermark = log.watermark();
    let answer = commit(log, keys.iter().map(|key| keyed(key)).collect());
    assert!(answer.deduplicated, "retry of {keys:?}");
    assert_eq!(offsets(&answer), expected, "retry of {keys:?}");
    assert_eq!(log.watermark(), watermark);
}

fn reads(sim: &Sim, base: u64, purpose: Purpose) -> usize {
    sim.count(
        Path::new(&raw::segment(base)),
        purpose,
        &crate::storage::Op::Read(0, 0),
    )
}
