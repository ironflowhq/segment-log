//! The scenarios that roll set their own roll size, because the file names they expect follow from it.
use super::support::{fixtures::*, raw};
use crate::*;
use std::{path::Path, sync::Arc};

mod crashes;
mod layouts;
mod rolling;

fn open_rolling(sim: &Sim, roll_bytes: u64) -> SegmentLog {
    open_with(
        sim,
        Options {
            roll_bytes,
            ..options()
        },
    )
}

fn record(stream: &[u8], key: Option<&[u8]>, payload: &[u8], revision: Option<u64>) -> NewRecord {
    NewRecord {
        stream: stream.to_vec(),
        key: key.map(<[u8]>::to_vec),
        payload: payload.to_vec(),
        expected: revision.map_or(ExpectedRevision::Empty, ExpectedRevision::At),
    }
}

fn send(log: &SegmentLog, records: Vec<NewRecord>) -> Pending {
    log.append(Submit { records }).unwrap()
}

fn positions(answer: &Committed) -> Vec<(u64, u64)> {
    answer
        .records
        .iter()
        .map(|position| (position.offset, position.revision))
        .collect()
}

fn bases(sim: &Sim) -> Vec<u64> {
    raw::segments(sim)
        .into_iter()
        .map(|(base, _)| base)
        .collect()
}
