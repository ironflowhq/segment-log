// A fuzz build uses only the plain calls. The crash and fault parts serve the tests.
#![cfg_attr(not(test), allow(dead_code))]
use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
#[derive(Clone, Debug, PartialEq, Eq)]
struct File {
    durable: Vec<u8>,
    volatile: Vec<u8>,
    eligible: Vec<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Directory {
    durable: BTreeMap<PathBuf, u64>,
    volatile: BTreeMap<PathBuf, u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Image {
    files: BTreeMap<u64, File>,
    dirs: BTreeMap<PathBuf, Directory>,
    metadata_failed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Crash {
    Kill,
    PowerLoss,
    PowerLossTorn,
}
pub(crate) const ERROR_KINDS: &[io::ErrorKind] = &[
    io::ErrorKind::NotFound,
    io::ErrorKind::PermissionDenied,
    io::ErrorKind::ConnectionRefused,
    io::ErrorKind::ConnectionReset,
    io::ErrorKind::HostUnreachable,
    io::ErrorKind::NetworkUnreachable,
    io::ErrorKind::ConnectionAborted,
    io::ErrorKind::NotConnected,
    io::ErrorKind::AddrInUse,
    io::ErrorKind::AddrNotAvailable,
    io::ErrorKind::NetworkDown,
    io::ErrorKind::BrokenPipe,
    io::ErrorKind::AlreadyExists,
    io::ErrorKind::WouldBlock,
    io::ErrorKind::NotADirectory,
    io::ErrorKind::IsADirectory,
    io::ErrorKind::DirectoryNotEmpty,
    io::ErrorKind::ReadOnlyFilesystem,
    io::ErrorKind::StaleNetworkFileHandle,
    io::ErrorKind::InvalidInput,
    io::ErrorKind::InvalidData,
    io::ErrorKind::TimedOut,
    io::ErrorKind::WriteZero,
    io::ErrorKind::StorageFull,
    io::ErrorKind::NotSeekable,
    io::ErrorKind::QuotaExceeded,
    io::ErrorKind::FileTooLarge,
    io::ErrorKind::ResourceBusy,
    io::ErrorKind::ExecutableFileBusy,
    io::ErrorKind::Deadlock,
    io::ErrorKind::CrossesDevices,
    io::ErrorKind::TooManyLinks,
    io::ErrorKind::InvalidFilename,
    io::ErrorKind::ArgumentListTooLong,
    io::ErrorKind::Interrupted,
    io::ErrorKind::Unsupported,
    io::ErrorKind::UnexpectedEof,
    io::ErrorKind::OutOfMemory,
    io::ErrorKind::Other,
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FaultError(io::ErrorKind);
impl FaultError {
    pub(crate) fn kind(self) -> io::ErrorKind {
        self.0
    }
}
impl Fault {
    pub(crate) fn error(kind: io::ErrorKind) -> Result<Self, io::ErrorKind> {
        if ERROR_KINDS.contains(&kind) {
            Ok(Self::Error(FaultError(kind)))
        } else {
            Err(kind)
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    Error(FaultError),
    Partial(usize),
    NoSpace,
    Corrupt(usize, u8),
    Eof,
    Panic,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Ok(Value),
    Error(io::ErrorKind),
    Panic,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Call(Call, Option<Fault>, Outcome),
    Canceled(Call),
    Lock(PathBuf, bool),
    Unlock(u64),
    Crash(Crash, u64),
}
struct Held {
    call: Call,
    started: mpsc::SyncSender<()>,
    release: mpsc::Receiver<Option<Fault>>,
}
pub(crate) struct Hold {
    started: mpsc::Receiver<()>,
    release: mpsc::SyncSender<Option<Fault>>,
}
impl Hold {
    pub(crate) fn wait(&self) {
        self.started
            .recv_timeout(Duration::from_secs(5))
            .expect("held call did not start");
    }
    pub(crate) fn release(self, fault: Option<Fault>) {
        self.release.send(fault).expect("held call disappeared");
    }
}
struct State {
    image: Image,
    rng: u64,
    initial_seed: u64,
    boot: u64,
    trace: Vec<Event>,
    faults: Vec<(Call, Fault)>,
    held: Option<Held>,
    locks: BTreeSet<u64>,
}
#[derive(Clone)]
pub(crate) struct Sim {
    state: Arc<Mutex<State>>,
}
fn error(kind: io::ErrorKind) -> io::Error {
    io::Error::from(kind)
}
fn next(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    let mut x = *seed;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}
impl Sim {
    pub(crate) fn new(seed: u64) -> Self {
        let dirs = BTreeMap::from([(PathBuf::from("/"), Directory::default())]);
        Self {
            state: Arc::new(Mutex::new(State {
                image: Image {
                    files: BTreeMap::new(),
                    dirs,
                    metadata_failed: false,
                },
                rng: seed,
                initial_seed: seed,
                boot: 0,
                trace: vec![],
                faults: vec![],
                held: None,
                locks: BTreeSet::new(),
            })),
        }
    }
    pub(crate) fn inject(&self, call: Call, fault: Fault) {
        self.state.lock().unwrap().faults.push((call, fault));
    }
    pub(crate) fn hold(&self, call: Call) -> Hold {
        let (started, rx) = mpsc::sync_channel(1);
        let (tx, release) = mpsc::sync_channel(1);
        let mut s = self.state.lock().unwrap();
        assert!(s.held.is_none());
        s.held = Some(Held {
            call,
            started,
            release,
        });
        Hold {
            started: rx,
            release: tx,
        }
    }
    pub(crate) fn trace(&self) -> (u64, Vec<Event>) {
        let s = self.state.lock().unwrap();
        (s.initial_seed, s.trace.clone())
    }
    pub(crate) fn count(&self, path: &Path, purpose: Purpose, op: &Op) -> usize {
        self.state.lock().unwrap().trace.iter().filter(|e| matches!(e, Event::Call(c, _, _) if (c.path == path || segment_pattern(path, &c.path)) && c.purpose == purpose && std::mem::discriminant(&c.op) == std::mem::discriminant(op))).count()
    }
    pub(crate) fn image(&self) -> Image {
        self.state.lock().unwrap().image.clone()
    }
    pub(crate) fn crash(&self, mode: Crash, seed: u64) {
        let mut s = self.state.lock().unwrap();
        crash_image(&mut s.image, mode, seed);
        s.locks.clear();
        s.boot += 1;
        s.trace.push(Event::Crash(mode, seed));
    }
    pub(crate) fn replay(seed: u64, trace: &[Event]) -> Self {
        let sim = Self::new(seed);
        {
            let mut s = sim.state.lock().unwrap();
            for event in trace {
                match event {
                    Event::Canceled(_) => {}
                    Event::Call(call, fault, expected) => {
                        let State { image, rng, .. } = &mut *s;
                        let actual = apply(image, rng, call, fault.clone());
                        assert_eq!(&actual, expected, "replay diverged at {call:?}");
                    }
                    Event::Crash(mode, seed) => {
                        crash_image(&mut s.image, *mode, *seed);
                        s.locks.clear();
                        s.boot += 1;
                    }
                    Event::Lock(path, success) => {
                        let actual = file_id(&s.image, path).is_some_and(|id| s.locks.insert(id));
                        assert_eq!(actual, *success);
                    }
                    Event::Unlock(id) => {
                        s.locks.remove(id);
                    }
                }
            }
            s.trace = trace.to_vec();
        }
        sim
    }
}
struct Guard {
    boot: u64,
    sim: Sim,
    id: u64,
}
impl Lock for Guard {}
impl Drop for Guard {
    fn drop(&mut self) {
        let mut s = self.sim.state.lock().unwrap();
        if self.boot != s.boot {
            return;
        }
        s.locks.remove(&self.id);
        s.trace.push(Event::Unlock(self.id));
    }
}
impl Storage for Sim {
    fn call(&self, call: Call) -> io::Result<Value> {
        let (held, boot) = {
            let mut s = self.state.lock().unwrap();
            let held = if s.held.as_ref().is_some_and(|h| h.call.matches(&call)) {
                s.held.take()
            } else {
                None
            };
            (held, s.boot)
        };
        let release = held.map(|held| {
            held.started.send(()).unwrap();
            held.release
                .recv_timeout(Duration::from_secs(5))
                .expect("held call was not released")
        });
        let outcome = {
            let mut s = self.state.lock().unwrap();
            if boot != s.boot {
                s.trace.push(Event::Canceled(call));
                return Err(error(io::ErrorKind::Interrupted));
            }
            let fault = release.unwrap_or_else(|| {
                s.faults
                    .iter()
                    .position(|(c, _)| c.matches(&call))
                    .map(|i| s.faults.remove(i).1)
            });
            let State { image, rng, .. } = &mut *s;
            let outcome = apply(image, rng, &call, fault.clone());
            s.trace.push(Event::Call(call, fault, outcome.clone()));
            outcome
        };
        match outcome {
            Outcome::Ok(v) => Ok(v),
            Outcome::Error(k) => Err(error(k)),
            Outcome::Panic => panic!("injected storage panic"),
        }
    }
    fn lock(&self, path: &Path) -> io::Result<Box<dyn Lock>> {
        let mut s = self.state.lock().unwrap();
        let Some(id) = file_id(&s.image, path) else {
            s.trace.push(Event::Lock(path.to_owned(), false));
            return Err(error(io::ErrorKind::NotFound));
        };
        let success = s.locks.insert(id);
        s.trace.push(Event::Lock(path.to_owned(), success));
        if !success {
            return Err(error(io::ErrorKind::WouldBlock));
        }
        Ok(Box::new(Guard {
            boot: s.boot,
            sim: self.clone(),
            id,
        }))
    }
}
fn apply(image: &mut Image, rng: &mut u64, call: &Call, fault: Option<Fault>) -> Outcome {
    if fault == Some(Fault::Panic) {
        return Outcome::Panic;
    }
    let sync_error = match fault {
        Some(Fault::Error(kind)) => Some(kind.kind()),
        Some(Fault::NoSpace) if !matches!(call.op, Op::Write(..) | Op::Append(_)) => {
            Some(io::ErrorKind::StorageFull)
        }
        _ => None,
    };
    if let Some(kind) = sync_error {
        if matches!(call.op, Op::SyncData | Op::SyncAll)
            && let Some(file) = file_id(image, &call.path).and_then(|id| image.files.get_mut(&id))
        {
            file.eligible.fill(false);
        }
        if call.op == Op::SyncDir {
            image.metadata_failed = true;
        }
        return Outcome::Error(kind);
    }
    let result = execute(image, rng, call, &fault);
    match result {
        Ok(v) => Outcome::Ok(v),
        Err(e) => Outcome::Error(e.kind()),
    }
}
fn execute(
    image: &mut Image,
    rng: &mut u64,
    call: &Call,
    fault: &Option<Fault>,
) -> io::Result<Value> {
    let path = &call.path;
    if image.metadata_failed
        && !matches!(call.op, Op::Read(..) | Op::Open | Op::List | Op::Random(_))
    {
        return Err(error(io::ErrorKind::Other));
    }
    let parent = path.parent().unwrap_or(Path::new("/"));
    match &call.op {
        Op::CreateDir | Op::Create => {
            if file_id(image, path).is_some() || image.dirs.contains_key(path) {
                return Err(error(io::ErrorKind::AlreadyExists));
            }
            let id = image.files.keys().next_back().copied().unwrap_or(0) + 1;
            let dir = image
                .dirs
                .get_mut(parent)
                .ok_or_else(|| error(io::ErrorKind::NotFound))?;
            dir.volatile
                .insert(path.clone(), if call.op == Op::CreateDir { 0 } else { id });
            if call.op == Op::CreateDir {
                image.dirs.insert(path.clone(), Directory::default());
            } else {
                image.files.insert(
                    id,
                    File {
                        durable: vec![],
                        volatile: vec![],
                        eligible: vec![],
                    },
                );
            }
        }
        Op::List => {
            let dir = image
                .dirs
                .get(path)
                .ok_or_else(|| error(io::ErrorKind::NotFound))?;
            return Ok(Value::Names(
                dir.volatile
                    .keys()
                    .map(|p| PathBuf::from(p.file_name().unwrap()))
                    .collect(),
            ));
        }
        Op::SyncDir => {
            let dir = image
                .dirs
                .get_mut(path)
                .ok_or_else(|| error(io::ErrorKind::NotFound))?;
            dir.durable = dir.volatile.clone();
        }
        Op::Remove => {
            file_id(image, path).ok_or_else(|| error(io::ErrorKind::NotFound))?;
            image.dirs.get_mut(parent).unwrap().volatile.remove(path);
        }
        Op::Random(len) => return Ok(Value::Bytes((0..*len).map(|_| next(rng) as u8).collect())),
        op => {
            let id = file_id(image, path).ok_or_else(|| error(io::ErrorKind::NotFound))?;
            let file = image.files.get_mut(&id).unwrap();
            match op {
                Op::Open => return Ok(Value::Length(file.volatile.len() as u64)),
                Op::Append(bytes) | Op::Write(_, bytes) => {
                    let pos = if let Op::Write(pos, _) = op {
                        usize::try_from(*pos).map_err(|_| error(io::ErrorKind::InvalidInput))?
                    } else {
                        file.volatile.len()
                    };
                    let len = match fault {
                        Some(Fault::Partial(n)) => (*n).min(bytes.len()),
                        Some(Fault::NoSpace) => 0,
                        _ => bytes.len(),
                    };
                    let end = pos
                        .checked_add(len)
                        .ok_or_else(|| error(io::ErrorKind::InvalidInput))?;
                    if len > 0 {
                        if end > file.volatile.len() {
                            file.volatile.resize(end, 0);
                            file.eligible.resize(end, true);
                        }
                        file.volatile[pos..end].copy_from_slice(&bytes[..len]);
                        file.eligible[pos..end].fill(true);
                    }
                    if fault.is_some() {
                        return Err(error(if *fault == Some(Fault::NoSpace) {
                            io::ErrorKind::StorageFull
                        } else {
                            io::ErrorKind::Other
                        }));
                    }
                }
                Op::Read(pos, len) => {
                    let pos = usize::try_from(*pos)
                        .unwrap_or(usize::MAX)
                        .min(file.volatile.len());
                    let end = pos.saturating_add(*len).min(file.volatile.len());
                    let mut bytes = file.volatile[pos..end].to_vec();
                    match fault {
                        Some(Fault::Eof) => bytes.clear(),
                        Some(Fault::Corrupt(i, x)) if *i < bytes.len() => bytes[*i] ^= x,
                        _ => {}
                    }
                    return Ok(Value::Bytes(bytes));
                }
                Op::SyncData | Op::SyncAll => {
                    let old_len = file.durable.len();
                    file.durable.resize(file.volatile.len().max(old_len), 0);
                    for (i, eligible) in file.eligible.iter().enumerate() {
                        if *eligible {
                            file.durable[i] = file.volatile[i];
                        }
                    }
                    // Failed-writeback bytes must remain volatile until explicitly rewritten.
                    let len = file
                        .eligible
                        .iter()
                        .rposition(|b| *b)
                        .map_or(old_len.min(file.volatile.len()), |i| {
                            (i + 1).max(old_len.min(file.volatile.len()))
                        });
                    file.durable.truncate(len);
                    file.eligible.fill(false);
                }
                Op::Truncate(len) => {
                    let len =
                        usize::try_from(*len).map_err(|_| error(io::ErrorKind::InvalidInput))?;
                    file.volatile.resize(len, 0);
                    file.eligible.resize(len, true);
                }
                _ => unreachable!(),
            }
        }
    }
    Ok(Value::Unit)
}
fn crash_image(image: &mut Image, mode: Crash, mut seed: u64) {
    if mode == Crash::Kill {
        return;
    }
    for file in image.files.values_mut() {
        let len = file.volatile.len().max(file.durable.len());
        let mut bytes = file.durable.clone();
        bytes.resize(len, 0);
        for page in (0..file.volatile.len()).step_by(4096) {
            if next(&mut seed) & 1 == 0 {
                continue;
            }
            let end = (page + 4096).min(file.volatile.len());
            bytes[page..end].copy_from_slice(&file.volatile[page..end]);
            if mode == Crash::PowerLossTorn {
                for sector in (page..end).step_by(512) {
                    let end = (sector + 512).min(end);
                    if (sector..end)
                        .any(|i| file.volatile[i] != file.durable.get(i).copied().unwrap_or(0))
                    {
                        let start = sector + next(&mut seed) as usize % (end - sector);
                        for (i, b) in bytes.iter_mut().enumerate().take(end).skip(start) {
                            if file.volatile[i] != file.durable.get(i).copied().unwrap_or(0) {
                                *b = next(&mut seed) as u8;
                            }
                        }
                    }
                }
            }
        }
        // Surviving device bytes are still unflushed from the model's perspective.
        file.volatile = bytes;
        file.eligible.resize(len, false);
    }
    for dir in image.dirs.values_mut() {
        if next(&mut seed) & 1 == 0 {
            dir.volatile = dir.durable.clone();
        }
    }
    let mut reachable = BTreeSet::from([PathBuf::from("/")]);
    loop {
        let before = reachable.len();
        for (path, dir) in &image.dirs {
            if reachable.contains(path) {
                reachable.extend(dir.volatile.keys().cloned());
            }
        }
        if before == reachable.len() {
            break;
        }
    }
    let ids: BTreeSet<_> = image
        .dirs
        .iter()
        .filter(|(p, _)| reachable.contains(*p))
        .flat_map(|(_, d)| d.volatile.values().chain(d.durable.values()).copied())
        .collect();
    image.files.retain(|id, _| ids.contains(id));
    image.dirs.retain(|p, _| reachable.contains(p));
    image.metadata_failed = false;
}

fn file_id(image: &Image, path: &Path) -> Option<u64> {
    image
        .dirs
        .get(path.parent()?)?
        .volatile
        .get(path)
        .copied()
        .filter(|id| *id != 0)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
impl Drop for Sim {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        let Ok(state) = self.state.try_lock() else {
            return;
        };
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "segment-log-failure-{}-{}.trace",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let crash = state
            .trace
            .iter()
            .rev()
            .find(|event| matches!(event, Event::Crash(..)));
        match crate::acceptance::support::trace::save(&path, state.initial_seed, &state.trace) {
            Ok(()) => eprintln!(
                "SIM-MODEL, SIM-CONFORMANCE initial seed {} crash {crash:?} trace {}",
                state.initial_seed,
                path.display()
            ),
            Err(error) => eprintln!(
                "SIM-MODEL, SIM-CONFORMANCE could not save trace {}: {error}",
                path.display()
            ),
        }
    }
}
/// Whether `pattern` is `<dir>/*.seg` and `path` is a segment file in `<dir>`. A scenario names the
/// next group's flush this way, so it holds wherever a roll has left the active segment.
fn segment_pattern(pattern: &Path, path: &Path) -> bool {
    pattern.file_name() == Some("*.seg".as_ref())
        && path.parent() == pattern.parent()
        && path.extension() == Some("seg".as_ref())
}
impl Call {
    pub(crate) fn matches(&self, call: &Call) -> bool {
        self.purpose == call.purpose
            && self.op == call.op
            && (self.path == call.path || segment_pattern(&self.path, &call.path))
    }
}
