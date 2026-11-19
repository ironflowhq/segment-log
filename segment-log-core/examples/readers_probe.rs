//! Several readers beside several appenders on a large history (issue 156), through the public
//! API only.
//!
//! usage:
//!   readers_probe build DIR RECORDS STREAMS
//!   readers_probe run DIR APPENDERS READERS tail|random|scan SECONDS BATCH
//!   readers_probe seal DIR
//!   readers_probe sealed DIR READERS random|scan SECONDS BATCH
//!
//! `build` writes RECORDS keyed 256-byte records round-robin over STREAMS streams, under the
//! default options, so the history spans many sealed segments. `run` reopens the log, then for
//! SECONDS runs APPENDERS closed-loop callers (one keyed 256-byte record per submit, each on its
//! own new stream, as in the M1 benchmark) beside READERS threads that call `read(from, BATCH)`:
//!
//! - `tail` follows the watermark from where the run started, and sleeps 200 µs when it has read
//!   everything durable, as a polling consumer would, so it does not hold a core while it waits;
//! - `random` reads at a uniformly random offset below the starting watermark, so mostly in
//!   sealed segments, more of them than the 16 frame tables a log keeps;
//! - `scan` reads the history from offset 0 in order, and starts again at the end.
//!
//! `sealed` reads a sealed log through `LogReader`, which opens only a sealed log, so it cannot
//! run beside appenders on the same log (api.md). Every read checks that its records are
//! contiguous from `from` and carry 256-byte payloads.
//!
//! Output: one `APPEND` line and one `READ` line per run, with latency quantiles in µs.

use segment_log_core::*;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const PAYLOAD: usize = 256;
const BUILD_GROUP: usize = 256;
const BUILD_WINDOW: usize = 16;
const TAIL_POLL: Duration = Duration::from_micros(200);

fn usage() -> ! {
    eprintln!(
        "usage:\n  readers_probe build DIR RECORDS STREAMS\n  readers_probe run DIR APPENDERS READERS tail|random|scan SECONDS BATCH\n  readers_probe seal DIR\n  readers_probe sealed DIR READERS random|scan SECONDS BATCH"
    );
    std::process::exit(2)
}

fn number(args: &[String], i: usize) -> usize {
    args.get(i)
        .and_then(|a| a.parse().ok())
        .unwrap_or_else(|| usage())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match (args.get(1).map(String::as_str), args.len()) {
        (Some("build"), 5) => build(Path::new(&args[2]), number(&args, 3), number(&args, 4)),
        (Some("run"), 8) => run(
            Path::new(&args[2]),
            number(&args, 3),
            number(&args, 4),
            &args[5],
            number(&args, 6),
            number(&args, 7),
        ),
        (Some("seal"), 3) => {
            let log = SegmentLog::open(&args[2], Options::default()).expect("open");
            log.seal().expect("seal");
            println!("SEAL watermark={}", log.watermark());
        }
        (Some("sealed"), 7) => sealed(
            Path::new(&args[2]),
            number(&args, 3),
            &args[4],
            number(&args, 5),
            number(&args, 6),
        ),
        _ => usage(),
    }
}

fn build(dir: &Path, records: usize, streams: usize) {
    assert!(!dir.exists(), "build needs a new directory");
    assert!(streams > 0 && streams <= records);
    let log = SegmentLog::open(dir, Options::default()).expect("open");
    let started = Instant::now();
    let record = |i: usize| NewRecord {
        stream: (i % streams).to_le_bytes().to_vec(),
        expected: if i < streams {
            ExpectedRevision::Empty
        } else {
            ExpectedRevision::At((i / streams - 1) as u64)
        },
        key: Some(format!("h{i}").into_bytes()),
        payload: vec![(i % 251) as u8; PAYLOAD],
    };
    // A window of pending submits keeps the writer's groups full without a thread per caller.
    let mut window = std::collections::VecDeque::new();
    let mut next = 0;
    while next < records || !window.is_empty() {
        while next < records && window.len() < BUILD_WINDOW {
            let count = BUILD_GROUP.min(records - next);
            let submit = Submit {
                records: (next..next + count).map(record).collect(),
            };
            window.push_back((next, count, log.append(submit).expect("append")));
            next += count;
        }
        let (from, count, pending) = window.pop_front().unwrap();
        let committed = pending.wait().expect("commit");
        assert_eq!(committed.records.len(), count);
        assert_eq!(committed.records[0].offset, from as u64);
    }
    println!(
        "BUILD records={records} streams={streams} seconds={:.1}",
        started.elapsed().as_secs_f64()
    );
}

/// Microsecond quantiles of a set of latencies in nanoseconds.
fn quantiles(mut ns: Vec<u64>) -> String {
    if ns.is_empty() {
        return "p50_us=- p99_us=- p999_us=- max_us=-".into();
    }
    ns.sort_unstable();
    let q = |p: f64| ns[((ns.len() - 1) as f64 * p) as usize] as f64 / 1000.0;
    format!(
        "p50_us={:.1} p99_us={:.1} p999_us={:.1} max_us={:.1}",
        q(0.5),
        q(0.99),
        q(0.999),
        q(1.0)
    )
}

/// A small xorshift, so the probe needs no dependency.
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

#[derive(Default)]
struct ReaderResult {
    latencies: Vec<u64>,
    records: usize,
    empty: usize,
}

/// One reader's loop. `read` is `SegmentLog::read` or `LogReader::read`; `top` gives the offset
/// below which it reads.
fn reader(
    id: usize,
    pattern: &str,
    batch: usize,
    history: u64,
    stop: &AtomicBool,
    top: &dyn Fn() -> u64,
    read: &dyn Fn(u64, usize) -> Result<Vec<Record>, ReadError>,
) -> ReaderResult {
    let mut r = ReaderResult::default();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ (id as u64 + 1));
    // Scans start spread over the history, so several scanners do not read the same pages.
    let mut cursor = match pattern {
        "tail" => history,
        "scan" => history / 16 * (id as u64 % 16),
        "random" => 0,
        _ => usage(),
    };
    while !stop.load(Ordering::Relaxed) {
        let from = match pattern {
            "random" => rng.below(history.saturating_sub(batch as u64)),
            "scan" if cursor >= history => {
                cursor = 0;
                0
            }
            _ => cursor,
        };
        if pattern == "tail" && from >= top() {
            thread::sleep(TAIL_POLL);
            continue;
        }
        let started = Instant::now();
        let records = read(from, batch).expect("read");
        r.latencies.push(started.elapsed().as_nanos() as u64);
        for (i, rec) in records.iter().enumerate() {
            assert_eq!(rec.offset, from + i as u64, "read is not contiguous");
            assert_eq!(rec.payload.len(), PAYLOAD, "payload length");
        }
        if records.is_empty() {
            r.empty += 1;
        }
        r.records += records.len();
        cursor = from + records.len() as u64;
    }
    r
}

fn report_reads(
    kind: &str,
    readers: usize,
    pattern: &str,
    batch: usize,
    secs: f64,
    results: Vec<ReaderResult>,
) {
    let calls: usize = results.iter().map(|r| r.latencies.len()).sum();
    let records: usize = results.iter().map(|r| r.records).sum();
    let empty: usize = results.iter().map(|r| r.empty).sum();
    let latencies = results.into_iter().flat_map(|r| r.latencies).collect();
    println!(
        "READ kind={kind} readers={readers} pattern={pattern} batch={batch} calls={calls} empty={empty} records={records} records_per_s={:.0} {}",
        records as f64 / secs,
        quantiles(latencies)
    );
}

fn run(dir: &Path, appenders: usize, readers: usize, pattern: &str, seconds: usize, batch: usize) {
    assert!(dir.exists(), "run needs a built log");
    let opened = Instant::now();
    let log = SegmentLog::open(dir, Options::default()).expect("open");
    let history = log.watermark();
    println!(
        "OPEN watermark={history} ms={:.1}",
        opened.elapsed().as_secs_f64() * 1000.0
    );
    let stop = AtomicBool::new(false);
    let started = Instant::now();
    let (appends, reads) = thread::scope(|s| {
        let log = &log;
        let stop = &stop;
        let appends: Vec<_> = (0..appenders)
            .map(|t| {
                s.spawn(move || {
                    // The starting watermark makes streams and keys new in every run.
                    let stream = format!("a{history}-{t}").into_bytes();
                    let mut latencies = Vec::new();
                    let mut full = 0usize;
                    let mut n = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        let submit = Submit {
                            records: vec![NewRecord {
                                stream: stream.clone(),
                                expected: if n == 0 {
                                    ExpectedRevision::Empty
                                } else {
                                    ExpectedRevision::At(n - 1)
                                },
                                key: Some(format!("a{history}-{t}-{n}").into_bytes()),
                                payload: vec![(n % 251) as u8; PAYLOAD],
                            }],
                        };
                        let t0 = Instant::now();
                        match log.append(submit) {
                            Ok(pending) => {
                                let c = pending.wait().expect("commit");
                                assert_eq!(c.records[0].revision, n);
                                latencies.push(t0.elapsed().as_nanos() as u64);
                                n += 1;
                            }
                            Err(AppendError::Full) => {
                                full += 1;
                                thread::yield_now();
                            }
                            Err(e) => panic!("append: {e:?}"),
                        }
                    }
                    (latencies, full)
                })
            })
            .collect();
        let reads: Vec<_> = (0..readers)
            .map(|id| {
                s.spawn(move || {
                    reader(
                        id,
                        pattern,
                        batch,
                        history,
                        stop,
                        &|| log.watermark(),
                        &|from, max| log.read(from, max),
                    )
                })
            })
            .collect();
        thread::sleep(Duration::from_secs(seconds as u64));
        stop.store(true, Ordering::Relaxed);
        (
            appends
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>(),
            reads
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>(),
        )
    });
    let secs = started.elapsed().as_secs_f64();
    let full: usize = appends.iter().map(|a| a.1).sum();
    let latencies: Vec<u64> = appends.into_iter().flat_map(|a| a.0).collect();
    println!(
        "APPEND appenders={appenders} readers={readers} pattern={pattern} records={} records_per_s={:.0} full={full} {}",
        latencies.len(),
        latencies.len() as f64 / secs,
        quantiles(latencies)
    );
    report_reads("live", readers, pattern, batch, secs, reads);
}

fn sealed(dir: &Path, readers: usize, pattern: &str, seconds: usize, batch: usize) {
    assert!(pattern != "tail", "a sealed log has no tail to follow");
    let reader_log = LogReader::open(dir).expect("LogReader::open");
    let history = reader_log.watermark();
    let stop = AtomicBool::new(false);
    let started = Instant::now();
    let reads = thread::scope(|s| {
        let log = &reader_log;
        let stop = &stop;
        let handles: Vec<_> = (0..readers)
            .map(|id| {
                s.spawn(move || {
                    reader(
                        id,
                        pattern,
                        batch,
                        history,
                        stop,
                        &|| history,
                        &|from, max| log.read(from, max),
                    )
                })
            })
            .collect();
        thread::sleep(Duration::from_secs(seconds as u64));
        stop.store(true, Ordering::Relaxed);
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    report_reads(
        "sealed",
        readers,
        pattern,
        batch,
        started.elapsed().as_secs_f64(),
        reads,
    );
}
