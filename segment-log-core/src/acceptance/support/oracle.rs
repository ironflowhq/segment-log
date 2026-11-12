use super::{fixtures::assert_records, raw};
use crate::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Input {
    pub stream: Vec<u8>,
    pub key: Option<Vec<u8>>,
    pub payload: Vec<u8>,
    pub expected: Option<u64>,
    pub spare: usize,
}
impl Input {
    pub fn new_record(&self) -> NewRecord {
        let mut payload = Vec::with_capacity(self.payload.len() + self.spare);
        payload.extend(&self.payload);
        NewRecord {
            stream: self.stream.clone(),
            key: self.key.clone(),
            payload,
            expected: self
                .expected
                .map_or(ExpectedRevision::Empty, ExpectedRevision::At),
        }
    }
    pub fn stored(&self, offset: u64, revision: u64) -> Record {
        Record {
            offset,
            revision,
            stream: self.stream.clone(),
            key: self.key.clone(),
            payload: self.payload.clone(),
        }
    }
    pub fn matches(&self, r: &Record) -> bool {
        self.stream == r.stream && self.key == r.key && self.payload == r.payload
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Ack(bool, Vec<(u64, u64)>),
    Refused(String),
    Unknown,
}
impl Answer {
    pub fn commit(result: Result<Committed, CommitError>) -> Self {
        match result {
            Ok(c) => Self::Ack(
                c.deduplicated,
                c.records.iter().map(|p| (p.offset, p.revision)).collect(),
            ),
            Err(CommitError::Indeterminate) => Self::Unknown,
            Err(e) => Self::Refused(format!("{e:?}")),
        }
    }
    pub fn admission(e: AppendError) -> Self {
        Self::Refused(match e {
            AppendError::Invalid(_) => "Invalid".into(),
            e => format!("{e:?}"),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub inputs: Vec<Input>,
    pub original: bool,
    pub answer: Answer,
    pub watermark: u64,
}
pub(crate) type HeadObservation = (Vec<u8>, Option<(u64, u64)>);
#[derive(Debug)]
pub(crate) struct Observation {
    pub before: u64,
    pub after: u64,
    pub from: u64,
    pub max: usize,
    pub records: Vec<Record>,
    pub heads: Vec<HeadObservation>,
}

pub(crate) fn submit(inputs: &[Input]) -> Submit {
    Submit {
        records: inputs.iter().map(Input::new_record).collect(),
    }
}
pub(crate) fn execute(log: &SegmentLog, inputs: &[Input]) -> Answer {
    match log.append(submit(inputs)) {
        Ok(p) => Answer::commit(p.wait()),
        Err(e) => Answer::admission(e),
    }
}

pub(crate) fn charge(inputs: &[Input]) -> Option<usize> {
    let owned = submit(inputs);
    let bytes = crate::engine::admission_support::bookkeeping()
        .checked_add(
            owned
                .records
                .capacity()
                .checked_mul(std::mem::size_of::<NewRecord>())?,
        )?
        .checked_add(
            owned
                .records
                .len()
                .checked_mul(std::mem::size_of::<Position>())?,
        )?;
    owned.records.iter().try_fold(bytes, |total, r| {
        total
            .checked_add(r.stream.capacity())?
            .checked_add(r.key.as_ref().map_or(0, Vec::capacity))?
            .checked_add(r.payload.capacity())
    })
}

fn size_eligible(inputs: &[Input], options: &Options) -> bool {
    let body = inputs.iter().try_fold(24usize, |n, r| {
        n.checked_add(16)?
            .checked_add(r.stream.len())?
            .checked_add(r.key.as_ref().map_or(0, Vec::len))?
            .checked_add(r.payload.len())
    });
    body.is_some_and(|n| n <= options.group_byte_cap)
        && charge(inputs).is_some_and(|n| n <= options.pending_byte_budget)
}

pub(crate) fn predict(inputs: &[Input], history: &mut Vec<Record>) -> Answer {
    let mut keys = BTreeSet::new();
    if inputs.is_empty()
        || inputs
            .iter()
            .any(|r| r.key.as_ref().is_some_and(|k| !keys.insert(k)))
    {
        return Answer::Refused("Invalid".into());
    }
    let mut positions = vec![];
    for (index, input) in inputs.iter().enumerate() {
        if let Some(old) = input
            .key
            .as_ref()
            .and_then(|key| history.iter().find(|r| r.key.as_ref() == Some(key)))
        {
            if !input.matches(old) {
                return Answer::Refused(format!("KeyConflict {{ index: {index} }}"));
            }
            positions.push((old.offset, old.revision));
        }
    }
    if positions.len() == inputs.len() {
        return Answer::Ack(true, positions);
    }
    if !positions.is_empty() {
        return Answer::Refused("PartialRetry".into());
    }
    let mut heads: BTreeMap<_, _> = history
        .iter()
        .map(|r| (r.stream.clone(), r.revision))
        .collect();
    positions.clear();
    for (index, input) in inputs.iter().enumerate() {
        let actual = heads.get(&input.stream).copied();
        if actual != input.expected {
            return Answer::Refused(format!(
                "WrongRevision {{ index: {index}, actual: {actual:?} }}"
            ));
        }
        let revision = match actual {
            None => 0,
            Some(n) => match n.checked_add(1) {
                Some(n) => n,
                None => return Answer::Refused("Exhausted".into()),
            },
        };
        heads.insert(input.stream.clone(), revision);
        positions.push((history.len() as u64 + index as u64, revision));
    }
    history.extend(
        inputs
            .iter()
            .zip(&positions)
            .map(|(r, &(o, v))| r.stored(o, v)),
    );
    Answer::Ack(false, positions)
}

type Identity = (Vec<u8>, Option<Vec<u8>>, Vec<u8>);
pub(crate) fn frame_inputs(log_id: &[u8; 16], frame: &[u8]) -> Vec<Identity> {
    assert!(frame.len() >= 32);
    let length = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
    assert_eq!(length + 8, frame.len());
    assert_eq!(
        raw::frame_crc(log_id, frame),
        u32::from_le_bytes(frame[4..8].try_into().unwrap())
    );
    assert_eq!(frame[8], 1);
    let count = u32::from_le_bytes(frame[28..32].try_into().unwrap());
    let mut at = 32;
    let mut out = vec![];
    for _ in 0..count {
        let s = u16::from_le_bytes(frame[at..at + 2].try_into().unwrap()) as usize;
        let k = u16::from_le_bytes(frame[at + 2..at + 4].try_into().unwrap()) as usize;
        let p = u32::from_le_bytes(frame[at + 4..at + 8].try_into().unwrap()) as usize;
        at += 16;
        let stream = frame[at..at + s].to_vec();
        at += s;
        let key = (k != 0).then(|| frame[at..at + k].to_vec());
        at += k;
        let payload = frame[at..at + p].to_vec();
        at += p;
        out.push((stream, key, payload));
    }
    assert_eq!(at, frame.len());
    out
}

fn trace_log_id(trace: &[crate::storage::sim::Event]) -> [u8; 16] {
    use crate::storage::{Op, sim::Event};
    trace
        .iter()
        .find_map(|event| match event {
            Event::Call(c, _, _) => match &c.op {
                Op::Write(0, bytes) if bytes.len() == 64 && bytes.starts_with(b"SGLG") => {
                    Some(bytes[8..24].try_into().unwrap())
                }
                _ => None,
            },
            _ => None,
        })
        .expect("the trace writes a segment header")
}

pub(crate) fn written(
    trace: &[crate::storage::sim::Event],
    requests: &[Request],
) -> (Vec<Record>, Vec<usize>) {
    use crate::storage::{Op, Purpose, sim::Event};
    let log_id = trace_log_id(trace);
    let mut history = vec![];
    let mut ends = vec![0];
    let mut seen = BTreeSet::new();
    for event in trace {
        let Event::Call(c, _, _) = event else {
            continue;
        };
        let Op::Append(bytes) = &c.op else { continue };
        if c.purpose != Purpose::Group {
            continue;
        }
        let entries = frame_inputs(&log_id, bytes);
        let mut at = 0;
        while at < entries.len() {
            let (id, request) = requests
                .iter()
                .enumerate()
                .find(|(_, q)| {
                    q.original
                        && q.inputs.first().is_some_and(|r| {
                            (&r.stream, &r.key, &r.payload)
                                == (&entries[at].0, &entries[at].1, &entries[at].2)
                        })
                })
                .expect("written input absent from original ledger");
            assert!(seen.insert(id), "original written twice");
            assert!(
                !matches!(request.answer, Answer::Refused(_)),
                "terminal refusal wrote records"
            );
            for r in &request.inputs {
                let e = &entries[at];
                assert_eq!(
                    (&r.stream, &r.key, &r.payload),
                    (&e.0, &e.1, &e.2),
                    "atomic input order"
                );
                at += 1;
            }
            assert!(
                matches!(
                    predict(&request.inputs, &mut history),
                    Answer::Ack(false, _)
                ),
                "written input violates contract"
            );
        }
        ends.push(history.len());
    }
    (history, ends)
}

pub(crate) fn history(log: &SegmentLog, expected: &[Record]) {
    assert_eq!(log.watermark(), expected.len() as u64);
    assert_records(&log.read(0, usize::MAX).unwrap(), expected);
    for r in expected {
        let last = expected
            .iter()
            .rev()
            .find(|x| x.stream == r.stream)
            .unwrap();
        assert_eq!(
            log.stream_head(&r.stream).map(|h| (h.offset, h.revision)),
            Some((last.offset, last.revision))
        );
    }
}
pub(crate) fn observations(
    requests: &[Request],
    reads: &[Observation],
    kept: &[Record],
    ends: &[usize],
) {
    for q in requests {
        assert!(
            q.watermark <= kept.len() as u64,
            "published watermark regressed"
        );
        if let Answer::Ack(_, positions) = &q.answer {
            assert_eq!(positions.len(), q.inputs.len());
            for (r, &(o, v)) in q.inputs.iter().zip(positions) {
                let old = &kept[o as usize];
                assert!(r.matches(old));
                assert_eq!(v, old.revision);
            }
        }
    }
    for pair in reads.windows(2) {
        assert!(
            pair[0].after <= pair[1].before,
            "reader watermark regressed"
        );
    }
    for read in reads {
        assert!(read.before <= read.after && read.after <= kept.len() as u64);
        assert!(ends.contains(&(read.before as usize)) && ends.contains(&(read.after as usize)));
        let snapshots: Vec<_> = ends
            .iter()
            .copied()
            .filter(|end| (*end as u64) >= read.before && (*end as u64) <= read.after)
            .collect();
        assert!(read.records.len() <= read.max);
        let start = (read.from as usize).min(kept.len());
        assert_records(&read.records, &kept[start..start + read.records.len()]);
        assert!(read.records.iter().all(|r| r.offset < read.after));
        assert!(
            snapshots.iter().any(|end| {
                ((*end as u64).saturating_sub(read.from)).min(read.max as u64) as usize
                    == read.records.len()
            }),
            "read length has no committed snapshot in its bracket"
        );
        for (stream, head) in &read.heads {
            assert!(
                snapshots.iter().any(|end| {
                    kept[..*end]
                        .iter()
                        .rev()
                        .find(|r| &r.stream == stream)
                        .map(|r| (r.offset, r.revision))
                        == *head
                }),
                "stream head has no committed snapshot in its bracket"
            );
        }
    }
}

/// The number of records that recovery must keep: the valid prefix across every segment. Each
/// segment before the newest must be whole and start where the one before it ends. A newest file no
/// longer than a header, whose header checksum does not match, holds no record.
pub(crate) fn valid_length(segments: &[(u64, Vec<u8>)]) -> usize {
    let mut count = 0;
    let mut epoch = 0;
    for (index, (base, bytes)) in segments.iter().enumerate() {
        let newest = index + 1 == segments.len();
        let header_valid = bytes.len() >= 64
            && raw::crc(&bytes[..60]) == u32::from_le_bytes(bytes[60..64].try_into().unwrap());
        if newest && bytes.len() <= 64 && !header_valid {
            break;
        }
        assert!(header_valid, "segment {base} has a valid header");
        assert_eq!(
            *base, count as u64,
            "segment {base} starts where the one before ends"
        );
        epoch = epoch.max(u64::from_le_bytes(bytes[32..40].try_into().unwrap()));
        let log_id: [u8; 16] = bytes[8..24].try_into().unwrap();
        let mut at = 64;
        while at + 32 <= bytes.len() {
            let len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            if !(24..=8 * 1024 * 1024).contains(&len) || at + 8 + len > bytes.len() {
                break;
            }
            let frame = &bytes[at..at + 8 + len];
            let crc = u32::from_le_bytes(frame[4..8].try_into().unwrap());
            let next_epoch = u64::from_le_bytes(frame[12..20].try_into().unwrap());
            let frame_base = u64::from_le_bytes(frame[20..28].try_into().unwrap());
            if raw::frame_crc(&log_id, frame) != crc
                || frame[8..12] != [1, 0, 0, 0]
                || next_epoch < epoch
                || frame_base != count as u64
            {
                break;
            }
            count += frame_inputs(&log_id, frame).len();
            epoch = next_epoch;
            at += 8 + len;
        }
        assert!(
            newest || at == bytes.len(),
            "sealed segment {base} is whole"
        );
    }
    count
}

/// ORACLE-6: recovery leaves every segment that had a successor byte for byte as it found it.
pub(crate) fn sealed_unchanged(before: &[(u64, Vec<u8>)], after: &[(u64, Vec<u8>)]) {
    let sealed = &before[..before.len().saturating_sub(1)];
    assert_eq!(
        after.get(..sealed.len()),
        Some(sealed),
        "recovery changed a sealed segment"
    );
}

pub(crate) fn answers(
    requests: &[Request],
    events: &[crate::storage::sim::Event],
    options: &Options,
) {
    use crate::storage::sim::Event;
    let faulted = events
        .iter()
        .any(|e| matches!(e, Event::Call(_, Some(_), _) | Event::Canceled(_)));
    let (written, _) = written(events, requests);
    let mut pending_view = vec![];
    for q in requests {
        match &q.answer {
            Answer::Refused(s) if s == "TooLarge" => {
                assert!(
                    !size_eligible(&q.inputs, options),
                    "false permanent size refusal: {q:?}"
                );
                continue;
            }
            Answer::Refused(s) if s == "Full" => {
                assert!(
                    size_eligible(&q.inputs, options),
                    "impossible size reported as Full: {q:?}"
                );
                continue;
            }
            Answer::Refused(s) if s == "Poisoned" => {
                assert!(faulted);
                continue;
            }
            answer => {
                let mut predicted = predict(&q.inputs, &mut pending_view);
                if q.original && matches!(answer, Answer::Ack(false, _)) {
                    predicted = Answer::Ack(
                        false,
                        q.inputs
                            .iter()
                            .map(|i| {
                                let r = written
                                    .iter()
                                    .find(|r| i.matches(r))
                                    .expect("ack original missing from writes");
                                (r.offset, r.revision)
                            })
                            .collect(),
                    );
                }
                if *answer != Answer::Unknown {
                    assert_eq!(
                        *answer, predicted,
                        "recorded answer violates input contract: {q:?}"
                    );
                }
            }
        }
    }
}

pub(crate) fn flush_contract(
    requests: &[Request],
    reads: &[Observation],
    events: &[crate::storage::sim::Event],
) {
    use crate::storage::{
        Op, Purpose, Value,
        sim::{Event, Outcome},
    };
    let log_id = trace_log_id(events);
    let mut attempted_end = 0;
    let mut durable_end = 0;
    let mut failed = false;
    for e in events {
        match e {
            Event::Canceled(_) => failed = true,
            Event::Call(c, fault, result)
                if matches!(c.purpose, Purpose::Group | Purpose::Lookup | Purpose::Roll) =>
            {
                if !matches!(c.op, Op::Read(..)) {
                    assert!(!failed, "failed writer performed more I/O");
                }
                match (c.purpose, &c.op, result) {
                    (Purpose::Group, Op::Append(bytes), _) => {
                        attempted_end += frame_inputs(&log_id, bytes).len()
                    }
                    (Purpose::Group, Op::SyncData, Outcome::Ok(Value::Unit)) => {
                        durable_end = attempted_end
                    }
                    _ => {}
                }
                if fault.is_some() {
                    failed = true;
                }
            }
            _ => {}
        }
    }
    for q in requests {
        assert!(
            q.watermark <= durable_end as u64,
            "watermark passed successful data flush"
        );
        if let Answer::Ack(_, ps) = &q.answer {
            assert!(
                ps.iter().all(|(o, _)| *o < durable_end as u64),
                "ack without successful data flush"
            );
        }
    }
    for r in reads {
        assert!(
            r.after <= durable_end as u64,
            "reader passed successful data flush"
        );
    }
}
