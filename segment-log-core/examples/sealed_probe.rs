//! Measures a log with sealed, keyed history through the public API only, so that the same file
//! builds against the M3 and M4 code (scripts/m4-evidence.sh).
//!
//! usage:
//!   sealed_probe build NEW_FIXTURE keyed|unkeyed SEALED_SEGMENTS ROLL_BYTES ACTIVE_BYTES
//!   sealed_probe open FIXTURE cold|warm RUNS
//!   sealed_probe append FIXTURE CALLERS ROUNDS [INTERVAL_US]
//!
//! A fixture is a directory holding `log/` and `manifest`, the facts that `open` checks.
use segment_log_core::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::BTreeMap,
    fs,
    path::Path,
    process::Command,
    sync::{
        Barrier, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    },
    thread,
    time::{Duration, Instant},
};

const PAYLOAD: usize = 256;
const STREAMS: u64 = 1000;
/// One build submit holds each fixture stream once, so expected revisions never chain inside it.
const GROUP: u64 = STREAMS;
/// Fixture streams and keys carry this tag; workload callers stay far below it, so a workload key
/// is never a fixture key and every workload lookup is the "not found" path.
const TAG: u64 = 0xf1f1_f1f1_f1f1_f1f1;

struct Counting;
static COUNTING: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn added(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(live, Relaxed);
}
// The counters run only during `open`, so the 1,000 append callers never share their cache line.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() && COUNTING.load(Relaxed) {
            added(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if COUNTING.load(Relaxed) {
            LIVE.fetch_sub(layout.size(), Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, old: Layout, size: usize) -> *mut u8 {
        let ptr = unsafe { System.realloc(ptr, old, size) };
        if !ptr.is_null() && COUNTING.load(Relaxed) {
            LIVE.fetch_sub(old.size(), Relaxed);
            added(size);
        }
        ptr
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

struct Manifest {
    keyed: bool,
    records: u64,
    roll_bytes: u64,
}
impl Manifest {
    fn write(&self, fixture: &Path) {
        let text = format!(
            "keyed={}\nrecords={}\nroll_bytes={}\nstreams={STREAMS}\npayload={PAYLOAD}\n",
            u8::from(self.keyed),
            self.records,
            self.roll_bytes
        );
        fs::write(fixture.join("manifest"), text).unwrap();
    }
    fn read(fixture: &Path) -> Self {
        let text = fs::read_to_string(fixture.join("manifest")).unwrap();
        let fields: BTreeMap<_, _> = text.lines().filter_map(|l| l.split_once('=')).collect();
        let number = |name: &str| fields[name].parse::<u64>().unwrap();
        assert_eq!(
            (number("streams"), number("payload")),
            (STREAMS, PAYLOAD as u64)
        );
        Self {
            keyed: number("keyed") == 1,
            records: number("records"),
            roll_bytes: number("roll_bytes"),
        }
    }
    fn options(&self) -> Options {
        Options {
            roll_bytes: self.roll_bytes,
            ..Options::default()
        }
    }
}

fn expected(revision: u64) -> ExpectedRevision {
    match revision {
        0 => ExpectedRevision::Empty,
        r => ExpectedRevision::At(r - 1),
    }
}
fn fixture_record(keyed: bool, i: u64) -> NewRecord {
    let mut key = TAG.to_le_bytes().to_vec();
    key.extend_from_slice(&i.to_le_bytes());
    NewRecord {
        stream: (TAG ^ (i % STREAMS)).to_le_bytes().to_vec(),
        expected: expected(i / STREAMS),
        key: keyed.then_some(key),
        payload: vec![(i % 251) as u8; PAYLOAD],
    }
}
fn workload_record(caller: u64, round: u64) -> NewRecord {
    let mut key = caller.to_le_bytes().to_vec();
    key.extend_from_slice(&round.to_le_bytes());
    let mut payload = vec![0x5a; PAYLOAD];
    payload[..16].copy_from_slice(&key);
    NewRecord {
        stream: caller.to_le_bytes().to_vec(),
        expected: expected(round),
        key: Some(key),
        payload,
    }
}
fn matches(record: &Record, offset: u64, revision: u64, new: &NewRecord) -> bool {
    (record.offset, record.revision) == (offset, revision)
        && record.stream == new.stream
        && record.key == new.key
        && record.payload == new.payload
}

/// The segment files of a log, oldest first, as (base offset, bytes). The file name is the base
/// offset (format.md), so a sealed segment holds the records up to the next file's name.
struct Segments {
    files: Vec<(u64, u64)>,
    other_bytes: u64,
}
impl Segments {
    fn scan(log: &Path) -> Self {
        let mut files = Vec::new();
        let mut other_bytes = 0;
        for entry in fs::read_dir(log).unwrap() {
            let entry = entry.unwrap();
            let bytes = entry.metadata().unwrap().len();
            let name = entry.file_name().into_string().unwrap();
            match name.strip_suffix(".seg").map(str::parse::<u64>) {
                Some(Ok(base)) => files.push((base, bytes)),
                _ => other_bytes += bytes,
            }
        }
        files.sort_unstable();
        Self { files, other_bytes }
    }
    fn sealed_records(&self) -> Vec<u64> {
        self.files.windows(2).map(|w| w[1].0 - w[0].0).collect()
    }
    fn active_bytes(&self) -> u64 {
        self.files.last().map_or(0, |f| f.1)
    }
    /// The fixture facts, and the key-filter bytes that ADR 0048 rule 5 predicts from the
    /// key filter frame layout in format.md: `8 * ceil(n / 255) + 64 * ceil(n / 32)` per sealed
    /// segment with `n` keyed records.
    fn print(&self, manifest: &Manifest) {
        let sealed = self.sealed_records();
        let keys: Vec<u64> = sealed
            .iter()
            .map(|&n| if manifest.keyed { n } else { 0 })
            .collect();
        let filter_bytes: u64 = keys
            .iter()
            .map(|&n| 8 * n.div_ceil(255) + 64 * n.div_ceil(32))
            .sum();
        let index_bytes: u64 = keys.iter().map(|&n| 4096 * n.div_ceil(255)).sum();
        let sealed_keys: u64 = keys.iter().sum();
        let active_base = self.files.last().map_or(0, |f| f.0);
        let segment_bytes: u64 = self.files.iter().map(|f| f.1).sum();
        println!(
            "FIXTURE keyed={} records={} roll_bytes={} streams={STREAMS} payload={PAYLOAD} key_bytes={} record_entry_bytes={} sealed_segments={} min_records_per_sealed={} max_records_per_sealed={} sealed_bytes={} active_bytes={} active_records={} active_keys={} dir_bytes={}",
            u8::from(manifest.keyed),
            manifest.records,
            manifest.roll_bytes,
            if manifest.keyed { 16 } else { 0 },
            16 + 8 + if manifest.keyed { 16 } else { 0 } + PAYLOAD,
            sealed.len(),
            sealed.iter().min().unwrap_or(&0),
            sealed.iter().max().unwrap_or(&0),
            segment_bytes - self.active_bytes(),
            self.active_bytes(),
            manifest.records - active_base,
            if manifest.keyed {
                manifest.records - active_base
            } else {
                0
            },
            segment_bytes + self.other_bytes,
        );
        println!(
            "FILTERS sealed_keys={sealed_keys} expected_filter_bytes={filter_bytes} filter_bytes_per_key={:.3} key_index_bytes={index_bytes}",
            filter_bytes as f64 / sealed_keys.max(1) as f64
        );
    }
}

fn build(fixture: &Path, keyed: bool, sealed: usize, roll_bytes: u64, active_bytes: u64) {
    assert!(sealed > 0 && active_bytes < roll_bytes);
    fs::create_dir(fixture).expect("build needs a new fixture directory");
    let log_dir = fixture.join("log");
    let options = Options {
        linger: Duration::ZERO,
        roll_bytes,
        ..Options::default()
    };
    let started = Instant::now();
    let log = SegmentLog::open(&log_dir, options).unwrap();
    let mut records = 0;
    loop {
        let submit = Submit {
            records: (records..records + GROUP)
                .map(|i| fixture_record(keyed, i))
                .collect(),
        };
        let committed = log.append(submit).unwrap().wait().unwrap();
        assert!(!committed.deduplicated);
        assert_eq!(committed.records[0].offset, records);
        records += GROUP;
        let segments = Segments::scan(&log_dir);
        assert!(
            segments.files.len() <= sealed + 1,
            "passed the sealed count"
        );
        if segments.files.len() == sealed + 1 && segments.active_bytes() >= active_bytes {
            break;
        }
    }
    assert_eq!(log.watermark(), records);
    drop(log);
    let manifest = Manifest {
        keyed,
        records,
        roll_bytes,
    };
    manifest.write(fixture);
    println!("BUILD ns={}", started.elapsed().as_nanos());
    Segments::scan(&log_dir).print(&manifest);
}

fn proc_kib(file: &str, field: &str) -> Option<u64> {
    let text = fs::read_to_string(file).ok()?;
    let line = text.lines().find_map(|l| l.strip_prefix(field))?;
    line.split_whitespace().next()?.parse().ok()
}
fn shown(value: Option<u64>) -> String {
    value.map_or_else(|| "na".into(), |v| v.to_string())
}
fn drop_caches() {
    let dropped = if cfg!(target_os = "linux") {
        Command::new("sync").status().unwrap().success()
            && (fs::write("/proc/sys/vm/drop_caches", "3\n").is_ok()
                || Command::new("sudo")
                    .args(["-n", "sh", "-c", "echo 3 > /proc/sys/vm/drop_caches"])
                    .status()
                    .unwrap()
                    .success())
    } else {
        Command::new("sudo")
            .args(["-n", "purge"])
            .status()
            .is_ok_and(|s| s.success())
    };
    assert!(
        dropped,
        "a cold run needs root to drop the page cache (Linux drop_caches, macOS purge)"
    );
}
fn quantile(sorted: &[u64], numerator: usize, denominator: usize) -> u64 {
    sorted[(sorted.len() * numerator)
        .div_ceil(denominator)
        .saturating_sub(1)]
}

/// Returns the number of failed checks: the watermark, the first and last records, the last
/// stream head, and, for a keyed fixture, a retry of the oldest key, which must find it through
/// the oldest sealed segment.
fn check_open(log: &SegmentLog, manifest: &Manifest) -> usize {
    let last = manifest.records - 1;
    let read_ok = |offset: u64| {
        log.read(offset, 1).is_ok_and(|r| {
            r.len() == 1
                && matches(
                    &r[0],
                    offset,
                    offset / STREAMS,
                    &fixture_record(manifest.keyed, offset),
                )
        })
    };
    let head = log.stream_head(&fixture_record(false, last).stream);
    let mut failures = usize::from(log.watermark() != manifest.records)
        + usize::from(!read_ok(0))
        + usize::from(!read_ok(last))
        + usize::from(head.is_none_or(|h| (h.offset, h.revision) != (last, last / STREAMS)));
    if manifest.keyed {
        let retry = Submit {
            records: vec![fixture_record(true, 0)],
        };
        let found = log
            .append(retry)
            .map_err(|e| format!("{e:?}"))
            .and_then(|p| p.wait().map_err(|e| format!("{e:?}")));
        failures += usize::from(!found.is_ok_and(|c| c.deduplicated && c.records[0].offset == 0));
    }
    failures
}

fn open(fixture: &Path, cold: bool, runs: usize) {
    assert!(runs > 0);
    let manifest = Manifest::read(fixture);
    let log_dir = fixture.join("log");
    Segments::scan(&log_dir).print(&manifest);
    let mut times = Vec::with_capacity(runs);
    let mut failures = 0;
    let cache = if cold { "cold" } else { "warm" };
    for run in 1..=runs {
        if cold {
            drop_caches();
        }
        let page_cache = proc_kib("/proc/meminfo", "Cached:");
        let rss_before = proc_kib("/proc/self/status", "VmRSS:");
        LIVE.store(0, Relaxed);
        PEAK.store(0, Relaxed);
        COUNTING.store(true, Relaxed);
        let started = Instant::now();
        let log = SegmentLog::open(&log_dir, manifest.options()).unwrap();
        let ns = started.elapsed().as_nanos() as u64;
        COUNTING.store(false, Relaxed);
        let (retained, peak) = (LIVE.load(Relaxed), PEAK.load(Relaxed));
        let rss_after = proc_kib("/proc/self/status", "VmRSS:");
        let errors = check_open(&log, &manifest);
        failures += errors;
        println!(
            "OPEN run={run} cache={cache} ns={ns} watermark={} heap_retained_bytes={retained} heap_peak_bytes={peak} rss_before_kib={} rss_after_kib={} page_cache_before_kib={} correctness_errors={errors}",
            log.watermark(),
            shown(rss_before),
            shown(rss_after),
            shown(page_cache),
        );
        drop(log);
        times.push(ns);
    }
    times.sort_unstable();
    println!(
        "OPEN_SUMMARY cache={cache} runs={runs} min_ns={} p50_ns={} max_ns={} target_ns=50000000 correctness_errors={failures}",
        times[0],
        quantile(&times, 50, 100),
        times[runs - 1]
    );
    assert_eq!(failures, 0, "open returned a log that failed its checks");
}

#[derive(Default)]
struct Caller {
    latencies: Vec<u64>,
    offsets: Vec<u64>,
    full: usize,
    commit_errors: usize,
    correctness_errors: usize,
}

fn run_caller(
    log: &SegmentLog,
    caller: u64,
    rounds: u64,
    pace: Option<(Instant, Duration, f64)>,
) -> Caller {
    let mut out = Caller::default();
    for round in 0..rounds {
        // A late paced caller still measures from its scheduled send, as the M1 benchmark does.
        let sent = match pace {
            None => Instant::now(),
            Some((epoch, interval, share)) => {
                let due = epoch + interval.mul_f64(share) + interval * round as u32;
                thread::sleep(due.saturating_duration_since(Instant::now()));
                due
            }
        };
        let pending = loop {
            match log.append(Submit {
                records: vec![workload_record(caller, round)],
            }) {
                Ok(p) => break p,
                Err(AppendError::Full) => {
                    out.full += 1;
                    thread::yield_now();
                }
                Err(error) => panic!("admission failure: {error:?}"),
            }
        };
        match pending.wait() {
            Ok(c) => {
                out.latencies.push(sent.elapsed().as_nanos() as u64);
                let ok = !c.deduplicated && c.records.len() == 1 && c.records[0].revision == round;
                out.correctness_errors += usize::from(!ok);
                out.offsets.push(c.records[0].offset);
            }
            Err(error) => {
                eprintln!("commit failure caller={caller} round={round}: {error:?}");
                out.commit_errors += 1;
                break;
            }
        }
    }
    out
}

fn append(fixture: &Path, callers: u64, rounds: u64, interval: Option<Duration>) {
    assert!(callers > 0 && rounds > 0);
    let manifest = Manifest::read(fixture);
    let log_dir = fixture.join("log");
    let start_segments = Segments::scan(&log_dir).files.len();
    let options = manifest.options();
    println!(
        "CONFIG callers={callers} rounds={rounds} payload={PAYLOAD} key_bytes=16 records_per_submit=1 interval_us={} offered_records_per_second={} linger_ns={} group_byte_cap={} roll_bytes={} queue_capacity={} pending_byte_budget={} start_records={} start_segments={start_segments}",
        interval.map_or_else(|| "closed_loop".into(), |i| i.as_micros().to_string()),
        interval.map_or_else(
            || "closed_loop".into(),
            |i| format!("{:.0}", callers as f64 / i.as_secs_f64())
        ),
        options.linger.as_nanos(),
        options.group_byte_cap,
        options.roll_bytes,
        options.queue_capacity,
        options.pending_byte_budget,
        manifest.records,
    );
    let started = Instant::now();
    let log = SegmentLog::open(&log_dir, options).unwrap();
    println!("OPEN ns={}", started.elapsed().as_nanos());
    assert_eq!(log.watermark(), manifest.records);
    let start = Barrier::new(callers as usize + 1);
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let results: Vec<Caller> = thread::scope(|scope| {
        let workers: Vec<_> = (0..callers)
            .map(|caller| {
                let (log, start) = (&log, &start);
                thread::Builder::new()
                    .name(format!("caller-{caller}"))
                    .spawn_scoped(scope, move || {
                        start.wait();
                        let epoch = *EPOCH.get().unwrap();
                        let pace = interval.map(|i| (epoch, i, caller as f64 / callers as f64));
                        run_caller(log, caller, rounds, pace)
                    })
                    .unwrap()
            })
            .collect();
        // Callers read the epoch only after the barrier, which opens when this thread arrives.
        EPOCH.set(Instant::now()).unwrap();
        start.wait();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    let elapsed = EPOCH.get().unwrap().elapsed();
    let mut latencies: Vec<u64> = results
        .iter()
        .flat_map(|r| r.latencies.iter().copied())
        .collect();
    let mut offsets: Vec<u64> = results
        .iter()
        .flat_map(|r| r.offsets.iter().copied())
        .collect();
    let full: usize = results.iter().map(|r| r.full).sum();
    let commit_errors: usize = results.iter().map(|r| r.commit_errors).sum();
    let mut correctness_errors: usize = results.iter().map(|r| r.correctness_errors).sum();
    let records = callers * rounds;
    offsets.sort_unstable();
    correctness_errors += usize::from(
        !offsets
            .iter()
            .copied()
            .eq(manifest.records..manifest.records + records),
    );
    correctness_errors += usize::from(log.watermark() != manifest.records + records);
    for caller in 0..callers {
        let new = workload_record(caller, rounds - 1);
        let ok = log.stream_head(&new.stream).is_some_and(|h| {
            h.revision == rounds - 1
                && log
                    .read(h.offset, 1)
                    .is_ok_and(|r| r.len() == 1 && matches(&r[0], h.offset, rounds - 1, &new))
        });
        correctness_errors += usize::from(!ok);
    }
    drop(log);
    let end_segments = Segments::scan(&log_dir).files.len();
    println!(
        "APPEND records={records} elapsed_ns={} records_per_second={:.3} full={full} commit_errors={commit_errors} correctness_errors={correctness_errors} start_segments={start_segments} end_segments={end_segments}",
        elapsed.as_nanos(),
        records as f64 / elapsed.as_secs_f64(),
    );
    latencies.sort_unstable();
    if !latencies.is_empty() {
        println!(
            "METRIC append_ns count={} min={} p50={} p99={} p999={} max={}",
            latencies.len(),
            latencies[0],
            quantile(&latencies, 50, 100),
            quantile(&latencies, 99, 100),
            quantile(&latencies, 999, 1000),
            latencies[latencies.len() - 1]
        );
    }
    assert_eq!(
        commit_errors + correctness_errors,
        0,
        "the workload failed its checks"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let number = |i: usize| -> u64 { args[i].parse().unwrap() };
    let path = |i: usize| Path::new(&args[i]);
    match (args.get(1).map(String::as_str), args.len()) {
        (Some("build"), 7) => build(
            path(2),
            match args[3].as_str() {
                "keyed" => true,
                "unkeyed" => false,
                other => panic!("keyed or unkeyed, not {other}"),
            },
            number(4) as usize,
            number(5),
            number(6),
        ),
        (Some("open"), 5) => open(
            path(2),
            match args[3].as_str() {
                "cold" => true,
                "warm" => false,
                other => panic!("cold or warm, not {other}"),
            },
            number(4) as usize,
        ),
        (Some("append"), 5 | 6) => append(
            path(2),
            number(3),
            number(4),
            (args.len() == 6).then(|| Duration::from_micros(number(5))),
        ),
        _ => panic!(
            "usage: sealed_probe build NEW_FIXTURE keyed|unkeyed SEALED_SEGMENTS ROLL_BYTES ACTIVE_BYTES\n       sealed_probe open FIXTURE cold|warm RUNS\n       sealed_probe append FIXTURE CALLERS ROUNDS [INTERVAL_US]"
        ),
    }
}
