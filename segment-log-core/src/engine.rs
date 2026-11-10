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
    time::Instant,
};
mod keys;
mod recovery;
mod reply;
use keys::{Classification, Keys};
pub use reply::Pending;

struct Writer {
    epoch: u64,
    keys: Keys,
    lookup: Vec<u8>,
    active: Active,
}

/// The segment that takes new frames. Only the writer thread appends to it.
struct Active {
    base: u64,
    length: u64,
}

fn segment_path(dir: &Path, base: u64) -> PathBuf {
    dir.join(format!("{base:020}.seg"))
}

#[derive(Clone)]
struct Frame {
    /// Base offset of the segment file that holds the frame.
    segment: u64,
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
    reply: reply::Reply,
    reservation: Reservation,
    /// When `append` queued the request. A group's linger runs from this instant of its first submit.
    admitted: Instant,
}
struct RetryWaiter {
    request: Request,
    positions: Vec<Position>,
}
const REQUEST_BOOKKEEPING: usize = std::mem::size_of::<RetryWaiter>()
    + std::mem::size_of::<Pending>()
    + std::mem::size_of::<reply::Slot>();
impl Request {
    fn answer(self, result: Result<Committed, CommitError>) {
        #[cfg(test)]
        crate::pipeline::request(&self.submit, crate::pipeline::SEND);
        drop(self.reservation);
        self.reply.send(result);
    }
    /// Answers without waking the caller. The group's relay wakes it.
    fn store(self, result: Result<Committed, CommitError>) -> Arc<reply::Slot> {
        #[cfg(test)]
        crate::pipeline::request(&self.submit, crate::pipeline::SEND);
        drop(self.reservation);
        self.reply.store(result)
    }
}
pub struct SegmentLog {
    storage: Arc<dyn Storage>,
    dir: PathBuf,
    published: Arc<Mutex<Published>>,
    sender: Option<mpsc::SyncSender<Request>>,
    writer: Option<JoinHandle<()>>,
    poisoned: Arc<std::sync::atomic::AtomicBool>,
    options: Arc<Options>,
    used: Arc<std::sync::atomic::AtomicUsize>,
    _lock: Mutex<Box<dyn Lock>>,
    #[cfg(test)]
    peak: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    drop_closed: Option<mpsc::Sender<()>>,
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
        let dir = dir.to_owned();
        let published = Arc::new(Mutex::new(recovered.published));
        let poisoned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(options.queue_capacity);
        let writer_storage = storage.clone();
        let writer_dir = dir.clone();
        let writer_published = published.clone();
        let writer_poisoned = poisoned.clone();
        let writer_options = options.clone();
        let writer = thread::Builder::new()
            .name("segment-log-writer".into())
            .spawn(move || {
                run(
                    writer_storage,
                    writer_dir,
                    writer_published,
                    writer_poisoned,
                    receiver,
                    Writer {
                        epoch: recovered.epoch,
                        keys: recovered.keys,
                        lookup: Vec::new(),
                        active: recovered.active,
                    },
                    writer_options,
                );
            })
            .map_err(OpenError::Io)?;
        Ok(Self {
            storage,
            dir,
            published,
            sender: Some(sender),
            writer: Some(writer),
            poisoned,
            options,
            used: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            _lock: Mutex::new(recovered.lock),
            #[cfg(test)]
            peak: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            drop_closed: None,
        })
    }
    #[cfg(test)]
    pub(crate) fn observe_drop(&mut self) -> mpsc::Receiver<()> {
        let (sender, receiver) = mpsc::channel();
        self.drop_closed = Some(sender);
        receiver
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
        #[cfg(test)]
        crate::pipeline::request(&submit, crate::pipeline::ADMISSION);
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
        let _previous = self
            .used
            .try_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |n| {
                    n.checked_add(bytes)
                        .filter(|n| *n <= self.options.pending_byte_budget)
                },
            )
            .map_err(|_| AppendError::Full)?;
        #[cfg(test)]
        self.peak
            .fetch_max(_previous + bytes, std::sync::atomic::Ordering::Relaxed);
        let reservation = Reservation {
            bytes,
            used: self.used.clone(),
        };
        let (reply, pending) = reply::pair();
        #[cfg(test)]
        crate::pipeline::request(&submit, crate::pipeline::ENQUEUE);
        match self.sender.as_ref().unwrap().try_send(Request {
            submit,
            reply,
            reservation,
            admitted: Instant::now(),
        }) {
            Ok(()) => Ok(pending),
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
        let first = state.frames.partition_point(|frame| frame.end <= from);
        let end = from.saturating_add(max as u64);
        let last = state.frames.partition_point(|frame| frame.base < end);
        let frames = state.frames[first..last].to_vec();
        drop(state);
        let mut records = Vec::new();
        for frame in frames {
            let bytes = read_bytes(
                self.storage.as_ref(),
                &segment_path(&self.dir, frame.segment),
                Purpose::Lookup,
                frame.position,
                frame.length,
            )
            .map_err(ReadError::Io)?;
            let corrupt = || ReadError::Corrupt {
                segment: frame.segment,
                position: frame.position,
            };
            let group = codec::decode(&bytes).map_err(|_| corrupt())?;
            if group.base_offset != frame.base
                || group.base_offset + group.entries.len() as u64 != frame.end
            {
                return Err(corrupt());
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
        #[cfg(test)]
        if let Some(observer) = self.drop_closed.take() {
            let _ = observer.send(());
        }
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
/// `pending` holds the revisions that the current group has assigned. It overrides the
/// durable heads, which the group has not changed yet.
fn revisions(
    submit: &Submit,
    pending: &BTreeMap<Vec<u8>, u64>,
    published: &Mutex<Published>,
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
    // Only the writer thread changes the durable heads, and it does so between groups.
    let durable = published.lock().unwrap();
    while cursor < slots.len() {
        let stream = &submit.records[slots[cursor].index].stream;
        let mut actual = pending
            .get(stream)
            .copied()
            .or_else(|| durable.heads.get(stream).map(|head| head.revision));
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
    dir: PathBuf,
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
        #[cfg(test)]
        crate::pipeline::request(&first.submit, crate::pipeline::DEQUEUE);
        let base = published.lock().unwrap().watermark;
        let segment = segment_path(&dir, writer.active.base);
        // Copying every durable head here would make each group cost grow with the stream count.
        let mut heads = BTreeMap::new();
        let mut requests = Vec::new();
        let mut assigned = Vec::new();
        let mut origins = Vec::new();
        let mut waiters = Vec::new();
        let mut size = 24;
        // When the writer first saw this group's linger over.
        let mut gathered = None;
        let mut next = Some(first);
        while let Some(request) = next.take() {
            #[cfg(test)]
            crate::pipeline::request(&request.submit, crate::pipeline::CLASSIFY);
            match writer.keys.classify(
                storage.as_ref(),
                &dir,
                &request.submit,
                &mut writer.lookup,
                &keys::Staged {
                    base,
                    requests: &requests,
                    origins: &origins,
                },
                &published,
            ) {
                Classification::New => match revisions(&request.submit, &heads, &published) {
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
                                &request.submit,
                                base + assigned.len() as u64,
                                &revisions,
                            );
                            origins.extend(
                                (0..request.submit.records.len())
                                    .map(|record| (requests.len(), record)),
                            );
                            size += added;
                            for (record, revision) in request.submit.records.iter().zip(&revisions)
                            {
                                heads.insert(record.stream.clone(), revision.revision);
                            }
                            assigned.extend(revisions.iter().map(|slot| slot.revision));
                            #[cfg(test)]
                            crate::pipeline::request(&request.submit, crate::pipeline::READY);
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
            next = match requests.first() {
                // No new submit has joined yet, so no linger runs.
                None => receiver.try_recv().ok(),
                Some(first) => {
                    // The linger runs from the admission of the group's first submit, so the time
                    // that submit spent queued behind the previous flush counts (ADR 0046).
                    let remaining = options.linger.saturating_sub(first.admitted.elapsed());
                    if remaining.is_zero() {
                        // The group takes the submits admitted before the writer first saw the
                        // linger over. Without that limit, steady arrivals keep one group
                        // gathering until the byte cap.
                        let limit = *gathered.get_or_insert_with(Instant::now);
                        match receiver.try_recv().ok() {
                            Some(late) if late.admitted > limit => {
                                deferred = Some(late);
                                None
                            }
                            queued => queued,
                        }
                    } else {
                        receiver.recv_timeout(remaining).ok()
                    }
                }
            };
            #[cfg(test)]
            if let Some(request) = &next {
                crate::pipeline::request(&request.submit, crate::pipeline::DEQUEUE);
            }
        }
        if requests.is_empty() {
            continue;
        }
        #[cfg(test)]
        let mut timeline = crate::pipeline::Group::new(base, requests.len());
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
        #[cfg(test)]
        timeline.mark(1);
        let result = call(
            storage.as_ref(),
            &segment,
            Purpose::Group,
            Op::Append(bytes),
        )
        .and_then(|_| {
            #[cfg(test)]
            timeline.mark(2);
            call(storage.as_ref(), &segment, Purpose::Group, Op::SyncData)
        });
        #[cfg(test)]
        timeline.mark(3);
        if result.is_err() {
            poison(&poisoned, requests, waiters, deferred.take(), receiver);
            return;
        }
        let frame = Frame {
            segment: writer.active.base,
            position: writer.active.length,
            length,
            base,
            end: base + assigned.len() as u64,
            epoch: writer.epoch,
        };
        writer.active.length += length as u64;
        publish(&mut published.lock().unwrap(), frame, &group);
        #[cfg(test)]
        timeline.mark(4);
        let mut index = 0;
        let mut slots = Vec::with_capacity(requests.len() + waiters.len());
        for request in requests {
            #[cfg(test)]
            crate::pipeline::group_id(&request.submit, base);
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
            slots.push(request.store(Ok(Committed {
                records,
                deduplicated: false,
            })));
        }
        for waiter in waiters {
            slots.push(waiter.request.store(Ok(Committed {
                records: waiter.positions,
                deduplicated: true,
            })));
        }
        reply::Relay::start(slots);
        #[cfg(test)]
        timeline.finish();
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
        std::mem::size_of::<RetryWaiter>()
            + std::mem::size_of::<Pending>()
            + std::mem::size_of::<reply::Slot>()
    }
    pub(crate) fn peak(log: &SegmentLog) -> usize {
        log.peak.load(std::sync::atomic::Ordering::Relaxed)
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
