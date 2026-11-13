use crate::{
    codec,
    storage::{self, Call, Lock, Op, Purpose, Storage, Value},
    *,
};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, mpsc},
    thread::{self, JoinHandle},
    time::Instant,
};
mod closing;
#[cfg(fuzzing)]
pub mod fuzzing;
mod keys;
mod reader;
mod recovery;
mod reply;
mod survey;
mod view;
mod writer;
pub(crate) use keys::LogFilterMode;
use keys::{KeyHash, Keys};
pub use reader::LogReader;
pub use reply::Pending;
#[cfg(test)]
pub(crate) use view::FRAME_TABLES;
use view::{GroupSpan, Published, View};
use writer::{Active, Writer, run};

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
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct Hooks {
    pub key_hashes: BTreeMap<Vec<u8>, u64>,
    pub limits: Limits,
    /// Tests build the log key filter on the writer thread by default, so that traces stay the
    /// same from run to run.
    pub log_filter: LogFilterMode,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub footer_body: usize,
    pub keyed_records: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            footer_body: codec::FRAME_BODY_LIMIT,
            keyed_records: codec::KEYED_RECORD_LIMIT,
        }
    }
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeyMemory {
    pub filter_bytes: usize,
    pub sealed_keys: u64,
    pub active_keys: usize,
    pub log_filter_bytes: usize,
    pub log_filter_covered: usize,
}
pub struct SegmentLog {
    view: Arc<View>,
    gate: RwLock<Gate>,
    lifecycle: Mutex<Lifecycle>,
    poisoned: Arc<std::sync::atomic::AtomicBool>,
    options: Arc<Options>,
    used: Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    peak: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    drop_closed: Option<mpsc::Sender<()>>,
    #[cfg(test)]
    memory: Arc<Mutex<KeyMemory>>,
}
/// The queue's only sender. `append` holds the read lock across its state check and `try_send`,
/// and `seal` takes the sender out under the write lock. So seal starts at one point in queue
/// order: every admitted request is ahead of the seal marker, and none is behind it.
enum Gate {
    Open(mpsc::SyncSender<Message>),
    Closed,
}

enum Lifecycle {
    Writing {
        writer: JoinHandle<()>,
        lock: Box<dyn Lock>,
    },
    Stopped {
        _lock: Box<dyn Lock>,
    },
    Sealed,
}

enum Message {
    Append(Request),
    /// The writer answers after the seal's closing run is durable, and drops it unanswered if
    /// the seal fails.
    Seal(mpsc::Sender<()>),
}

impl SegmentLog {
    pub fn open(dir: impl AsRef<Path>, options: Options) -> Result<Self, OpenError> {
        Self::start(
            dir.as_ref(),
            options,
            Arc::new(storage::real::Real::default()),
            KeyHash::new,
            Limits::default(),
            LogFilterMode::Background,
        )
    }
    #[cfg(test)]
    pub(crate) fn with_storage(
        dir: &Path,
        options: Options,
        storage: Arc<dyn Storage>,
    ) -> Result<Self, OpenError> {
        Self::with_hooks(dir, options, storage, Hooks::default())
    }
    #[cfg(test)]
    pub(crate) fn with_hooks(
        dir: &Path,
        options: Options,
        storage: Arc<dyn Storage>,
        hooks: Hooks,
    ) -> Result<Self, OpenError> {
        let Hooks {
            key_hashes,
            limits,
            log_filter,
        } = hooks;
        let hash = |log_id| KeyHash::new(log_id).forcing(key_hashes);
        Self::start(dir, options, storage, hash, limits, log_filter)
    }
    fn start(
        dir: &Path,
        options: Options,
        storage: Arc<dyn Storage>,
        hash: impl FnOnce(codec::LogId) -> KeyHash,
        limits: Limits,
        log_filter: LogFilterMode,
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
        let recovery::Recovered {
            log_id,
            published,
            state,
        } = recovery::open(storage.as_ref(), dir)?;
        let view = Arc::new(View {
            storage,
            dir: dir.to_owned(),
            log_id,
            published: Mutex::new(published),
            tables: Mutex::default(),
        });
        let poisoned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        #[cfg(test)]
        let memory = Arc::new(Mutex::new(KeyMemory::default()));
        let (gate, lifecycle) = match state {
            recovery::State::Sealed => (Gate::Closed, Lifecycle::Sealed),
            recovery::State::Writable(writable) => {
                let mut keys = Keys::new(
                    hash(view.log_id.clone()),
                    writable.active_keys,
                    writable.sealed_keys,
                );
                // Open's own reads are done, so the build's reads come after them.
                keys.start_building(&view, log_filter);
                #[cfg(test)]
                {
                    *memory.lock().unwrap() = keys.memory();
                }
                let (sender, receiver) = mpsc::sync_channel(options.queue_capacity);
                let writer_view = view.clone();
                let writer_poisoned = poisoned.clone();
                let writer_options = options.clone();
                #[cfg(test)]
                let writer_memory = memory.clone();
                let writer = thread::Builder::new()
                    .name("segment-log-writer".into())
                    .spawn(move || {
                        run(
                            writer_view,
                            writer_poisoned,
                            receiver,
                            Writer {
                                epoch: writable.epoch,
                                keys,
                                active: writable.active,
                                limits,
                                #[cfg(test)]
                                memory: writer_memory,
                            },
                            writer_options,
                        );
                    })
                    .map_err(OpenError::Io)?;
                let lock = writable.lock;
                (Gate::Open(sender), Lifecycle::Writing { writer, lock })
            }
        };
        Ok(Self {
            view,
            gate: RwLock::new(gate),
            lifecycle: Mutex::new(lifecycle),
            poisoned,
            options,
            used: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            #[cfg(test)]
            peak: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            drop_closed: None,
            #[cfg(test)]
            memory,
        })
    }
    #[cfg(test)]
    pub(crate) fn key_memory(&self) -> KeyMemory {
        *self.memory.lock().unwrap()
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
        let gate = self.gate.read().unwrap();
        let Gate::Open(sender) = &*gate else {
            return Err(AppendError::Sealed);
        };
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
        match sender.try_send(Message::Append(Request {
            submit,
            reply,
            reservation,
            admitted: Instant::now(),
        })) {
            Ok(()) => Ok(pending),
            Err(mpsc::TrySendError::Full(_)) => Err(AppendError::Full),
            Err(mpsc::TrySendError::Disconnected(_)) => Err(AppendError::Poisoned),
        }
    }

    pub fn seal(&self) -> Result<(), SealError> {
        let mut lifecycle = self.lifecycle.lock().unwrap();
        let (writer, lock) = match std::mem::replace(&mut *lifecycle, Lifecycle::Sealed) {
            Lifecycle::Sealed => return Ok(()),
            stopped @ Lifecycle::Stopped { .. } => {
                *lifecycle = stopped;
                return Err(SealError::Poisoned);
            }
            Lifecycle::Writing { writer, lock } => (writer, lock),
        };
        // The write lock ends with this statement, before the send, which can wait for queue room
        // that `append` must never wait for.
        let gate = std::mem::replace(&mut *self.gate.write().unwrap(), Gate::Closed);
        let Gate::Open(sender) = gate else {
            unreachable!("a running writer's gate is open")
        };
        let (reply, sealed) = mpsc::channel();
        let sent = sender.send(Message::Seal(reply)).is_ok();
        drop(sender);
        let sealed = sent && sealed.recv().is_ok();
        let _ = writer.join();
        if sealed {
            drop(lock);
            Ok(())
        } else {
            self.poisoned
                .store(true, std::sync::atomic::Ordering::Release);
            *lifecycle = Lifecycle::Stopped { _lock: lock };
            Err(SealError::Poisoned)
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
        *self.gate.get_mut().unwrap() = Gate::Closed;
        #[cfg(test)]
        if let Some(observer) = self.drop_closed.take() {
            let _ = observer.send(());
        }
        let lifecycle = std::mem::replace(self.lifecycle.get_mut().unwrap(), Lifecycle::Sealed);
        if let Lifecycle::Writing { writer, lock } = lifecycle {
            let _ = writer.join();
            drop(lock);
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
/// `read_bytes` stops at the end of the file, which ends a scan. A lookup that needs every byte
/// it asks for uses this instead.
fn read_exact(
    storage: &dyn Storage,
    path: &Path,
    purpose: Purpose,
    position: u64,
    length: usize,
) -> io::Result<Vec<u8>> {
    let bytes = read_bytes(storage, path, purpose, position, length)?;
    if bytes.len() == length {
        Ok(bytes)
    } else {
        Err(io::ErrorKind::UnexpectedEof.into())
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
            *log.gate.get_mut().unwrap() = Gate::Closed;
        }
        let lifecycle = std::mem::replace(log.lifecycle.get_mut().unwrap(), Lifecycle::Sealed);
        let Lifecycle::Writing { writer, lock } = lifecycle else {
            panic!("the writer runs")
        };
        assert_eq!(writer.join().is_err(), panicked);
        *log.lifecycle.get_mut().unwrap() = Lifecycle::Stopped { _lock: lock };
    }
    pub(crate) fn base(capacity: usize, count: usize) -> Option<usize> {
        charge_base(capacity, count)
    }
    pub(crate) fn buffers(base: usize, stream: usize, key: usize, payload: usize) -> Option<usize> {
        charge_buffers(base, stream, key, payload)
    }
}
