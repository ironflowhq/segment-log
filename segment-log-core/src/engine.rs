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
mod closing;
mod keys;
mod recovery;
mod reply;
mod survey;
mod view;
mod writer;
use keys::Keys;
pub use reply::Pending;
use view::{GroupSpan, Published, View};
use writer::{Active, RevisionSlot, Writer, run};

fn segment_path(dir: &Path, base: u64) -> PathBuf {
    dir.join(format!("{base:020}.seg"))
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
    view: Arc<View>,
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
        if !(41..=codec::GROUP_BODY_LIMIT).contains(&options.group_byte_cap) {
            return Err(OpenError::InvalidOptions("group byte cap outside limits"));
        }
        if options.roll_bytes == 0 || options.roll_bytes > codec::ROLL_BYTES_LIMIT {
            return Err(OpenError::InvalidOptions("roll bytes outside limits"));
        }
        let options = Arc::new(options);
        let recovered = recovery::open(storage.as_ref(), dir)?;
        let view = Arc::new(View {
            storage,
            dir: dir.to_owned(),
            log_id: recovered.log_id,
            published: Mutex::new(recovered.published),
        });
        let poisoned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(options.queue_capacity);
        let writer_view = view.clone();
        let writer_poisoned = poisoned.clone();
        let writer_options = options.clone();
        let writer = thread::Builder::new()
            .name("segment-log-writer".into())
            .spawn(move || {
                run(
                    writer_view,
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
            view,
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
        let mut state = self.view.published.lock().unwrap();
        assert!(state.sealed.is_empty() && state.newest.spans.is_empty());
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
        self.view.watermark()
    }
    pub fn stream_head(&self, stream: &[u8]) -> Option<StreamHead> {
        self.view.stream_head(stream)
    }
    pub fn read(&self, from: LogOffset, max: usize) -> Result<Vec<Record>, ReadError> {
        self.view.read(from, max)
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
/// Creates a segment that holds only its header. Returns after the file and its directory
/// entry are durable, so that a frame is never appended to a segment that a crash can remove.
fn create_segment(
    storage: &dyn Storage,
    dir: &Path,
    purpose: Purpose,
    header: &codec::Header,
) -> io::Result<()> {
    let path = segment_path(dir, header.base_offset);
    call(storage, &path, purpose, Op::Create)?;
    let bytes = codec::encode_header(header).to_vec();
    call(storage, &path, purpose, Op::Write(0, bytes))?;
    call(storage, &path, purpose, Op::SyncData)?;
    call(storage, dir, purpose, Op::SyncDir)?;
    Ok(())
}
fn validate(submit: &Submit) -> Result<(), AppendError> {
    if submit.records.is_empty() {
        return Err(AppendError::Invalid("empty submit"));
    }
    let mut keys = std::collections::BTreeSet::new();
    for record in &submit.records {
        if !(1..=codec::STREAM_LEN_MAX).contains(&record.stream.len()) {
            return Err(AppendError::Invalid("stream length outside limits"));
        }
        if let Some(key) = &record.key {
            if !(1..=codec::KEY_LEN_MAX).contains(&key.len()) {
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
