//! Requested heap bytes by phase, not allocator footprint or RSS.
use segment_log_core::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
    time::Duration,
};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}
fn added(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(live, Relaxed);
    let _ = ALLOCATED.try_with(|n| n.set(n.get() + bytes));
}
// Forward each allocation unchanged. Counters never allocate or dereference pointers.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            added(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, old: Layout, size: usize) -> *mut u8 {
        let ptr = unsafe { System.realloc(ptr, old, size) };
        if !ptr.is_null() {
            LIVE.fetch_sub(old.size(), Relaxed);
            added(size);
        }
        ptr
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn measure<T>(phase: &str, f: impl FnOnce() -> T) -> (T, usize) {
    let before = LIVE.load(Relaxed);
    PEAK.store(before, Relaxed);
    let allocated = ALLOCATED.get();
    let value = f();
    let allocated = ALLOCATED.get() - allocated;
    let after = LIVE.load(Relaxed);
    let peak = PEAK.load(Relaxed);
    println!(
        "{phase},{allocated},{},{},{}",
        after as i64 - before as i64,
        peak.saturating_sub(before),
        after
    );
    (value, allocated)
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
    fn check(&self, records: &[Record], from: usize, count: usize) {
        assert_eq!(records.len(), count);
        for (i, record) in (from..).zip(records) {
            assert_eq!(record.offset, i as u64);
            assert_eq!(record.revision, (i / self.streams) as u64);
            assert_eq!(record.stream, (i % self.streams).to_le_bytes());
            assert_eq!(
                record.key.as_deref(),
                (i < self.keys)
                    .then_some(i.to_le_bytes())
                    .as_ref()
                    .map(|v| v.as_slice())
            );
            assert_eq!(record.payload.len(), self.payload);
            assert!(record.payload.iter().all(|&b| b == (i % 251) as u8));
        }
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert!(
        args.len() == 7 || args.len() == 8,
        "usage: memory_probe NEW_DIR RECORDS KEYS STREAMS GROUP_RECORDS PAYLOAD_BYTES [MAX_SINGLE_READ_ALLOCATION]"
    );
    let number = |i: usize| args[i].parse::<usize>().unwrap();
    let h = History {
        records: number(2),
        keys: number(3),
        streams: number(4),
        group: number(5),
        payload: number(6),
    };
    assert!(
        h.records > 0
            && h.keys <= h.records
            && h.streams > 0
            && h.streams <= h.records
            && h.group > 0
    );
    let dir = std::path::Path::new(&args[1]);
    assert!(!dir.exists(), "probe needs a new directory");
    let options = || Options {
        linger: Duration::ZERO,
        ..Options::default()
    };
    println!(
        "phase,thread_allocated_bytes,process_live_delta_bytes,process_peak_extra_bytes,process_live_bytes"
    );
    let (log, _) = measure("open_empty", || SegmentLog::open(dir, options()).unwrap());
    let (submits, _) = measure("caller_build", || {
        (0..h.records)
            .step_by(h.group)
            .map(|start| Submit {
                records: (start..(start + h.group).min(h.records))
                    .map(|i| h.record(i))
                    .collect(),
            })
            .collect::<Vec<_>>()
    });
    measure("append_and_release_caller", || {
        let mut offset = 0;
        for submit in submits {
            let count = submit.records.len();
            let committed = log.append(submit).unwrap().wait().unwrap();
            assert!(!committed.deduplicated);
            assert_eq!(committed.records.len(), count);
            for p in committed.records {
                assert_eq!(p.offset, offset as u64);
                assert_eq!(p.revision, (offset / h.streams) as u64);
                offset += 1;
            }
        }
        assert_eq!(offset, h.records);
        assert_eq!(log.watermark(), h.records as u64);
    });
    measure("drop_before_reopen", || drop(log));
    let (log, _) = measure("warm_reopen", || SegmentLog::open(dir, options()).unwrap());
    for (phase, from) in [("read_start", 0), ("read_end", h.records - 1)] {
        let (records, allocated) = measure(phase, || log.read(from as u64, 1).unwrap());
        h.check(&records, from, 1);
        if args.len() == 8 {
            assert!(
                allocated <= number(7),
                "{phase} allocated {allocated} bytes"
            );
        }
        drop(records);
    }
    let (records, _) = measure("validation_buffer", || log.read(0, usize::MAX).unwrap());
    measure("validation_check", || h.check(&records, 0, h.records));
    measure("validation_release", || drop(records));
    measure("drop_engine", || drop(log));
    println!(
        "validated_records={},failures=0,groups={}",
        h.records,
        h.records.div_ceil(h.group)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
