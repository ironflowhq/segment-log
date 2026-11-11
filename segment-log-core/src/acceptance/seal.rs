//! Seal, the read-only open of a sealed log, and `LogReader`.
use super::support::{bounded, fixtures::*, raw};
use crate::storage::{Call, Op, Purpose};
use crate::*;
use std::{path::Path, sync::Arc};

mod admission;
mod crashes;
mod reader;
mod sealed_open;
mod sealing;

/// No roll, so that a scenario can name the segment that each frame lands in.
fn unrolled() -> Options {
    Options {
        roll_bytes: Options::default().roll_bytes,
        ..options()
    }
}

/// Rolls every few groups.
fn rolling() -> Options {
    Options {
        roll_bytes: 300,
        ..options()
    }
}

/// Appends rounds that each add a stream that only that round has, keyed in most rounds, and a
/// record to one stream that appends throughout.
fn fill(log: &SegmentLog) {
    for round in 0..24u64 {
        let once = format!("once-{round}").into_bytes();
        let key = format!("k{round}").into_bytes();
        let records = vec![
            record(&once, (round % 3 != 0).then_some(&key), b"only", None),
            record(b"live", None, &round.to_le_bytes(), round.checked_sub(1)),
        ];
        send(log, records).wait().unwrap();
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

/// Watermark, every stream head and every record, as a reader of the log sees them.
type Observed = (u64, Vec<(Vec<u8>, Option<(u64, u64)>)>, String);

fn observe(
    watermark: u64,
    records: Vec<Record>,
    head: impl Fn(&[u8]) -> Option<StreamHead>,
) -> Observed {
    let mut streams: Vec<_> = records.iter().map(|r| r.stream.clone()).collect();
    streams.sort();
    streams.dedup();
    let heads = streams
        .into_iter()
        .map(|s| {
            let at = head(&s).map(|h| (h.offset, h.revision));
            (s, at)
        })
        .collect();
    (watermark, heads, format!("{records:?}"))
}

fn observe_log(log: &SegmentLog) -> Observed {
    observe(log.watermark(), log.read(0, usize::MAX).unwrap(), |s| {
        log.stream_head(s)
    })
}

fn group_flush() -> Call {
    Call::new(ANY_SEG, Purpose::Group, Op::SyncData)
}

fn seal_flush() -> Call {
    Call::new(ANY_SEG, Purpose::Seal, Op::SyncData)
}

fn seal_on_a_thread(log: &Arc<SegmentLog>) -> std::thread::JoinHandle<Result<(), SealError>> {
    let log = log.clone();
    std::thread::spawn(move || log.seal())
}

/// Whether the newest segment ends in a seal's closing run under `epoch`. Anything after its
/// group frames must be that whole run.
fn sealed_on_disk(sim: &Sim, epoch: u64) -> bool {
    let (_, newest) = raw::segments(sim).pop().unwrap();
    let (groups, run) = raw::split_run(&newest);
    if run.is_empty() {
        return false;
    }
    assert_eq!(run, raw::seal_run(groups, epoch), "a whole seal's run");
    true
}
