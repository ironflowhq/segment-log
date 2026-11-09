use crate::Submit;
use std::{
    fs::File,
    io::{BufWriter, Write},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

pub(crate) const START: usize = 0;
pub(crate) const ADMISSION: usize = 1;
pub(crate) const ENQUEUE: usize = 2;
pub(crate) const DEQUEUE: usize = 3;
pub(crate) const CLASSIFY: usize = 4;
pub(crate) const READY: usize = 5;
pub(crate) const SEND: usize = 6;
pub(crate) const WAKE: usize = 7;
pub(crate) const ADMITTED: usize = 8;
const GROUP: usize = 9;
const OFFSET: usize = 10;

struct Trace {
    origin: Instant,
    rounds: usize,
    slots: Vec<[AtomicU64; 11]>,
    groups: Mutex<Vec<Group>>,
    path: std::path::PathBuf,
}
static TRACE: OnceLock<Trace> = OnceLock::new();

pub(crate) fn init(callers: usize, rounds: usize) {
    let Some(path) = std::env::var_os("SEGMENT_LOG_PIPELINE_OUT") else {
        return;
    };
    assert!(
        TRACE
            .set(Trace {
                origin: Instant::now(),
                rounds,
                slots: (0..callers.checked_mul(rounds).unwrap())
                    .map(|_| std::array::from_fn(|_| AtomicU64::new(0)))
                    .collect(),
                groups: Mutex::new(Vec::new()),
                path: path.into(),
            })
            .is_ok(),
        "run only measured_appends_survive_reopen in this process"
    );
    let mut samples = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let start = Instant::now();
        std::hint::black_box(Instant::now());
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    println!(
        "CLOCK samples=10000 min_ns={} p50_ns={} p99_ns={} max_ns={}",
        samples[0], samples[4999], samples[9899], samples[9999]
    );
}
fn now(trace: &Trace) -> u64 {
    trace.origin.elapsed().as_nanos().try_into().unwrap()
}
fn id(trace: &Trace, submit: &Submit) -> usize {
    assert_eq!(
        submit.records.len(),
        1,
        "pipeline supports the evidence workload only"
    );
    let key = submit.records[0].key.as_ref().unwrap();
    assert_eq!(key.len(), 16);
    let caller = u64::from_le_bytes(key[..8].try_into().unwrap()) as usize;
    let revision = u64::from_le_bytes(key[8..].try_into().unwrap()) as usize;
    assert!(revision < trace.rounds);
    caller * trace.rounds + revision
}
pub(crate) fn caller(caller: usize, revision: usize, stage: usize) {
    if let Some(trace) = TRACE.get() {
        trace.slots[caller * trace.rounds + revision][stage].store(now(trace), Ordering::Relaxed);
    }
}
pub(crate) fn committed(caller: usize, revision: usize, offset: u64) {
    if let Some(trace) = TRACE.get() {
        trace.slots[caller * trace.rounds + revision][OFFSET].store(offset + 1, Ordering::Relaxed);
    }
}
pub(crate) fn request(submit: &Submit, stage: usize) {
    if let Some(trace) = TRACE.get() {
        let slot = &trace.slots[id(trace, submit)][stage];
        let ns = now(trace);
        if stage == DEQUEUE {
            let _ = slot.compare_exchange(0, ns, Ordering::Relaxed, Ordering::Relaxed);
        } else {
            slot.store(ns, Ordering::Relaxed);
        }
    }
}
pub(crate) fn group_id(submit: &Submit, base: u64) {
    if let Some(trace) = TRACE.get() {
        trace.slots[id(trace, submit)][GROUP].store(base + 1, Ordering::Relaxed);
    }
}
pub(crate) struct Group {
    base: u64,
    count: usize,
    times: [u64; 6],
}
impl Group {
    pub(crate) fn new(base: u64, count: usize) -> Self {
        let mut group = Self {
            base,
            count,
            times: [0; 6],
        };
        group.mark(0);
        group
    }
    pub(crate) fn mark(&mut self, stage: usize) {
        if let Some(trace) = TRACE.get() {
            self.times[stage] = now(trace);
        }
    }
    pub(crate) fn finish(mut self) {
        if let Some(trace) = TRACE.get() {
            self.mark(5);
            trace.groups.lock().unwrap().push(self);
        }
    }
}
pub(crate) fn report() {
    let Some(trace) = TRACE.get() else { return };
    let mut out = BufWriter::new(File::create(&trace.path).unwrap());
    writeln!(out, "caller\trevision\tstart\tadmission\tenqueue\tdequeue\tclassify\tready\tsend\twake\tadmitted\tgroup\toffset_plus_one").unwrap();
    for (i, slot) in trace.slots.iter().enumerate() {
        write!(out, "{}\t{}", i / trace.rounds, i % trace.rounds).unwrap();
        for value in slot {
            write!(out, "\t{}", value.load(Ordering::Relaxed)).unwrap();
        }
        writeln!(out).unwrap();
    }
    out.flush().unwrap();
    let mut out = BufWriter::new(File::create(trace.path.with_extension("groups.tsv")).unwrap());
    writeln!(
        out,
        "group\tcount\tformed\tencoded\twritten\tflushed\tpublished\tdelivered"
    )
    .unwrap();
    for group in trace.groups.lock().unwrap().iter() {
        write!(out, "{}\t{}", group.base + 1, group.count).unwrap();
        for value in group.times {
            write!(out, "\t{value}").unwrap();
        }
        writeln!(out).unwrap();
    }
    out.flush().unwrap();
    println!(
        "PIPELINE requests={} raw={}",
        trace.slots.len(),
        trace.path.display()
    );
}
