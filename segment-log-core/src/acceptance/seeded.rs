use super::support::{
    fixtures::{ANY_SEG, SEG, Sim, options},
    oracle::{self, Answer, Input, Observation, Request},
    trace,
};
use crate::storage::{
    Call, Op, Purpose,
    sim::{Crash, Event, Fault},
};
use crate::*;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};
mod actors;
mod saved;
mod workloads;
use actors::{Caller, Command, Reader, observe, watchdog, watchdog_with_failure};
use workloads::{pressure, workload};

struct Ledger {
    seed: u64,
    mode: Crash,
    options: Options,
    requests: Vec<Request>,
    reads: Vec<Observation>,
}
fn opts(o: &Options) -> Options {
    Options {
        linger: o.linger,
        group_byte_cap: o.group_byte_cap,
        queue_capacity: o.queue_capacity,
        pending_byte_budget: o.pending_byte_budget,
        roll_bytes: o.roll_bytes,
    }
}
fn open(sim: &Sim, o: &Options) -> SegmentLog {
    super::support::fixtures::open_with(sim, opts(o))
}
fn flush() -> Call {
    Call::new(ANY_SEG, Purpose::Group, Op::SyncData)
}
fn literal(id: u64, count: usize, size: usize, keying: u64) -> Vec<Input> {
    (0..count)
        .map(|i| Input {
            stream: format!("s{id}").into_bytes(),
            key: (keying == 0 || (keying == 1 && i == 0))
                .then(|| format!("k{id}:{i}").into_bytes()),
            payload: (0..size)
                .map(|p| (p as u64 ^ id ^ i as u64) as u8)
                .collect(),
            expected: i.checked_sub(1).map(|v| v as u64),
            spare: 0,
        })
        .collect()
}

/// Given literal caller answers and an independent reader's observed records and heads
/// When paired replay metadata is saved and loaded
/// Then all reader observations and request fields survive exactly
/// Cases: SIM-MODEL, ORACLE-8, ORACLE-9
#[test]
fn saved_metadata_retains_reader_observations() {
    let _watchdog = watchdog("metadata round-trip".into());
    let inputs = literal(42, 2, 19, 1);
    let ledger = Ledger {
        seed: 9,
        mode: Crash::PowerLossTorn,
        options: options(),
        requests: vec![Request {
            inputs: inputs.clone(),
            original: true,
            answer: Answer::Ack(false, vec![(0, 0), (1, 1)]),
            watermark: 2,
        }],
        reads: vec![Observation {
            before: 2,
            after: 2,
            from: 0,
            max: 3,
            records: inputs
                .iter()
                .enumerate()
                .map(|(i, r)| r.stored(i as u64, i as u64))
                .collect(),
            heads: vec![(b"s42".to_vec(), Some((1, 1)))],
        }],
    };
    let path = std::env::temp_dir().join(format!("sl9-metadata-{}", std::process::id()));
    saved::save(&path, &ledger);
    let loaded = saved::load(&path);
    std::fs::remove_file(path).unwrap();
    assert_eq!(loaded.requests, ledger.requests);
    assert_eq!((loaded.seed, loaded.mode), (ledger.seed, ledger.mode));
    assert_eq!(
        (
            loaded.options.group_byte_cap,
            loaded.options.queue_capacity,
            loaded.options.pending_byte_budget,
            loaded.options.linger,
            loaded.options.roll_bytes
        ),
        (
            ledger.options.group_byte_cap,
            ledger.options.queue_capacity,
            ledger.options.pending_byte_budget,
            ledger.options.linger,
            ledger.options.roll_bytes
        )
    );
    assert_eq!(
        loaded.reads.len(),
        1,
        "saved replay must retain independent reader observations"
    );
    super::support::fixtures::assert_records(&loaded.reads[0].records, &ledger.reads[0].records);
    assert_eq!(loaded.reads[0].heads, ledger.reads[0].heads);
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = self.0;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn shuffle<T>(&mut self, xs: &mut [T]) {
        for i in (1..xs.len()).rev() {
            xs.swap(i, self.pick(i + 1));
        }
    }
}
fn body(inputs: &[Input]) -> usize {
    24 + inputs
        .iter()
        .map(|r| 16 + r.stream.len() + r.key.as_ref().map_or(0, Vec::len) + r.payload.len())
        .sum::<usize>()
}
fn frame(base: u64, inputs: &[Input], prior: &[Record]) -> Vec<u8> {
    let mut history: Vec<_> = prior
        .iter()
        .map(|r| Record {
            offset: r.offset,
            revision: r.revision,
            stream: r.stream.clone(),
            key: r.key.clone(),
            payload: r.payload.clone(),
        })
        .collect();
    assert!(matches!(
        oracle::predict(inputs, &mut history),
        Answer::Ack(false, _)
    ));
    let mut body = vec![1, 0, 0, 0];
    body.extend(1u64.to_le_bytes());
    body.extend(base.to_le_bytes());
    body.extend((inputs.len() as u32).to_le_bytes());
    for (r, p) in inputs.iter().zip(&history[prior.len()..]) {
        body.extend((r.stream.len() as u16).to_le_bytes());
        body.extend((r.key.as_ref().map_or(0, Vec::len) as u16).to_le_bytes());
        body.extend((r.payload.len() as u32).to_le_bytes());
        body.extend(p.revision.to_le_bytes());
        body.extend(&r.stream);
        if let Some(k) = &r.key {
            body.extend(k)
        }
        body.extend(&r.payload);
    }
    let len = (body.len() as u32).to_le_bytes();
    let crc = super::support::raw::crc(&[len.as_slice(), &body].concat());
    [len.as_slice(), &crc.to_le_bytes(), &body].concat()
}

fn check_replay(sim: &Sim, ledger: &Ledger) {
    let (_, events) = sim.trace();
    let (written, ends) = oracle::written(&events, &ledger.requests);
    oracle::flush_contract(&ledger.requests, &ledger.reads, &events);
    oracle::answers(&ledger.requests, &events, &ledger.options);
    let valid = oracle::valid_length(&super::support::raw::segments(sim));
    let log = open(sim, &ledger.options);
    let kept = log.watermark() as usize;
    assert_eq!(
        kept, valid,
        "recovery retains the entire readable valid prefix"
    );
    assert!(
        ends.contains(&kept),
        "recovery must end at a written atomic group boundary: {kept} {ends:?}"
    );
    oracle::history(&log, &written[..kept]);
    oracle::observations(&ledger.requests, &ledger.reads, &written[..kept], &ends);
    assert!(matches!(
        SegmentLog::with_storage(
            Path::new("/log"),
            opts(&ledger.options),
            Arc::new(sim.clone())
        ),
        Err(OpenError::Locked)
    ));
    drop(log);
    sim.crash(Crash::PowerLoss, ledger.seed.wrapping_add(1));
    let log = open(sim, &ledger.options);
    oracle::history(&log, &written[..kept]);
    drop(log);
    sim.crash(Crash::PowerLossTorn, ledger.seed.wrapping_add(2));
    let log = open(sim, &ledger.options);
    let mut expected: Vec<_> = written.into_iter().take(kept).collect();
    oracle::history(&log, &expected);
    for q in &ledger.requests {
        if q.answer == Answer::Unknown {
            let answer = oracle::predict(&q.inputs, &mut expected);
            assert_eq!(
                oracle::execute(&log, &q.inputs),
                answer,
                "unknown replay {q:?}"
            );
            oracle::history(&log, &expected);
        }
    }
    let fresh = literal(99, 2, 3, 0);
    let answer = oracle::predict(&fresh, &mut expected);
    assert_eq!(oracle::execute(&log, &fresh), answer);
    oracle::history(&log, &expected);
    drop(log);
    let log = open(sim, &ledger.options);
    oracle::history(&log, &expected);
    for q in &ledger.requests {
        if q.original
            && q.inputs.iter().all(|r| r.key.is_some())
            && q.inputs
                .iter()
                .all(|r| expected.iter().any(|e| r.matches(e)))
        {
            assert_eq!(
                oracle::execute(&log, &q.inputs),
                oracle::predict(&q.inputs, &mut expected)
            );
        }
    }
    oracle::history(&log, &expected);
}

fn artifact(seed: u64, mode: Crash, variant: usize) -> PathBuf {
    let root = std::env::var_os("SEGMENT_LOG_TRACE_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("segment-log-traces"));
    root.join(format!("{}-{seed}-{mode:?}-{variant}", std::process::id()))
}
fn save_pair(path: &Path, sim: &Sim, ledger: &Ledger) {
    let (seed, events) = sim.trace();
    trace::save(&path.with_extension("trace"), seed, &events).unwrap();
    saved::save(&path.with_extension("ledger"), ledger);
}
/// A record that the oracle must reject: a permanent refusal of a request that fits every limit.
fn false_refusal() -> Request {
    Request {
        inputs: literal(2, 2, 7, 0),
        original: true,
        answer: Answer::Refused("TooLarge".into()),
        watermark: 0,
    }
}
/// Whether the checks of this process plant the false refusal. A hosted drill sets it to prove
/// that a failed check leaves a pair that can be fetched and replayed to the same rejection.
fn drill() -> bool {
    match std::env::var("SEGMENT_LOG_TRACE_DRILL").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("0") => false,
        Ok("1") => true,
        other => panic!("invalid SEGMENT_LOG_TRACE_DRILL: {other:?}"),
    }
}
fn replay(path: &Path, drill: bool) {
    let mut ledger = saved::load(&path.with_extension("ledger"));
    if drill {
        ledger.requests.push(false_refusal());
    }
    let (seed, events) = trace::load(&path.with_extension("trace")).unwrap();
    let sim = Sim::replay(seed, &events);
    check_replay(&sim, &ledger);
}
fn run(seed: u64, mode: Crash, variant: usize, keep: bool, drill: bool) -> PathBuf {
    let path = artifact(seed, mode, variant);
    let mut options = options();
    if seed % 2 == 1 {
        options.roll_bytes = EVERY_GROUP;
    }
    let label = format!(
        "cases=ORACLE-1..12 seed={seed} mode={mode:?} variant={variant} roll_bytes={} artifacts={}",
        options.roll_bytes,
        path.display()
    );
    let sim = Sim::new(seed);
    let ledger = Arc::new(Mutex::new(Ledger {
        seed,
        mode,
        options,
        requests: vec![],
        reads: vec![],
    }));
    let watcher_sim = sim.clone();
    let watcher_ledger = ledger.clone();
    let watcher_path = path.clone();
    let _watchdog = watchdog_with_failure(label.clone(), move || {
        if let Ok(ledger) = watcher_ledger.try_lock() {
            save_pair(&watcher_path, &watcher_sim, &ledger);
        }
    });
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if variant == 8 {
            pressure(&sim, &ledger)
        } else {
            workload(&sim, &ledger, variant)
        }
    }));
    save_pair(
        &path,
        &sim,
        &ledger.lock().unwrap_or_else(|e| e.into_inner()),
    );
    if let Err(panic) = result {
        eprintln!("{label}");
        std::panic::resume_unwind(panic);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| replay(&path, drill)));
    if let Err(panic) = result {
        eprintln!("{label}");
        std::panic::resume_unwind(panic);
    }
    if !keep {
        std::fs::remove_file(path.with_extension("trace")).unwrap();
        std::fs::remove_file(path.with_extension("ledger")).unwrap();
    }
    path
}
/// The roll size of odd seeds. Every segment that holds a frame has reached it, so each group
/// starts a new segment.
const EVERY_GROUP: u64 = 65;
fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name).map_or(default, |v| v.parse().expect("numeric trace setting"))
}
fn suite(default: u64) {
    let first = setting("SEGMENT_LOG_TRACE_SEED", 900);
    let count = setting("SEGMENT_LOG_TRACE_SEEDS", default);
    assert!((1..=10000).contains(&count));
    let drill = drill();
    for seed in first..first.checked_add(count).unwrap() {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            for variant in 0..9 {
                run(seed, mode, variant, false, drill);
            }
        }
    }
    eprintln!(
        "trace seeds={first}..={} scenarios={} (9 workloads x 3 crash modes per seed)",
        first + count - 1,
        count * 27
    );
}

/// Given seeded caller inputs, staged pending and durable retries, refusals and independent readers
/// When actual threads hit held writes, flushes and lookups, with crashes or injected failures
/// Then literal inputs and written order bound atomic recovery, unknown retries and all observations
/// And same-boot fault recovery, chained power loss and cache reconstruction preserve the contract
/// Cases: ORACLE-1, ORACLE-2, ORACLE-3, ORACLE-4, ORACLE-5, ORACLE-6, ORACLE-7, ORACLE-8, ORACLE-9, ORACLE-10, ORACLE-11, ORACLE-12, RETRY-PENDING, RETRY-REGROUPED, KEY-CONFLICT-CRASH, KEY-PARTIAL-CRASH, READ-BOUNDED, ADM-BUDGETS, ADM-SIZE
#[test]
fn threaded_seeded_traces_preserve_the_contract() {
    suite(2);
}

/// Given the same threaded oracle with a larger seed budget
/// When the scheduled Linux and macOS job invokes the ignored test explicitly
/// Then every generated trace is checked and failures retain paired replay artifacts
/// Cases: SIM-MODEL
#[test]
#[ignore = "long run"]
fn threaded_seeded_traces_long_run() {
    suite(100);
}

/// Given a bounded threaded trace or a saved failure selected by environment
/// When storage events and literal caller/reader ledgers are loaded in a fresh simulator
/// Then recovery and the oracle rerun without reproducing caller scheduling
/// Cases: SIM-MODEL, ORACLE-1, ORACLE-8, ORACLE-12
#[test]
fn saved_trace_replays_the_oracle() {
    let _watchdog = watchdog("saved oracle replay".into());
    if let Some(path) = std::env::var_os("SEGMENT_LOG_TRACE_REPLAY") {
        let path = PathBuf::from(path);
        let _watchdog = watchdog(format!("replay {}", path.display()));
        replay(&path, drill());
    } else {
        let path = run(919, Crash::PowerLoss, 6, true, false);
        replay(&path, false);
        if std::env::var_os("SEGMENT_LOG_TRACE_KEEP").is_none() {
            std::fs::remove_file(path.with_extension("trace")).unwrap();
            std::fs::remove_file(path.with_extension("ledger")).unwrap();
        } else {
            eprintln!("replay artifact {}", path.display());
        }
    }
}

/// Given a threaded trace that passes, and a drill that plants one false refusal in its ledger
/// When the run checks the pair, and the retained pair is then replayed with and without the drill
/// Then the run and the drill replay fail, the plain replay passes, and the pair stays on disk
/// Cases: SIM-MODEL, ADM-SIZE
#[test]
fn a_drill_failure_retains_a_pair_that_replays_the_rejection() {
    let _watchdog = watchdog("drill".into());
    let path = artifact(920, Crash::Kill, 0);
    let failed = std::panic::catch_unwind(|| run(920, Crash::Kill, 0, false, true));
    assert!(failed.is_err(), "the planted refusal passed the run");
    replay(&path, false);
    let replayed = std::panic::catch_unwind(|| replay(&path, true));
    assert!(replayed.is_err(), "the planted refusal passed the replay");
    std::fs::remove_file(path.with_extension("trace")).unwrap();
    std::fs::remove_file(path.with_extension("ledger")).unwrap();
}

/// Given an independent reader and one durable record
/// When a simulated crash overtakes a read that the reader has started
/// Then the reader keeps running and reports one canceled read
/// And every state that it recorded holds only the durable record
/// Cases: SIM-MODEL, ORACLE-8
#[test]
fn a_reader_outlives_a_read_that_a_crash_cancels() {
    let _watchdog = watchdog("reader across a crash".into());
    let sim = Sim::new(921);
    let ledger = Arc::new(Mutex::new(Ledger {
        seed: 921,
        mode: Crash::PowerLoss,
        options: options(),
        requests: vec![],
        reads: vec![],
    }));
    let log = Arc::new(open(&sim, &options()));
    Caller::start(&log, &ledger, literal(0, 1, 3, 0), true).finish();
    let bytes = super::support::raw::bytes(&sim);
    let length = u32::from_le_bytes(bytes[64..68].try_into().unwrap()) as usize + 8;
    let reader = Reader::start(&log, &ledger);
    let read = sim.hold(Call::new(SEG, Purpose::Lookup, Op::Read(64, length)));
    read.wait();
    sim.crash(Crash::PowerLoss, 921);
    read.release(None);
    assert_eq!(reader.finish(), 1);
    let ledger = ledger.lock().unwrap();
    assert!(!ledger.reads.is_empty());
    for read in &ledger.reads {
        assert_eq!(read.records.len(), 1);
        assert_eq!((read.before, read.after), (1, 1));
    }
}

/// Given the reduced pressure trace from seed 1000 with a small group cap
/// When recovery finishes and the oracle checks durable-offset non-reuse
/// Then its fresh verification submit fits the saved cap without changing options
/// Cases: ORACLE-10, ADM-SIZE
#[test]
fn recovery_verification_respects_the_saved_small_group_cap() {
    let _watchdog = watchdog("reduced pressure seed 1000".into());
    replay(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/evidence/m1/seeded/pressure-1000"),
        false,
    );
}

/// Given a recorded TooLarge refusal for a small, structurally valid request
/// When the oracle validates it under the saved default limits
/// Then it rejects the false permanent refusal independently of the engine
/// Cases: ADM-SIZE, SIM-MODEL
#[test]
fn oracle_rejects_a_false_permanent_size_refusal() {
    let _watchdog = watchdog("false size refusal".into());
    let checked = std::panic::catch_unwind(|| oracle::answers(&[false_refusal()], &[], &options()));
    assert!(
        checked.is_err(),
        "oracle accepted an eligible request's TooLarge refusal"
    );
}

/// Given one atomic four-record group and a reader bracket spanning its publication
/// When a read returns half the group without a max limit, or a head from its middle
/// Then the oracle rejects the snapshot because no committed group boundary permits it
/// Cases: ORACLE-8, READ-BOUNDED
#[test]
fn oracle_rejects_partial_group_reader_states() {
    let _watchdog = watchdog("partial group reader states".into());
    let records: Vec<_> = literal(8, 4, 3, 0)
        .iter()
        .enumerate()
        .map(|(i, r)| r.stored(i as u64, i as u64))
        .collect();
    for (count, head) in [(2, None), (4, Some((1, 1)))] {
        let read = Observation {
            before: 0,
            after: 4,
            from: 0,
            max: 100,
            records: records[..count]
                .iter()
                .map(|r| Record {
                    offset: r.offset,
                    revision: r.revision,
                    stream: r.stream.clone(),
                    key: r.key.clone(),
                    payload: r.payload.clone(),
                })
                .collect(),
            heads: vec![(b"s8".to_vec(), head)],
        };
        let checked =
            std::panic::catch_unwind(|| oracle::observations(&[], &[read], &records, &[0, 4]));
        assert!(
            checked.is_err(),
            "oracle accepted an impossible reader snapshot"
        );
    }
}
