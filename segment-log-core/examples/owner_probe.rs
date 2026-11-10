//! Bytes that the engine owns, apart from the caller and the test.
//!
//! Every allocation that engine code requests comes from a separate arena: allocations on any
//! thread other than the main thread, and allocations on the main thread inside an engine call.
//! Everything else comes from the system allocator. The arena never returns memory to the
//! operating system, so its resident pages are a high-water mark of the engine's footprint.
use segment_log_core::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
    path::Path,
    ptr::NonNull,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};
use talc::{DefaultBinning, base::Talc, source::Manual};

/// Larger than the engine's footprint for every history this probe runs. The pages are reserved,
/// not resident, until the arena touches them.
const ARENA_BYTES: usize = 2 << 30;
#[cfg(target_os = "linux")]
const PAGE: usize = 4096;
/// The known size that checks the instrument, in each allocator.
const CALIBRATION_BYTES: usize = 1 << 20;

struct Counter {
    live: AtomicUsize,
    peak: AtomicUsize,
}
impl Counter {
    const fn new() -> Self {
        Self {
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }
    fn add(&self, bytes: usize) {
        let live = self.live.fetch_add(bytes, Relaxed) + bytes;
        self.peak.fetch_max(live, Relaxed);
    }
    fn sub(&self, bytes: usize) {
        self.live.fetch_sub(bytes, Relaxed);
    }
    /// Returns the live bytes and the peak since the last reset, then starts a new peak.
    fn take(&self) -> (usize, usize) {
        let live = self.live.load(Relaxed);
        (live, self.peak.swap(live, Relaxed))
    }
}

static ENGINE: Counter = Counter::new();
static OTHER: Counter = Counter::new();
static ARENA: Mutex<Talc<Manual, DefaultBinning>> = Mutex::new(Talc::new(Manual));
static ARENA_BASE: AtomicUsize = AtomicUsize::new(0);
/// An address on the stack of the first other thread that allocates: the engine's writer.
static WRITER_STACK: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    static MAIN: Cell<bool> = const { Cell::new(false) };
    static IN_ENGINE: Cell<bool> = const { Cell::new(false) };
}

fn in_arena(ptr: *mut u8) -> bool {
    let base = ARENA_BASE.load(Relaxed);
    base != 0 && (base..base + ARENA_BYTES).contains(&(ptr as usize))
}

fn engine_context() -> bool {
    if ARENA_BASE.load(Relaxed) == 0 {
        return false;
    }
    match (MAIN.try_with(Cell::get), IN_ENGINE.try_with(Cell::get)) {
        (Ok(true), Ok(inside)) => inside,
        // Any other thread belongs to the engine. A thread that is exiting has no locals left.
        _ => {
            let marker = 0u8;
            let _ = WRITER_STACK.compare_exchange(0, &raw const marker as usize, Relaxed, Relaxed);
            true
        }
    }
}

struct Owners;
unsafe impl GlobalAlloc for Owners {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if engine_context() {
            let ptr = unsafe { ARENA.lock().unwrap().allocate(layout) };
            match ptr {
                Some(ptr) => {
                    ENGINE.add(layout.size());
                    ptr.as_ptr()
                }
                None => std::ptr::null_mut(),
            }
        } else {
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                OTHER.add(layout.size());
            }
            ptr
        }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if in_arena(ptr) {
            ENGINE.sub(layout.size());
            unsafe { ARENA.lock().unwrap().deallocate(ptr, layout) };
        } else {
            OTHER.sub(layout.size());
            unsafe { System.dealloc(ptr, layout) };
        }
    }
    // A block stays in the allocator that made it, whoever grows it.
    unsafe fn realloc(&self, ptr: *mut u8, old: Layout, size: usize) -> *mut u8 {
        if !in_arena(ptr) {
            let new = unsafe { System.realloc(ptr, old, size) };
            if !new.is_null() {
                OTHER.sub(old.size());
                OTHER.add(size);
            }
            return new;
        }
        let mut arena = ARENA.lock().unwrap();
        let new = if unsafe { arena.try_realloc_in_place(ptr, old, size) } {
            ptr
        } else {
            let layout = unsafe { Layout::from_size_align_unchecked(size, old.align()) };
            let Some(new) = (unsafe { arena.allocate(layout) }).map(NonNull::as_ptr) else {
                return std::ptr::null_mut();
            };
            unsafe {
                std::ptr::copy_nonoverlapping(ptr, new, old.size().min(size));
                arena.deallocate(ptr, old);
            }
            new
        };
        ENGINE.sub(old.size());
        ENGINE.add(size);
        new
    }
}
#[global_allocator]
static ALLOCATOR: Owners = Owners;

fn reserve_arena() -> Result<(), String> {
    // SAFETY: an anonymous private mapping with no address hint aliases nothing.
    let base = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            ARENA_BYTES,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
            -1,
            0,
        )
    };
    if base == libc::MAP_FAILED {
        return Err(format!("mmap: {}", std::io::Error::last_os_error()));
    }
    // Huge pages would count 2 MiB for each touched range, which says nothing about the engine.
    #[cfg(target_os = "linux")]
    if unsafe { libc::madvise(base, ARENA_BYTES, libc::MADV_NOHUGEPAGE) } != 0 {
        return Err(format!("madvise: {}", std::io::Error::last_os_error()));
    }
    unsafe { ARENA.lock().unwrap().claim(base.cast(), ARENA_BYTES) }.ok_or("claim failed")?;
    ARENA_BASE.store(base as usize, Relaxed);
    Ok(())
}

fn engine<T>(f: impl FnOnce() -> T) -> T {
    IN_ENGINE.set(true);
    let value = f();
    IN_ENGINE.set(false);
    value
}

/// Heap bytes of records that the engine returned and the caller now holds.
fn result_bytes(records: &[Record], capacity: usize) -> Result<usize, String> {
    // This count must not allocate, or it would change the figures that it reports.
    let mut bytes = 0;
    let mut add = |ptr: *const u8, size: usize| {
        if size > 0 && !in_arena(ptr.cast_mut()) {
            return Err("a result block is outside the engine arena");
        }
        bytes += size;
        Ok(())
    };
    add(records.as_ptr().cast(), capacity * size_of::<Record>())?;
    for r in records {
        for v in [Some(&r.stream), r.key.as_ref(), Some(&r.payload)]
            .into_iter()
            .flatten()
        {
            add(v.as_ptr(), v.capacity())?;
        }
    }
    Ok(bytes)
}

fn arena_resident_bytes() -> Result<usize, String> {
    resident_bytes(ARENA_BASE.load(Relaxed), ARENA_BYTES)
}

/// Resident pages of a page-aligned range, from the page tables.
#[cfg(target_os = "linux")]
fn resident_bytes(base: usize, len: usize) -> Result<usize, String> {
    use std::os::unix::fs::FileExt;
    let file = std::fs::File::open("/proc/self/pagemap").map_err(|e| e.to_string())?;
    let first = base / PAGE;
    let mut buffer = vec![0u8; 8 * 8192];
    let mut present = 0;
    for chunk in (0..len / PAGE).step_by(8192) {
        let pages = 8192.min(len / PAGE - chunk);
        let bytes = &mut buffer[..8 * pages];
        file.read_exact_at(bytes, ((first + chunk) * 8) as u64)
            .map_err(|e| e.to_string())?;
        // Bit 63 is set for a present page, bit 62 for a swapped one.
        for entry in bytes.chunks_exact(8) {
            let entry = u64::from_le_bytes(entry.try_into().unwrap());
            if entry >> 62 & 1 == 1 {
                return Err("a counted page is swapped".into());
            }
            present += (entry >> 63) as usize;
        }
    }
    Ok(present * PAGE)
}

/// `Rss` of the mapping that holds the writer's stack, from the page tables.
#[cfg(target_os = "linux")]
fn writer_stack_bytes() -> Result<Option<usize>, String> {
    let address = WRITER_STACK.load(Relaxed);
    if address == 0 {
        return Ok(None);
    }
    let smaps = std::fs::read_to_string("/proc/self/smaps").map_err(|e| e.to_string())?;
    let mut inside = false;
    for line in smaps.lines() {
        let first = line.split_whitespace().next().unwrap_or("");
        if let Some((start, end)) = first.split_once('-')
            && let (Ok(start), Ok(end)) = (
                usize::from_str_radix(start, 16),
                usize::from_str_radix(end, 16),
            )
        {
            inside = (start..end).contains(&address);
        } else if inside && let Some(rest) = line.strip_prefix("Rss:") {
            let kib = rest.trim().trim_end_matches(" kB").trim();
            return kib
                .parse::<usize>()
                .map(|k| Some(k * 1024))
                .map_err(|e| e.to_string());
        }
    }
    Err("no mapping holds the writer's stack".into())
}

#[cfg(not(target_os = "linux"))]
fn resident_bytes(_base: usize, _len: usize) -> Result<usize, String> {
    Err("residency needs /proc/self/pagemap".into())
}
#[cfg(not(target_os = "linux"))]
fn writer_stack_bytes() -> Result<Option<usize>, String> {
    Err("stack residency needs /proc/self/smaps".into())
}

struct Report {
    results: usize,
}
impl Report {
    fn row(&self, phase: &str) -> Result<(), String> {
        let (engine_live, engine_peak) = ENGINE.take();
        let (other_live, other_peak) = OTHER.take();
        let arena = arena_resident_bytes()?;
        let stack = writer_stack_bytes()?.map_or("-".to_string(), |b| b.to_string());
        let held = engine_live
            .checked_sub(self.results)
            .ok_or("the caller holds more engine bytes than are live")?;
        println!(
            "{phase},{engine_live},{engine_peak},{},{held},{other_live},{other_peak},{arena},{stack}",
            self.results
        );
        // The instrument's own reads above are not part of the next phase.
        ENGINE.take();
        OTHER.take();
        Ok(())
    }
}

struct History {
    records: usize,
    keys: usize,
    streams: usize,
    group: usize,
    payload: usize,
}
impl History {
    fn record(&self, i: usize) -> NewRecord {
        NewRecord {
            stream: (i % self.streams).to_le_bytes().to_vec(),
            expected: if i < self.streams {
                ExpectedRevision::Empty
            } else {
                ExpectedRevision::At((i / self.streams - 1) as u64)
            },
            key: (i < self.keys).then(|| i.to_le_bytes().to_vec()),
            payload: vec![(i % 251) as u8; self.payload],
        }
    }
    fn check(&self, records: &[Record], from: usize, count: usize) -> Result<(), String> {
        if records.len() != count {
            return Err(format!("count {} != {count}", records.len()));
        }
        for (i, r) in (from..).zip(records) {
            let key = (i < self.keys).then(|| i.to_le_bytes());
            if r.offset != i as u64
                || r.revision != (i / self.streams) as u64
                || r.stream != (i % self.streams).to_le_bytes()
                || r.key.as_deref() != key.as_ref().map(|k| k.as_slice())
                || r.payload.len() != self.payload
                || !r.payload.iter().all(|&b| b == (i % 251) as u8)
            {
                return Err(format!("record mismatch at {i}"));
            }
        }
        Ok(())
    }
}

fn check(name: &str, expected: usize, observed: usize) {
    println!("check,{name},{expected},{observed}");
}

/// Counts a separate mapping of known size, so that the check leaves the arena untouched.
fn check_page_counter() -> Result<(), String> {
    // SAFETY: an anonymous private mapping with no address hint aliases nothing.
    let base = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            CALIBRATION_BYTES,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if base == libc::MAP_FAILED {
        return Err(format!("mmap: {}", std::io::Error::last_os_error()));
    }
    let pages = base.cast::<u8>();
    for offset in (0..CALIBRATION_BYTES).step_by(4096) {
        // SAFETY: the offset is inside the mapping, which nothing else uses.
        unsafe { pages.add(offset).write_volatile(0xa5) };
    }
    check(
        "pages_touched",
        CALIBRATION_BYTES,
        resident_bytes(base as usize, CALIBRATION_BYTES)?,
    );
    if unsafe { libc::munmap(base, CALIBRATION_BYTES) } != 0 {
        return Err(format!("munmap: {}", std::io::Error::last_os_error()));
    }
    check(
        "pages_unmapped",
        0,
        resident_bytes(base as usize, CALIBRATION_BYTES)?,
    );
    // The check's own reads are not part of the next phase.
    ENGINE.take();
    OTHER.take();
    Ok(())
}

/// Each allocator must count a buffer of known size exactly. The engine's buffer comes after the
/// engine is gone, so that it cannot lend the engine resident pages.
fn check_counters() -> Result<(), String> {
    for (held, released, counter, in_engine) in [
        ("engine_buffer", "engine_release", &ENGINE, true),
        ("other_buffer", "other_release", &OTHER, false),
    ] {
        let before = counter.live.load(Relaxed);
        let fill = || black_box(vec![0xa5u8; CALIBRATION_BYTES]);
        let buffer = if in_engine { engine(fill) } else { fill() };
        let during = counter.live.load(Relaxed);
        if !buffer.iter().all(|&b| b == 0xa5) {
            return Err("calibration buffer mismatch".into());
        }
        drop(buffer);
        let after = counter.live.load(Relaxed);
        check(held, CALIBRATION_BYTES, during.wrapping_sub(before));
        check(released, 0, after.wrapping_sub(before));
    }
    Ok(())
}

fn run(validated: &mut usize) -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 8 {
        return Err(
            "usage: owner_probe MODE DIR RECORDS KEYS STREAMS GROUP PAYLOAD; MODE = append|recover"
                .into(),
        );
    }
    let number = |i: usize| args[i].parse::<usize>().map_err(|e| e.to_string());
    let h = History {
        records: number(3)?,
        keys: number(4)?,
        streams: number(5)?,
        group: number(6)?,
        payload: number(7)?,
    };
    if h.records == 0
        || h.keys > h.records
        || h.streams == 0
        || h.streams > h.records
        || h.group == 0
    {
        return Err("invalid history dimensions".into());
    }
    let mode = args[1].as_str();
    let dir = Path::new(&args[2]);
    if !matches!(mode, "append" | "recover") || (mode == "recover") != dir.exists() {
        return Err("append needs a new directory; recover needs an existing one".into());
    }
    let options = || Options {
        linger: Duration::ZERO,
        ..Options::default()
    };
    let mut report = Report { results: 0 };
    println!(
        "phase,engine_live_bytes,engine_peak_bytes,engine_bytes_caller_holds,engine_held_bytes,\
         other_live_bytes,other_peak_bytes,arena_resident_bytes,writer_stack_resident_bytes"
    );
    report.row("baseline")?;
    check_page_counter()?;
    let log = engine(|| SegmentLog::open(dir, options())).map_err(|e| format!("open: {e:?}"))?;
    if mode == "append" {
        report.row("empty_held")?;
        let submits: Vec<Submit> = (0..h.records)
            .step_by(h.group)
            .map(|start| Submit {
                records: (start..(start + h.group).min(h.records))
                    .map(|i| h.record(i))
                    .collect(),
            })
            .collect();
        report.row("caller_held")?;
        let mut offset = 0;
        for submit in submits {
            let count = submit.records.len();
            let committed = engine(|| log.append(submit).map(Pending::wait))
                .map_err(|e| format!("append: {e:?}"))?
                .map_err(|e| format!("commit: {e:?}"))?;
            if committed.deduplicated || committed.records.len() != count {
                return Err("commit count/dedup mismatch".into());
            }
            for p in &committed.records {
                if p.offset != offset as u64 || p.revision != (offset / h.streams) as u64 {
                    return Err("commit position mismatch".into());
                }
                offset += 1;
            }
        }
        report.row("appended_held")?;
    } else {
        report.row("recovered_held")?;
    }
    if engine(|| log.watermark()) != h.records as u64 {
        return Err("watermark mismatch".into());
    }
    for (held, dropped, from, count) in [
        ("read_one_held", "read_one_dropped", 0, 1),
        ("validation_held", "validation_dropped", 0, h.records),
    ] {
        let records =
            engine(|| log.read(from as u64, count)).map_err(|e| format!("read: {e:?}"))?;
        report.results = result_bytes(&records, records.capacity())?;
        report.row(held)?;
        h.check(&records, from, count)?;
        *validated = records.len();
        drop(records);
        report.results = 0;
        report.row(dropped)?;
    }
    engine(|| drop(log));
    report.row("engine_dropped")?;
    check_counters()
}

fn main() {
    MAIN.set(true);
    let mut validated = 0;
    let result = reserve_arena().and_then(|()| run(&mut validated));
    println!(
        "result,validated={validated},failures={}",
        usize::from(result.is_err())
    );
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
