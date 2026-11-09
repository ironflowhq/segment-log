use crate::{
    engine::admission_support,
    storage::{Call, Lock, Op, Purpose, Storage, Value, real::Real},
    *,
};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

mod measured;
use measured::Measured;

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let root = std::env::var_os("SEGMENT_LOG_EVIDENCE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        fs::create_dir_all(&root).unwrap();
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = root.join(format!(
            "evidence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("use a fresh evidence directory");
        // Deployment owns durable ancestors. Provision them outside the engine
        // before testing the log's own directory-sync protocol.
        for ancestor in path.ancestors() {
            fs::File::open(ancestor).unwrap().sync_all().unwrap();
        }
        Self(path)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
struct Watchdog(mpsc::Sender<()>, Option<thread::JoinHandle<()>>);
impl Watchdog {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self(
            tx,
            Some(thread::spawn(move || {
                if rx.recv_timeout(Duration::from_secs(300)).is_err() {
                    eprintln!("evidence watchdog expired");
                    std::process::abort();
                }
            })),
        )
    }
}
impl Drop for Watchdog {
    fn drop(&mut self) {
        let _ = self.0.send(());
        self.1.take().unwrap().join().unwrap();
    }
}

fn input(caller: usize, revision: usize) -> Submit {
    let mut payload = vec![0x5a; 256];
    payload[..8].copy_from_slice(&(caller as u64).to_le_bytes());
    payload[8..16].copy_from_slice(&(revision as u64).to_le_bytes());
    Submit {
        records: vec![NewRecord {
            stream: (caller as u64).to_le_bytes().to_vec(),
            key: Some(payload[..16].to_vec()),
            payload,
            expected: if revision == 0 {
                ExpectedRevision::Empty
            } else {
                ExpectedRevision::At(revision as u64 - 1)
            },
        }],
    }
}

fn validate(log: &SegmentLog, positions: &[Vec<u64>]) {
    let n = positions.iter().map(Vec::len).sum::<usize>();
    assert_eq!(log.watermark(), n as u64);
    let records = log.read(0, n + 1).unwrap();
    assert_eq!(records.len(), n);
    for (caller, offsets) in positions.iter().enumerate() {
        for (revision, &offset) in offsets.iter().enumerate() {
            let expected = input(caller, revision).records.remove(0);
            let actual = &records[offset as usize];
            assert_eq!((actual.offset, actual.revision), (offset, revision as u64));
            assert_eq!(
                (&actual.stream, &actual.key, &actual.payload),
                (&expected.stream, &expected.key, &expected.payload)
            );
        }
        let head = log.stream_head(&(caller as u64).to_le_bytes()).unwrap();
        assert_eq!(
            (head.offset, head.revision),
            (*offsets.last().unwrap(), offsets.len() as u64 - 1)
        );
    }
}
fn mark(phase: &str) {
    io::stderr()
        .write_all(format!("PHASE {phase}\n").as_bytes())
        .unwrap();
    io::stderr().flush().unwrap();
}

/// Given actual storage in a fresh directory
/// When creation, two new commits, a durable retry, Drop and recovery run
/// Then both directories sync successfully and the retry adds no group or flush
/// Cases: GROUP-FORMATION, OPEN-DIR, ORACLE-12, RETRY-REGROUPED
#[test]
fn real_storage_sync_phases() {
    let _watchdog = Watchdog::new();
    mark("provision");
    let workspace = Workspace::new();
    let path = workspace.0.join("log");
    let storage = Arc::new(Measured::new(true));
    mark("create");
    let log = SegmentLog::with_storage(&path, Options::default(), storage.clone()).unwrap();
    storage.assert_directories(&path, 2, 1);
    let mut positions = vec![vec![]];
    for revision in 0..2 {
        mark(&format!("commit-{revision}"));
        let result = log.append(input(0, revision)).unwrap().wait().unwrap();
        assert!(!result.deduplicated);
        positions[0].push(result.records[0].offset);
    }
    assert_eq!(storage.groups().len(), 2);
    assert_eq!(storage.group_flushes(), 2);
    mark("retry");
    let retry = log.append(input(0, 0)).unwrap().wait().unwrap();
    assert!(retry.deduplicated);
    assert_eq!((retry.records[0].offset, retry.records[0].revision), (0, 0));
    assert_eq!(storage.groups().len(), 2);
    assert_eq!(storage.group_flushes(), 2);
    mark("drop");
    drop(log);
    mark("recovery");
    let recovered = Arc::new(Measured::new(true));
    let log = SegmentLog::with_storage(&path, Options::default(), recovered.clone()).unwrap();
    recovered.assert_directories(&path, 1, 1);
    validate(&log, &positions);
    assert_eq!(recovered.group_flushes(), 0);
    mark("recovered-drop");
    drop(log);
    mark("done");
}

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |s| s.parse().unwrap())
}
fn quantile(values: &[u64], numerator: usize, denominator: usize) -> u64 {
    values[(values.len() * numerator)
        .div_ceil(denominator)
        .saturating_sub(1)]
}
fn distribution(name: &str, mut values: Vec<u64>) {
    values.sort_unstable();
    println!(
        "METRIC {name} count={} min={} p50={} p99={} p999={} max={} sum={}",
        values.len(),
        values[0],
        quantile(&values, 50, 100),
        quantile(&values, 99, 100),
        quantile(&values, 999, 1000),
        values.last().unwrap(),
        values.iter().sum::<u64>()
    );
}
fn size(path: &Path) -> u64 {
    fs::metadata(path.join("00000000000000000000.seg"))
        .unwrap()
        .len()
}

/// Given literal keyed records, default options and actual storage
/// When callers wait for every result, then close and reopen the whole log
/// Then every acknowledged offset, revision, key and payload survives
/// Cases: ORACLE-1, ORACLE-12, ADM-CHARGE, GROUP-FORMATION
#[test]
fn measured_appends_survive_reopen() {
    let _watchdog = Watchdog::new();
    let workspace = Workspace::new();
    let callers = setting("SEGMENT_LOG_BENCH_CALLERS", 4);
    let rounds = setting("SEGMENT_LOG_BENCH_ROUNDS", 4);
    let device_rounds = setting("SEGMENT_LOG_DEVICE_ROUNDS", 8);
    assert!(callers > 0 && rounds > 0 && device_rounds > 0);
    let options = Options::default();
    println!(
        "CONFIG callers={callers} rounds={rounds} payload=256 records_per_submit=1 seed=none_literal_formula linger_ns={} group_byte_cap={} roll_bytes={} queue_capacity={} pending_byte_budget={}",
        options.linger.as_nanos(),
        options.group_byte_cap,
        options.roll_bytes,
        options.queue_capacity,
        options.pending_byte_budget
    );
    // Growing-file appends include size metadata in every data flush (testing.md).
    let real = Real::default();
    let device = workspace.0.join("device");
    real.call(Call::new(&device, Purpose::Open, Op::Create))
        .unwrap();
    let mut flushes = Vec::new();
    for _ in 0..device_rounds {
        real.call(Call::new(
            &device,
            Purpose::Group,
            Op::Append(vec![0x5a; 256 * 1024]),
        ))
        .unwrap();
        let now = Instant::now();
        real.call(Call::new(&device, Purpose::Group, Op::SyncData))
            .unwrap();
        flushes.push(now.elapsed().as_nanos() as u64);
    }
    distribution("device_fdatasync_ns", flushes);
    println!(
        "DEVICE start_bytes=0 end_bytes={} append_bytes=262144 failures=0",
        fs::metadata(device).unwrap().len()
    );
    let path = workspace.0.join("log");
    let storage = Arc::new(Measured::new(false));
    let now = Instant::now();
    let log =
        Arc::new(SegmentLog::with_storage(&path, Options::default(), storage.clone()).unwrap());
    println!(
        "OPEN initial_ns={} records=0 segments=1 bytes={}",
        now.elapsed().as_nanos(),
        size(&path)
    );
    let (latencies, positions, refusals, admission_ns, elapsed) = thread::scope(|scope| {
        let (ready_tx, ready_rx) = mpsc::channel();
        let mut gates = Vec::new();
        let mut workers = Vec::new();
        for caller in 0..callers {
            let log = log.clone();
            let (gate, start) = mpsc::channel();
            let ready = ready_tx.clone();
            let worker = thread::Builder::new()
                .name(format!("caller-{caller}"))
                .spawn_scoped(scope, move || {
                    ready.send(()).unwrap();
                    if start.recv().is_err() {
                        return None;
                    }
                    let mut latencies = Vec::with_capacity(rounds);
                    let mut offsets = Vec::with_capacity(rounds);
                    let mut full = 0;
                    let mut admission_ns = 0;
                    for revision in 0..rounds {
                        let now = Instant::now();
                        let pending = loop {
                            let request = input(caller, revision);
                            let admission = Instant::now();
                            let result = log.append(request);
                            admission_ns += admission.elapsed().as_nanos() as u64;
                            match result {
                                Ok(p) => break p,
                                Err(AppendError::Full) => {
                                    full += 1;
                                    thread::yield_now();
                                }
                                Err(error) => panic!("admission failure: {error:?}"),
                            }
                        };
                        let result = pending.wait().expect("commit failure");
                        latencies.push(now.elapsed().as_nanos() as u64);
                        assert!(!result.deduplicated);
                        assert_eq!(result.records.len(), 1);
                        assert_eq!(result.records[0].revision, revision as u64);
                        offsets.push(result.records[0].offset);
                    }
                    Some((latencies, offsets, full, admission_ns))
                });
            match worker {
                Ok(worker) => {
                    gates.push(gate);
                    workers.push(worker);
                }
                Err(error) => {
                    eprintln!(
                        "SPAWN_FAILURE requested={callers} started={} error={error}",
                        workers.len()
                    );
                    drop(gates);
                    for worker in workers {
                        worker.join().unwrap();
                    }
                    panic!("caller resource limit");
                }
            }
        }
        for _ in 0..callers {
            ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        }
        let now = Instant::now();
        for gate in gates {
            gate.send(()).unwrap();
        }
        let mut latencies = Vec::new();
        let mut positions = Vec::new();
        let mut refusals = 0;
        let mut admission_ns = 0;
        for worker in workers {
            let (times, offsets, full, admission) = worker.join().unwrap().unwrap();
            latencies.extend(times);
            positions.push(offsets);
            refusals += full;
            admission_ns += admission;
        }
        let elapsed = now.elapsed();
        (latencies, positions, refusals, admission_ns, elapsed)
    });
    let peak = admission_support::peak(&log);
    assert_eq!(admission_support::used(&log), 0);
    validate(&log, &positions);
    let groups = storage.groups();
    assert_eq!(groups.len(), storage.group_flushes());
    assert_eq!(
        groups.iter().map(|g| g.0).sum::<u64>(),
        (callers * rounds) as u64
    );
    assert_eq!(64 + groups.iter().map(|g| g.1).sum::<u64>(), size(&path));
    println!(
        "APPEND records={} elapsed_ns={} records_per_second={:.3} full={} other_admission_errors=0 commit_errors=0 correctness_errors=0 peak_reserved_bytes={peak} admission_call_sum_ns={admission_ns}",
        callers * rounds,
        elapsed.as_nanos(),
        (callers * rounds) as f64 / elapsed.as_secs_f64(),
        refusals
    );
    distribution("append_ns", latencies);
    distribution("group_submits", groups.iter().map(|g| g.0).collect());
    distribution("group_frame_bytes", groups.iter().map(|g| g.1).collect());
    storage.report();
    if let Some(out) = std::env::var_os("SEGMENT_LOG_GROUPS_OUT") {
        let mut file = fs::File::create(out).unwrap();
        writeln!(file, "group\tsubmits\tframe_bytes").unwrap();
        for (index, (submits, bytes)) in groups.iter().enumerate() {
            writeln!(file, "{index}\t{submits}\t{bytes}").unwrap();
        }
    }
    println!(
        "END records={} segments=1 bytes={}",
        log.watermark(),
        size(&path)
    );
    drop(log);
    let before = size(&path);
    let now = Instant::now();
    let log = SegmentLog::open(&path, Options::default()).unwrap();
    let open_ns = now.elapsed().as_nanos();
    validate(&log, &positions);
    assert_eq!(before, size(&path));
    println!(
        "REOPEN ns={open_ns} start_records={} end_records={} start_segments=1 end_segments=1 start_bytes={before} end_bytes={} correctness_errors=0",
        callers * rounds,
        log.watermark(),
        size(&path)
    );
    drop(log);
}
