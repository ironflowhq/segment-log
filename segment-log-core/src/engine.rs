use crate::{
    codec,
    storage::{self, Call, Lock, Op, Purpose, Storage, Value},
    *,
};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
};
mod keys;
mod recovery;
use keys::{Classification, Keys};

struct Writer {
    epoch: u64,
    keys: Keys,
    lookup: Vec<u8>,
}

#[derive(Clone)]
struct Frame {
    position: u64,
    length: usize,
    base: u64,
    end: u64,
    epoch: u64,
}
#[derive(Default)]
struct Published {
    frames: Vec<Frame>,
    heads: BTreeMap<Vec<u8>, StreamHead>,
    watermark: u64,
}
struct Reservation {
    bytes: usize,
    used: Arc<std::sync::atomic::AtomicUsize>,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.used
            .fetch_sub(self.bytes, std::sync::atomic::Ordering::AcqRel);
    }
}
struct Request {
    submit: Submit,
    reply: mpsc::Sender<Result<Committed, CommitError>>,
    reservation: Reservation,
}
struct RetryWaiter {
    request: Request,
    positions: Vec<Position>,
}
const REQUEST_BOOKKEEPING: usize =
    std::mem::size_of::<RetryWaiter>() + std::mem::size_of::<Pending>();
impl Request {
    fn answer(self, result: Result<Committed, CommitError>) {
        drop(self.reservation);
        let _ = self.reply.send(result);
    }
}
pub struct Pending {
    reply: mpsc::Receiver<Result<Committed, CommitError>>,
}
impl Pending {
    pub fn wait(self) -> Result<Committed, CommitError> {
        self.reply.recv().unwrap_or(Err(CommitError::Indeterminate))
    }
    #[cfg(test)]
    pub(crate) fn probe(&self) -> Option<Result<Committed, CommitError>> {
        match self.reply.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(CommitError::Indeterminate)),
        }
    }
}
pub struct SegmentLog {
    storage: Arc<dyn Storage>,
    segment: PathBuf,
    published: Arc<Mutex<Published>>,
    sender: Option<mpsc::SyncSender<Request>>,
    writer: Option<JoinHandle<()>>,
    poisoned: Arc<std::sync::atomic::AtomicBool>,
    options: Arc<Options>,
    used: Arc<std::sync::atomic::AtomicUsize>,
    _lock: Mutex<Box<dyn Lock>>,
}
impl SegmentLog {
    pub fn open(dir: impl AsRef<Path>, options: Options) -> Result<Self, OpenError> {
        Self::with_storage(
            dir.as_ref(),
            options,
            Arc::new(storage::real::Real::default()),
        )
    }
    pub(crate) fn with_storage(
        dir: &Path,
        options: Options,
        storage: Arc<dyn Storage>,
    ) -> Result<Self, OpenError> {
        if options.queue_capacity == 0 || options.pending_byte_budget == 0 {
            return Err(OpenError::InvalidOptions("capacities must be positive"));
        }
        if !(41..=8 * 1024 * 1024).contains(&options.group_byte_cap) {
            return Err(OpenError::InvalidOptions("group byte cap outside limits"));
        }
        if options.roll_bytes == 0 || options.roll_bytes > 4000 * 1024 * 1024 {
            return Err(OpenError::InvalidOptions("roll bytes outside limits"));
        }
        let options = Arc::new(options);
        let recovered = recovery::open(storage.as_ref(), dir)?;
        let segment = recovered.segment;
        let published = Arc::new(Mutex::new(recovered.published));
        let poisoned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(options.queue_capacity);
        let writer_storage = storage.clone();
        let writer_segment = segment.clone();
        let writer_published = published.clone();
        let writer_poisoned = poisoned.clone();
        let writer_options = options.clone();
        let writer = thread::Builder::new()
            .name("segment-log-writer".into())
            .spawn(move || {
                run(
                    writer_storage,
                    writer_segment,
                    writer_published,
                    writer_poisoned,
                    receiver,
                    Writer {
                        epoch: recovered.epoch,
                        keys: recovered.keys,
                        lookup: Vec::new(),
                    },
                    writer_options,
                );
            })
            .map_err(OpenError::Io)?;
        Ok(Self {
            storage,
            segment,
            published,
            sender: Some(sender),
            writer: Some(writer),
            poisoned,
            options,
            used: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            _lock: Mutex::new(recovered.lock),
        })
    }
    #[cfg(test)]
    pub(crate) fn seed_counters(&self, watermark: u64, heads: &[(&[u8], u64)]) {
        let mut state = self.published.lock().unwrap();
        assert!(state.frames.is_empty());
        state.watermark = watermark;
        for (stream, revision) in heads {
            state.heads.insert(
                stream.to_vec(),
                StreamHead {
                    revision: *revision,
                    offset: watermark.saturating_sub(1),
                },
            );
        }
    }
    pub fn append(&self, submit: Submit) -> Result<Pending, AppendError> {
        validate(&submit)?;
        if body_size(&submit).ok_or(AppendError::TooLarge)? > self.options.group_byte_cap {
            return Err(AppendError::TooLarge);
        }
        let bytes = charge(&submit).ok_or(AppendError::TooLarge)?;
        if bytes > self.options.pending_byte_budget {
            return Err(AppendError::TooLarge);
        }
        if self.poisoned.load(std::sync::atomic::Ordering::Acquire) {
            return Err(AppendError::Poisoned);
        }
        self.used
            .try_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |n| {
                    n.checked_add(bytes)
                        .filter(|n| *n <= self.options.pending_byte_budget)
                },
            )
            .map_err(|_| AppendError::Full)?;
        let reservation = Reservation {
            bytes,
            used: self.used.clone(),
        };
        let (reply, receiver) = mpsc::channel();
        match self.sender.as_ref().unwrap().try_send(Request {
            submit,
            reply,
            reservation,
        }) {
            Ok(()) => Ok(Pending { reply: receiver }),
            Err(mpsc::TrySendError::Full(_)) => Err(AppendError::Full),
            Err(mpsc::TrySendError::Disconnected(_)) => Err(AppendError::Poisoned),
        }
    }
    pub fn watermark(&self) -> LogOffset {
        self.published.lock().unwrap().watermark
    }
    pub fn stream_head(&self, stream: &[u8]) -> Option<StreamHead> {
        self.published
            .lock()
            .unwrap()
            .heads
            .get(stream)
            .map(|h| StreamHead {
                revision: h.revision,
                offset: h.offset,
            })
    }
    pub fn read(&self, from: LogOffset, max: usize) -> Result<Vec<Record>, ReadError> {
        let state = self.published.lock().unwrap();
        if max == 0 || from >= state.watermark {
            return Ok(vec![]);
        }
        let frames = state.frames.clone();
        drop(state);
        let mut records = Vec::new();
        for frame in frames.into_iter().filter(|f| f.end > from) {
            let bytes = read_bytes(
                self.storage.as_ref(),
                &self.segment,
                Purpose::Lookup,
                frame.position,
                frame.length,
            )
            .map_err(ReadError::Io)?;
            let group = codec::decode(&bytes).map_err(|_| ReadError::Corrupt {
                segment: 0,
                position: frame.position,
            })?;
            if group.base_offset != frame.base
                || group.base_offset + group.entries.len() as u64 != frame.end
            {
                return Err(ReadError::Corrupt {
                    segment: 0,
                    position: frame.position,
                });
            }
            for (i, entry) in group.entries.into_iter().enumerate() {
                let offset = group.base_offset + i as u64;
                if offset >= from {
                    records.push(Record {
                        offset,
                        stream: entry.stream.to_vec(),
                        revision: entry.revision,
                        key: entry.key.map(<[u8]>::to_vec),
                        payload: entry.payload.to_vec(),
                    });
                    if records.len() == max {
                        return Ok(records);
                    }
                }
            }
        }
        Ok(records)
    }
}
impl Drop for SegmentLog {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}
fn call(storage: &dyn Storage, path: &Path, purpose: Purpose, op: Op) -> io::Result<Value> {
    storage.call(Call::new(path, purpose, op))
}
fn read_bytes(
    storage: &dyn Storage,
    path: &Path,
    purpose: Purpose,
    position: u64,
    length: usize,
) -> io::Result<Vec<u8>> {
    match call(storage, path, purpose, Op::Read(position, length))? {
        Value::Bytes(bytes) => Ok(bytes),
        _ => Err(io::Error::other("storage did not return bytes")),
    }
}
fn publish(published: &mut Published, frame: Frame, group: &codec::Group<'_>) {
    for (i, entry) in group.entries.iter().enumerate() {
        published.heads.insert(
            entry.stream.to_vec(),
            StreamHead {
                revision: entry.revision,
                offset: group.base_offset + i as u64,
            },
        );
    }
    published.watermark = frame.end;
    published.frames.push(frame);
}
fn validate(submit: &Submit) -> Result<(), AppendError> {
    if submit.records.is_empty() {
        return Err(AppendError::Invalid("empty submit"));
    }
    let mut keys = std::collections::BTreeSet::new();
    for record in &submit.records {
        if !(1..=1024).contains(&record.stream.len()) {
            return Err(AppendError::Invalid("stream length outside limits"));
        }
        if let Some(key) = &record.key {
            if !(1..=256).contains(&key.len()) {
                return Err(AppendError::Invalid("key length outside limits"));
            }
            if !keys.insert(key.as_slice()) {
                return Err(AppendError::Invalid("repeated key"));
            }
        }
    }
    Ok(())
}
fn body_size(submit: &Submit) -> Option<usize> {
    submit.records.iter().try_fold(24usize, |size, r| {
        size.checked_add(16 + r.stream.len() + r.key.as_ref().map_or(0, Vec::len))?
            .checked_add(r.payload.len())
    })
}
struct RevisionSlot {
    index: usize,
    revision: Revision,
}
// Classification reuses the result-position allowance before any result is allocated.
const _: () = assert!(std::mem::size_of::<RevisionSlot>() <= std::mem::size_of::<Position>());
fn revisions(
    submit: &Submit,
    heads: &BTreeMap<Vec<u8>, u64>,
) -> Result<Box<[RevisionSlot]>, CommitError> {
    let mut slots: Box<[_]> = (0..submit.records.len())
        .map(|index| RevisionSlot { index, revision: 0 })
        .collect();
    slots.sort_unstable_by(|a, b| {
        submit.records[a.index]
            .stream
            .cmp(&submit.records[b.index].stream)
            .then(a.index.cmp(&b.index))
    });
    let mut cursor = 0;
    let mut failure: Option<(usize, CommitError)> = None;
    while cursor < slots.len() {
        let stream = &submit.records[slots[cursor].index].stream;
        let mut actual = heads.get(stream).copied();
        while cursor < slots.len() && submit.records[slots[cursor].index].stream == *stream {
            let index = slots[cursor].index;
            let matches = match submit.records[index].expected {
                ExpectedRevision::Empty => actual.is_none(),
                ExpectedRevision::At(n) => actual == Some(n),
            };
            let result = if !matches {
                Err(CommitError::WrongRevision { index, actual })
            } else {
                match actual {
                    None => Ok(0),
                    Some(n) => n.checked_add(1).ok_or(CommitError::Exhausted),
                }
            };
            let revision = match result {
                Ok(revision) => revision,
                Err(error) => {
                    if failure.as_ref().is_none_or(|(earlier, _)| index < *earlier) {
                        failure = Some((index, error));
                    }
                    while cursor < slots.len()
                        && submit.records[slots[cursor].index].stream == *stream
                    {
                        cursor += 1;
                    }
                    break;
                }
            };
            slots[cursor].revision = revision;
            actual = Some(revision);
            cursor += 1;
        }
    }
    if let Some((_, error)) = failure {
        return Err(error);
    }
    slots.sort_unstable_by_key(|slot| slot.index);
    Ok(slots)
}

fn run(
    storage: Arc<dyn Storage>,
    segment: PathBuf,
    published: Arc<Mutex<Published>>,
    poisoned: Arc<std::sync::atomic::AtomicBool>,
    receiver: mpsc::Receiver<Request>,
    mut writer: Writer,
    options: Arc<Options>,
) {
    let mut deferred = None;
    loop {
        let first = match deferred.take().or_else(|| receiver.recv().ok()) {
            Some(r) => r,
            None => return,
        };
        let snapshot = published.lock().unwrap();
        let base = snapshot.watermark;
        let mut heads: BTreeMap<_, _> = snapshot
            .heads
            .iter()
            .map(|(k, v)| (k.clone(), v.revision))
            .collect();
        let position = snapshot
            .frames
            .last()
            .map_or(64, |f| f.position + f.length as u64);
        drop(snapshot);
        let started = std::time::Instant::now();
        let mut requests = Vec::new();
        let mut assigned = Vec::new();
        let mut waiters = Vec::new();
        let mut size = 24;
        let mut next = Some(first);
        while let Some(request) = next.take() {
            match writer.keys.classify(
                storage.as_ref(),
                &segment,
                &request.submit,
                &mut writer.lookup,
                &requests,
            ) {
                Classification::New => match revisions(&request.submit, &heads) {
                    Err(error) => {
                        request.answer(Err(error));
                    }
                    Ok(revisions) => {
                        let added = body_size(&request.submit).unwrap() - 24;
                        if size + added > options.group_byte_cap {
                            deferred = Some(request);
                            break;
                        }
                        if base
                            .checked_add(assigned.len() as u64)
                            .and_then(|next| next.checked_add(request.submit.records.len() as u64))
                            .is_none()
                        {
                            request.answer(Err(CommitError::Exhausted));
                        } else {
                            writer.keys.stage(
                                requests.len(),
                                &request.submit,
                                base + assigned.len() as u64,
                                &revisions,
                            );
                            size += added;
                            for (record, revision) in request.submit.records.iter().zip(&revisions)
                            {
                                heads.insert(record.stream.clone(), revision.revision);
                            }
                            assigned.extend(revisions.iter().map(|slot| slot.revision));
                            requests.push(request);
                        }
                    }
                },
                Classification::DurableRetry(records) => {
                    request.answer(Ok(Committed {
                        records,
                        deduplicated: true,
                    }));
                }
                Classification::PendingRetry(positions) => {
                    waiters.push(RetryWaiter { request, positions });
                }
                Classification::Rejected(error) => {
                    request.answer(Err(error));
                }
                Classification::LookupFailure => {
                    requests.push(request);
                    poison(&poisoned, requests, waiters, deferred.take(), receiver);
                    return;
                }
            }
            next = if requests.is_empty() || options.linger.is_zero() {
                receiver.try_recv().ok()
            } else {
                let remaining = options.linger.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    None
                } else {
                    receiver.recv_timeout(remaining).ok()
                }
            };
        }
        if requests.is_empty() {
            continue;
        }
        let entries = requests
            .iter()
            .flat_map(|r| &r.submit.records)
            .zip(&assigned)
            .map(|(r, revision)| codec::Entry {
                stream: &r.stream,
                key: r.key.as_deref(),
                payload: &r.payload,
                revision: *revision,
            })
            .collect();
        let group = codec::Group {
            epoch: writer.epoch,
            base_offset: base,
            entries,
        };
        let bytes = codec::encode(&group).expect("validated group");
        let length = bytes.len();
        let result = call(
            storage.as_ref(),
            &segment,
            Purpose::Group,
            Op::Append(bytes),
        )
        .and_then(|_| call(storage.as_ref(), &segment, Purpose::Group, Op::SyncData));
        if result.is_err() {
            poison(&poisoned, requests, waiters, deferred.take(), receiver);
            return;
        }
        let frame = Frame {
            position,
            length,
            base,
            end: base + assigned.len() as u64,
            epoch: writer.epoch,
        };
        writer.keys.publish(&frame, &group);
        publish(&mut published.lock().unwrap(), frame, &group);
        let mut index = 0;
        for request in requests {
            let records = (0..request.submit.records.len())
                .map(|_| {
                    let p = Position {
                        offset: base + index as u64,
                        revision: assigned[index],
                    };
                    index += 1;
                    p
                })
                .collect();
            request.answer(Ok(Committed {
                records,
                deduplicated: false,
            }));
        }
        for waiter in waiters {
            waiter.request.answer(Ok(Committed {
                records: waiter.positions,
                deduplicated: true,
            }));
        }
    }
}

fn charge(submit: &Submit) -> Option<usize> {
    let base = charge_base(submit.records.capacity(), submit.records.len())?;
    submit.records.iter().try_fold(base, |n, r| {
        charge_buffers(
            n,
            r.stream.capacity(),
            r.key.as_ref().map_or(0, Vec::capacity),
            r.payload.capacity(),
        )
    })
}

fn charge_base(capacity: usize, count: usize) -> Option<usize> {
    REQUEST_BOOKKEEPING
        .checked_add(capacity.checked_mul(std::mem::size_of::<NewRecord>())?)?
        .checked_add(count.checked_mul(std::mem::size_of::<Position>())?)
}

fn charge_buffers(base: usize, stream: usize, key: usize, payload: usize) -> Option<usize> {
    base.checked_add(stream)?
        .checked_add(key)?
        .checked_add(payload)
}

fn poison(
    poisoned: &std::sync::atomic::AtomicBool,
    requests: Vec<Request>,
    waiters: Vec<RetryWaiter>,
    deferred: Option<Request>,
    receiver: mpsc::Receiver<Request>,
) {
    poisoned.store(true, std::sync::atomic::Ordering::Release);
    for request in requests {
        request.answer(Err(CommitError::Indeterminate));
    }
    for waiter in waiters {
        waiter.request.answer(Err(CommitError::Indeterminate));
    }
    if let Some(request) = deferred {
        request.answer(Err(CommitError::Poisoned));
    }
    for request in receiver {
        request.answer(Err(CommitError::Poisoned));
    }
}

#[cfg(test)]
pub(crate) mod admission_support {
    use super::*;
    pub(crate) fn bookkeeping() -> usize {
        std::mem::size_of::<RetryWaiter>() + std::mem::size_of::<Pending>()
    }
    pub(crate) fn used(log: &SegmentLog) -> usize {
        log.used.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) fn finish(log: &mut SegmentLog, panicked: bool) {
        if !panicked {
            log.sender.take();
        }
        assert_eq!(log.writer.take().unwrap().join().is_err(), panicked);
    }
    pub(crate) fn base(capacity: usize, count: usize) -> Option<usize> {
        charge_base(capacity, count)
    }
    pub(crate) fn buffers(base: usize, stream: usize, key: usize, payload: usize) -> Option<usize> {
        charge_buffers(base, stream, key, payload)
    }
}
