//! Entry points for the `cargo fuzz` targets in `fuzz/`. They compile only under `cfg(fuzzing)`,
//! so no release build has them. Each one panics only when it finds a bug.
//!
//! The first input byte picks a mode. In checksummed mode the entry point rewrites every frame CRC
//! before decoding, because random bytes almost never pass a CRC, and the fuzzer would otherwise
//! never reach the body checks.

use super::*;
use crate::storage::sim::{Crash, Sim};

const LOG_ID: [u8; 16] = [7; 16];

/// `FUZZ-DECODER`: the header decoder and the frame decoder on arbitrary bytes. A header or frame
/// that decodes must encode back to the same bytes.
///
/// Cases: FUZZ-DECODER
pub fn decode_frame(data: &[u8]) {
    let Some((&mode, bytes)) = data.split_first() else {
        return;
    };
    let log = codec::LogId::new(LOG_ID);
    if let Some(header) = bytes.get(..codec::HEADER_LEN)
        && let Ok(decoded) = codec::decode_header(header)
    {
        assert_eq!(codec::encode_header(&decoded), header);
    }
    let mut frame = bytes.to_vec();
    if mode & 1 == 1 && frame.len() >= codec::ENVELOPE_LEN {
        let crc = log.frame_crc(&frame);
        frame[4..8].copy_from_slice(&crc.to_le_bytes());
    }
    let Ok(decoded) = codec::decode(&log, &frame) else {
        return;
    };
    let (epoch, base) = (decoded.epoch, decoded.base_offset);
    let encoded = match decoded.body {
        codec::Body::Group(entries) => {
            let group = codec::Group {
                epoch,
                base_offset: base,
                entries,
            };
            codec::encode_group(&log, &group).unwrap()
        }
        codec::Body::Footer(footer) => codec::encode_footer(&log, epoch, base, &footer).unwrap(),
        codec::Body::Seal => codec::encode_seal(&log, epoch, base),
        codec::Body::Locator { footer_position } => {
            codec::encode_locator(&log, epoch, base, footer_position)
        }
        codec::Body::KeyFilter(filter) => {
            let hash = u64::from_le_bytes(frame[frame.len() - 8..].try_into().unwrap());
            let _ = (filter.may_contain(hash), filter.index_pages(hash));
            return;
        }
        codec::Body::KeyIndex { count, pages } => {
            for (number, page) in pages.chunks(codec::PAGE_SIZE).enumerate() {
                let _ = codec::decode_page(page, number, count, 0);
            }
            return;
        }
    };
    assert_eq!(encoded, frame);
}

/// `FUZZ-RECOVERY`: open on a log whose only segment is a valid header followed by arbitrary
/// bytes. Open either fails with an error or keeps a valid prefix. A second open of what the first
/// one left finds the same records, and changes nothing.
///
/// Cases: FUZZ-RECOVERY
pub fn recover_active_segment(data: &[u8]) {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let log = codec::LogId::new(LOG_ID);
    let header = codec::Header {
        log_id: LOG_ID,
        base_offset: 0,
        epoch: 1,
    };
    let mut segment = codec::encode_header(&header).to_vec();
    segment.extend_from_slice(rest);
    if mode & 1 == 1 {
        checksum_frames(&log, &mut segment);
    }
    let sim = Sim::new(0);
    let dir = Path::new("/log");
    let path = segment_path(dir, 0);
    for (path, op) in [
        (dir, Op::CreateDir),
        (&path, Op::Create),
        (&path, Op::Write(0, segment.clone())),
        (&path, Op::SyncAll),
        (dir, Op::SyncDir),
    ] {
        call(&sim, path, Purpose::Open, op).unwrap();
    }
    let first = match recovery::open(&sim, dir, keys::KeyHash::new) {
        Ok(recovered) => summary(recovered),
        // A frame epoch at the top of the range leaves no epoch for the next writer.
        Err(OpenError::Corrupt { .. } | OpenError::Exhausted) => return,
        Err(error) => panic!("open of one active segment failed with {error:?}"),
    };
    let files = files(&sim, dir);
    assert!(
        files[0].len() <= segment.len() && segment.starts_with(&files[0]),
        "open kept bytes that were not in the segment"
    );
    let second =
        summary(recovery::open(&sim, dir, keys::KeyHash::new).expect("a second open fails"));
    assert_eq!(first, second, "a second open finds other records");
    assert_eq!(
        files,
        self::files(&sim, dir),
        "a second open changed the files"
    );
}

type Summary = (u64, Vec<(Vec<u8>, u64, u64)>);

fn summary(recovered: recovery::Recovered) -> Summary {
    let Published {
        watermark, heads, ..
    } = recovered.published;
    let heads = heads.into_iter().map(|(s, h)| (s, h.revision, h.offset));
    (watermark, heads.collect())
}

fn files(storage: &dyn Storage, dir: &Path) -> Vec<Vec<u8>> {
    let Ok(Value::Names(mut names)) = call(storage, dir, Purpose::Open, Op::List) else {
        panic!("list the log directory");
    };
    names.sort();
    let segments = names
        .iter()
        .filter(|n| n.to_string_lossy().ends_with(".seg"));
    segments
        .map(|name| {
            let read = Op::Read(0, usize::MAX);
            match call(storage, &dir.join(name), Purpose::Open, read) {
                Ok(Value::Bytes(bytes)) => bytes,
                other => panic!("read {name:?}: {other:?}"),
            }
        })
        .collect()
}

/// Rewrites the CRC of each frame whose length fits, from the first frame until one does not.
fn checksum_frames(log: &codec::LogId, segment: &mut [u8]) {
    let mut at = codec::HEADER_LEN;
    while let Some(len) = segment.get(at..at + 4) {
        let end = at + codec::ENVELOPE_LEN + u32::from_le_bytes(len.try_into().unwrap()) as usize;
        let Some(frame) = segment.get_mut(at..end) else {
            return;
        };
        let crc = log.frame_crc(frame);
        frame[4..8].copy_from_slice(&crc.to_le_bytes());
        at = end;
    }
}

/// Reads a script from fuzz input. An exhausted input reads as zeros, which ends the script.
struct Script<'a> {
    data: &'a [u8],
    at: usize,
}

impl Script<'_> {
    fn byte(&mut self) -> u8 {
        let byte = self.data.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        byte
    }

    fn done(&self) -> bool {
        self.at >= self.data.len()
    }
}

/// What the script's acknowledged appends promise: each record at its offset.
#[derive(Default)]
struct Acked {
    records: BTreeMap<u64, Promised>,
}

#[derive(Clone, PartialEq)]
struct Promised {
    offset: u64,
    stream: Vec<u8>,
    revision: u64,
    key: Option<Vec<u8>>,
    payload: Vec<u8>,
}

impl Promised {
    fn is(&self, record: &Record) -> bool {
        (
            self.offset,
            &self.stream,
            self.revision,
            &self.key,
            &self.payload,
        ) == (
            record.offset,
            &record.stream,
            record.revision,
            &record.key,
            &record.payload,
        )
    }
}

const MAX_STEPS: usize = 64;

fn open_log(sim: &Sim, roll_bytes: u64) -> Result<SegmentLog, OpenError> {
    let options = Options {
        linger: std::time::Duration::ZERO,
        roll_bytes,
        ..Options::default()
    };
    let storage: Arc<dyn Storage> = Arc::new(sim.clone());
    // The inline log filter build keeps every run of one input the same.
    SegmentLog::start(
        Path::new("/log"),
        options,
        storage,
        keys::KeyHash::new,
        Limits::default(),
        LogFilterMode::Inline,
    )
}

fn submit(script: &mut Script<'_>, log: &SegmentLog) -> Vec<NewRecord> {
    (0..1 + script.byte() % 3)
        .map(|_| {
            let stream = vec![b'a' + script.byte() % 4];
            let key = (script.byte() % 2 == 1).then(|| vec![b'k', script.byte() % 32]);
            let payload = vec![script.byte(); usize::from(script.byte() % 40)];
            let head = log.stream_head(&stream).map(|h| h.revision);
            // Mostly the right expectation, sometimes a wrong one, so both answers occur.
            let expected = match (script.byte() % 8, head) {
                (0, _) | (_, None) => ExpectedRevision::Empty,
                (_, Some(revision)) => ExpectedRevision::At(revision),
            };
            NewRecord {
                stream,
                expected,
                key,
                payload,
            }
        })
        .collect()
}

fn append(log: &SegmentLog, records: Vec<NewRecord>, acked: &mut Acked) {
    let sent: Vec<_> = records
        .iter()
        .map(|r| (r.stream.clone(), r.key.clone(), r.payload.clone()))
        .collect();
    let Ok(pending) = log.append(Submit { records }) else {
        return;
    };
    match pending.wait() {
        Ok(committed) => {
            for ((stream, key, payload), position) in sent.into_iter().zip(&committed.records) {
                let record = Promised {
                    offset: position.offset,
                    stream,
                    revision: position.revision,
                    key,
                    payload,
                };
                match acked.records.get(&position.offset) {
                    Some(original) => assert!(
                        committed.deduplicated && *original == record,
                        "a retry returned another record's position"
                    ),
                    None => {
                        assert!(!committed.deduplicated, "a retry of an unknown record");
                        acked.records.insert(position.offset, record);
                    }
                }
            }
        }
        Err(CommitError::Indeterminate | CommitError::Poisoned) => {
            panic!("the writer failed with no storage fault")
        }
        Err(_) => {}
    }
}

/// Every acknowledged record reads back at its offset, and a retry of each keyed one returns its
/// original position without appending.
fn check(log: &SegmentLog, acked: &Acked) {
    let records: Vec<Promised> = acked.records.values().cloned().collect();
    for record in &records {
        let read = log
            .read(record.offset, 1)
            .expect("an acknowledged record reads back");
        assert!(
            read.len() == 1 && record.is(&read[0]),
            "an acknowledged record changed"
        );
    }
    let watermark = log.watermark();
    for record in records.iter().filter(|r| r.key.is_some()) {
        // A stale expectation, as a real retry carries: a key that the lookup misses then fails
        // as a new record, and this check sees it.
        let retry = NewRecord {
            stream: record.stream.clone(),
            expected: ExpectedRevision::Empty,
            key: record.key.clone(),
            payload: record.payload.clone(),
        };
        let answer = log
            .append(Submit {
                records: vec![retry],
            })
            .unwrap()
            .wait();
        let committed =
            answer.unwrap_or_else(|e| panic!("a retry of an acknowledged key got {e:?}"));
        let position = &committed.records[0];
        assert!(
            committed.deduplicated
                && (position.offset, position.revision) == (record.offset, record.revision),
            "a retry of an acknowledged key returned another position"
        );
    }
    assert_eq!(log.watermark(), watermark, "a retry appended a record");
}

fn check_reader(sim: &Sim, acked: &Acked) {
    let reader = LogReader::with_storage(Path::new("/log"), Arc::new(sim.clone()))
        .expect("a sealed log opens for reading");
    for record in acked.records.values() {
        let read = reader
            .read(record.offset, 1)
            .expect("an acknowledged record reads back");
        assert!(
            read.len() == 1 && record.is(&read[0]),
            "an acknowledged record changed"
        );
    }
}

/// `FUZZ-LOG`: the writer builds a log from a script of appends, reopens, crashes and a seal, with
/// a small roll size so that it rolls often. After every reopen, each acknowledged record reads
/// back and each keyed one deduplicates to its original position, through the sealed segments'
/// key frames. Then, if the script asks, one byte of one segment changes, with or without a valid
/// CRC, and open and every read either answer or fail with an error, never a panic.
///
/// Cases: FUZZ-LOG
pub fn build_and_recover_log(data: &[u8]) {
    let mut script = Script { data, at: 0 };
    let flags = script.byte();
    let roll_bytes = 64 + u64::from(script.byte()) * 8;
    let sim = Sim::new(u64::from(script.byte()));
    let mut acked = Acked::default();
    let mut log = Some(open_log(&sim, roll_bytes).expect("a new log opens"));
    let mut sealed = false;
    for _ in 0..MAX_STEPS {
        if script.done() {
            break;
        }
        let current = log.as_ref().unwrap();
        match script.byte() % 8 {
            0..=3 => {
                let records = submit(&mut script, current);
                append(current, records, &mut acked);
            }
            4 => {
                drop(log.take());
                let reopened = open_log(&sim, roll_bytes).expect("a clean reopen");
                check(&reopened, &acked);
                log = Some(reopened);
            }
            5 | 6 => {
                drop(log.take());
                let mode = [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn]
                    [usize::from(script.byte() % 3)];
                sim.crash(mode, u64::from(script.byte()));
                let reopened = open_log(&sim, roll_bytes).expect("a reopen after a crash");
                check(&reopened, &acked);
                log = Some(reopened);
            }
            _ => {
                current.seal().expect("seal succeeds with no storage fault");
                sealed = true;
                break;
            }
        }
    }
    drop(log);
    if sealed {
        check_reader(&sim, &acked);
    }
    if flags & 1 == 1 {
        corrupt(&mut script, &sim, flags & 2 == 2);
        reopen_damaged(&sim, roll_bytes, sealed, &acked);
    }
}

/// Changes one byte of one segment. With `checksum`, it then rewrites the CRC that covers the byte,
/// so that the change reaches the checks behind the CRC.
fn corrupt(script: &mut Script<'_>, sim: &Sim, checksum: bool) {
    let dir = Path::new("/log");
    let segments = files(sim, dir);
    let Ok(Value::Names(mut names)) = call(sim, dir, Purpose::Open, Op::List) else {
        panic!("list the log directory");
    };
    names.retain(|n| n.to_string_lossy().ends_with(".seg"));
    names.sort();
    let index = usize::from(script.byte()) % segments.len();
    let mut bytes = segments[index].clone();
    let position = (usize::from(script.byte()) << 8 | usize::from(script.byte())) % bytes.len();
    bytes[position] ^= script.byte() | 1;
    if checksum {
        rechecksum(&mut bytes, position);
    }
    let path = dir.join(&names[index]);
    call(sim, &path, Purpose::Open, Op::Write(0, bytes)).unwrap();
    call(sim, &path, Purpose::Open, Op::SyncAll).unwrap();
}

/// Rewrites the header CRC, or the CRC of the frame that holds `position`.
fn rechecksum(segment: &mut [u8], position: usize) {
    if position < codec::HEADER_LEN {
        let crc = codec::crc32c(&segment[..60]);
        segment[60..64].copy_from_slice(&crc.to_le_bytes());
        return;
    }
    let log = codec::LogId::new(segment[8..24].try_into().unwrap());
    let mut at = codec::HEADER_LEN;
    while let Some(len) = segment.get(at..at + 4) {
        let end = at + codec::ENVELOPE_LEN + u32::from_le_bytes(len.try_into().unwrap()) as usize;
        let Some(frame) = segment.get_mut(at..end) else {
            return;
        };
        if position < end {
            let crc = log.frame_crc(frame);
            frame[4..8].copy_from_slice(&crc.to_le_bytes());
            return;
        }
        at = end;
    }
}

/// Open, reads and retries on a damaged log answer or fail with an error; none panics.
fn reopen_damaged(sim: &Sim, roll_bytes: u64, sealed: bool, acked: &Acked) {
    if sealed {
        if let Ok(reader) = LogReader::with_storage(Path::new("/log"), Arc::new(sim.clone())) {
            for record in acked.records.values() {
                let _ = reader.read(record.offset, 1);
            }
        }
        return;
    }
    let log = match open_log(sim, roll_bytes) {
        Ok(log) => log,
        Err(
            OpenError::Corrupt { .. }
            | OpenError::Gap { .. }
            | OpenError::LogIdMismatch { .. }
            | OpenError::UnsupportedVersion(_)
            | OpenError::Exhausted
            | OpenError::NotSealed,
        ) => return,
        Err(error) => panic!("open of a damaged log failed with {error:?}"),
    };
    for record in acked.records.values() {
        let _ = log.read(record.offset, 1);
        if record.key.is_some() {
            let retry = NewRecord {
                stream: record.stream.clone(),
                expected: ExpectedRevision::Empty,
                key: record.key.clone(),
                payload: record.payload.clone(),
            };
            if let Ok(pending) = log.append(Submit {
                records: vec![retry],
            }) {
                let _ = pending.wait();
            }
        }
    }
}
