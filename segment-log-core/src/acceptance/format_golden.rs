use super::support::{fixtures::*, raw};
use crate::storage::{Call, Op, Purpose, sim::Fault};
use crate::*;
use std::{path::Path, sync::Arc};

/// The log id of format.md's check values.
const LOG_ID: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// Submits that the first writer appends before the log reopens.
const FIRST: usize = 3;

fn rolling() -> Options {
    Options {
        roll_bytes: 300,
        ..options()
    }
}

/// Creates the log at epoch 1, appends the first submits, reopens it at epoch 2 and appends the
/// rest, so the fixture holds a creation, an epoch change and rolls, as a real history does.
fn write(sim: &Sim) -> SegmentLog {
    let random = Call::new("/log", Purpose::Open, Op::Random(16));
    sim.inject(random, Fault::Random(LOG_ID.to_vec()));
    let mut submits = submits().into_iter();
    let append = |log: &SegmentLog, submit| {
        log.append(Submit { records: submit })
            .unwrap()
            .wait()
            .unwrap();
    };
    let log = open_with(sim, rolling());
    submits.by_ref().take(FIRST).for_each(|s| append(&log, s));
    drop(log);
    let log = open_with(sim, rolling());
    submits.for_each(|s| append(&log, s));
    log
}

/// Each round appends a keyed record to `orders`, and odd rounds add an unkeyed record to `audit`.
fn submits() -> Vec<Vec<NewRecord>> {
    let mut audits = 0;
    (0..9u64)
        .map(|round| {
            let mut records = vec![NewRecord {
                stream: b"orders".to_vec(),
                expected: expected(round),
                key: Some(format!("order-{round}").into_bytes()),
                payload: format!("order {round}").into_bytes(),
            }];
            if round % 2 == 1 {
                records.push(NewRecord {
                    stream: b"audit".to_vec(),
                    expected: expected(audits),
                    key: None,
                    payload: format!("audit after order {round}").into_bytes(),
                });
                audits += 1;
            }
            records
        })
        .collect()
}

fn expected(count: u64) -> ExpectedRevision {
    count
        .checked_sub(1)
        .map_or(ExpectedRevision::Empty, ExpectedRevision::At)
}

/// The records that `submits` must produce, derived from the contract: offsets count up from 0 in
/// submit order, and each stream's revisions count up from 0.
fn records() -> Vec<Record> {
    let mut heads = std::collections::BTreeMap::<Vec<u8>, u64>::new();
    let all = submits().into_iter().flatten().enumerate();
    all.map(|(offset, r)| {
        let head = heads.entry(r.stream.clone()).or_default();
        *head += 1;
        Record {
            offset: offset as u64,
            revision: *head - 1,
            stream: r.stream,
            key: r.key,
            payload: r.payload,
        }
    })
    .collect()
}

type Fields<'a> = (u64, &'a [u8], u64, Option<&'a [u8]>, &'a [u8]);

fn fields(records: &[Record]) -> Vec<Fields<'_>> {
    records
        .iter()
        .map(|r| {
            let (stream, key) = (r.stream.as_slice(), r.key.as_deref());
            (r.offset, stream, r.revision, key, r.payload.as_slice())
        })
        .collect()
}

macro_rules! fixture {
    ($dir:literal, $($base:literal $file:literal),+) => {
        vec![$((
            $base,
            include_bytes!(concat!(
                "../../tests/fixtures/format-v2/", $dir, "/", $file, ".seg"
            ))
            .to_vec(),
        )),+]
    };
}

/// Two rolled segments and an active one.
fn open_fixture() -> Vec<(u64, Vec<u8>)> {
    fixture!("open", 0 "00000000000000000000", 4 "00000000000000000004", 9 "00000000000000000009")
}

/// The open fixture after `seal`.
fn sealed_fixture() -> Vec<(u64, Vec<u8>)> {
    fixture!("sealed", 0 "00000000000000000000", 4 "00000000000000000004", 9 "00000000000000000009")
}

fn assert_same(actual: &[(u64, Vec<u8>)], fixture: &[(u64, Vec<u8>)]) {
    let bases = |s: &[(u64, Vec<u8>)]| s.iter().map(|(base, _)| *base).collect::<Vec<_>>();
    assert_eq!(bases(actual), bases(fixture), "segment bases");
    for ((base, actual), (_, fixture)) in actual.iter().zip(fixture) {
        let first = actual.iter().zip(fixture).position(|(a, f)| a != f);
        assert!(
            actual == fixture,
            "segment {base} differs from the format v2 fixture at byte {:?} (lengths {} and {}); \
             a layout change increments format_version and replaces this fixture (ADR 0053)",
            first,
            actual.len(),
            fixture.len(),
        );
    }
}

/// Given the committed format v2 fixture: two rolled segments and an active one
/// When a writable log opens it
/// Then it reads every record at its offset and revision
/// And a retry of each keyed record returns the original position without appending
/// And a key reused with another payload is a key conflict
///
/// Cases: FORMAT-GOLDEN
#[test]
fn a_writable_log_opens_the_format_v2_fixture() {
    let sim = Sim::new(0);
    raw::install_segments(&sim, &open_fixture());
    let log = open_with(&sim, options());
    let expected = records();
    assert_eq!(fields(&log.read(0, 100).unwrap()), fields(&expected));
    for record in expected.iter().filter(|r| r.key.is_some()) {
        let retry = NewRecord {
            stream: record.stream.clone(),
            expected: ExpectedRevision::Empty,
            key: record.key.clone(),
            payload: record.payload.clone(),
        };
        let committed = log
            .append(Submit {
                records: vec![retry],
            })
            .unwrap()
            .wait();
        let committed = committed.unwrap();
        let positions: Vec<_> = committed
            .records
            .iter()
            .map(|p| (p.offset, p.revision))
            .collect();
        assert_eq!(
            (positions, committed.deduplicated),
            (vec![(record.offset, record.revision)], true),
            "retry of offset {}",
            record.offset,
        );
    }
    let mut conflict = submits().remove(0);
    conflict[0].payload = b"another payload".to_vec();
    let answer = log.append(Submit { records: conflict }).unwrap().wait();
    assert!(
        matches!(answer, Err(CommitError::KeyConflict { index: 0 })),
        "{answer:?}"
    );
    assert_eq!(log.watermark(), expected.len() as u64);
}

/// Given the committed format v2 fixture of a sealed log
/// When a `LogReader` opens it
/// Then it reads every record at its offset and revision
///
/// Cases: FORMAT-GOLDEN
#[test]
fn a_log_reader_opens_the_sealed_format_v2_fixture() {
    let sim = Sim::new(0);
    raw::install_segments(&sim, &sealed_fixture());
    let reader = LogReader::with_storage(Path::new("/log"), Arc::new(sim.clone())).unwrap();
    let expected = records();
    assert_eq!(fields(&reader.read(0, 100).unwrap()), fields(&expected));
    assert_eq!(reader.watermark(), expected.len() as u64);
}

/// Given a new log created with the fixture's log id
/// When it appends the first submits, reopens, appends the rest one submit at a time, and seals
/// Then its segment files equal the committed format v2 fixture byte for byte, before and after the
/// seal
///
/// Cases: FORMAT-GOLDEN
#[test]
fn the_writer_still_writes_format_v2() {
    let sim = Sim::new(0);
    let log = write(&sim);
    assert_same(&raw::segments(&sim), &open_fixture());
    log.seal().unwrap();
    assert_same(&raw::segments(&sim), &sealed_fixture());
}
