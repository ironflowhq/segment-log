use segment_log_core::*;
use std::{
    sync::{
        Barrier,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const STAGES: usize = 4;
const GROUP: usize = 32;
const PAYLOAD: usize = 256;

struct History {
    keyed: bool,
}
impl History {
    fn record(&self, i: usize) -> NewRecord {
        NewRecord {
            stream: b"stream".to_vec(),
            expected: if i == 0 {
                ExpectedRevision::Empty
            } else {
                ExpectedRevision::At(i as u64 - 1)
            },
            key: self.keyed.then(|| (i as u64).to_le_bytes().to_vec()),
            payload: vec![(i % 251) as u8; PAYLOAD],
        }
    }
    fn submit(&self, from: usize, count: usize) -> Submit {
        Submit {
            records: (from..from + count).map(|i| self.record(i)).collect(),
        }
    }
    fn check(&self, records: &[Record], from: usize, count: usize) -> bool {
        records.len() == count
            && records.iter().enumerate().all(|(j, r)| {
                let i = from + j;
                r.offset == i as u64
                    && r.revision == i as u64
                    && r.stream == b"stream"
                    && r.key.as_deref()
                        == self
                            .keyed
                            .then_some((i as u64).to_le_bytes())
                            .as_ref()
                            .map(|k| k.as_slice())
                    && r.payload.len() == PAYLOAD
                    && r.payload.iter().all(|&b| b == (i % 251) as u8)
            })
    }
}

// Sixteen subdivisions per power of two, fixed memory regardless of operation count.
#[derive(Clone)]
struct Distribution {
    bins: [u64; 1024],
    count: u64,
    sum: u128,
    max: u64,
}
impl Default for Distribution {
    fn default() -> Self {
        Self {
            bins: [0; 1024],
            count: 0,
            sum: 0,
            max: 0,
        }
    }
}
impl Distribution {
    fn bucket(ns: u64) -> usize {
        let power = 63 - ns.max(1).leading_zeros() as usize;
        let base = 1u64 << power;
        let fraction = (((ns.saturating_sub(base)) as u128 * 16) / base as u128) as usize;
        power * 16 + fraction
    }
    fn bounds(i: usize) -> (u128, u128) {
        let base = 1u128 << (i / 16);
        let low = if i == 0 {
            0
        } else {
            base + (base * (i % 16) as u128).div_ceil(16)
        };
        let high = base + (base * (i % 16 + 1) as u128).div_ceil(16) - 1;
        (low, high)
    }
    fn add(&mut self, ns: u64) {
        self.bins[Self::bucket(ns)] += 1;
        self.count += 1;
        self.sum += ns as u128;
        self.max = self.max.max(ns);
    }
    fn print(&self, stage: usize, kind: &str) {
        println!(
            "distribution,{stage},{kind},{},{},{}",
            self.count, self.sum, self.max
        );
        for (i, count) in self.bins.iter().enumerate().filter(|(_, n)| **n != 0) {
            let (low, high) = Self::bounds(i);
            println!("bin,{stage},{kind},{low},{high},{count}");
        }
    }
}
#[derive(Default, Clone)]
struct Reads {
    start: Distribution,
    end: Distribution,
    prefix: Distribution,
    calls: usize,
    errors: usize,
    mismatches: usize,
    records: usize,
    max_gap_ns: u128,
}

fn append(
    log: &SegmentLog,
    h: &History,
    from: usize,
    count: usize,
    timing: bool,
    dist: &mut Distribution,
) -> bool {
    let submit = h.submit(from, count);
    let start = timing.then(Instant::now);
    let result = match log.append(submit) {
        Ok(p) => p.wait().map_err(|e| format!("commit_error,{e:?}")),
        Err(e) => Err(format!("append_refusal,{e:?}")),
    };
    if let Some(start) = start {
        dist.add(start.elapsed().as_nanos() as u64);
    }
    match result {
        Ok(c)
            if !c.deduplicated
                && c.records.len() == count
                && c.records.iter().enumerate().all(|(i, p)| {
                    p.offset == (from + i) as u64 && p.revision == (from + i) as u64
                }) =>
        {
            true
        }
        Ok(c) => {
            println!("failure,commit_mismatch,{c:?}");
            false
        }
        Err(e) => {
            println!("failure,{e}");
            false
        }
    }
}

fn read(
    log: &SegmentLog,
    h: &History,
    from: usize,
    count: usize,
    timing: bool,
    dist: &mut Distribution,
    stats: &mut (usize, usize, usize, usize),
) -> bool {
    let started = timing.then(Instant::now);
    let result = log.read(from as u64, count);
    if let Some(started) = started {
        dist.add(started.elapsed().as_nanos() as u64);
    }
    stats.0 += 1;
    match result {
        Ok(records) => {
            stats.3 += records.len();
            if h.check(&records, from, count) {
                true
            } else {
                stats.2 += 1;
                eprintln!(
                    "read mismatch from={from} expected={count} actual={}",
                    records.len()
                );
                false
            }
        }
        Err(e) => {
            stats.1 += 1;
            eprintln!("read error from={from}: {e:?}");
            false
        }
    }
}

fn main() {
    let a: Vec<_> = std::env::args().collect();
    if a.get(1).map(String::as_str) == Some("calibrate") {
        let h = History { keyed: true };
        let started = Instant::now();
        for i in 0..32768 / GROUP {
            std::hint::black_box(h.submit(i * GROUP, GROUP));
        }
        println!(
            "calibration,build_32768_records_ns,{}",
            started.elapsed().as_nanos()
        );
        let mut dist = Distribution::default();
        let started = Instant::now();
        for _ in 0..100_000 {
            let t = Instant::now();
            dist.add(t.elapsed().as_nanos() as u64);
        }
        std::hint::black_box(dist);
        println!(
            "calibration,clock_and_histogram_100000_ns,{}",
            started.elapsed().as_nanos()
        );
        return;
    }
    assert_eq!(
        a.len(),
        8,
        "usage: concurrent_probe NEW_DIR append|concurrent keyed|unkeyed default|zero SEED_RECORDS ADDED_RECORDS timing|untimed"
    );
    let concurrent = match a[2].as_str() {
        "append" => false,
        "concurrent" => true,
        _ => panic!("mode"),
    };
    let h = History {
        keyed: match a[3].as_str() {
            "keyed" => true,
            "unkeyed" => false,
            _ => panic!("keys"),
        },
    };
    let options = || match a[4].as_str() {
        "default" => Options::default(),
        "zero" => Options {
            linger: Duration::ZERO,
            ..Options::default()
        },
        _ => panic!("options"),
    };
    let seed: usize = a[5].parse().unwrap();
    let added: usize = a[6].parse().unwrap();
    let timing = match a[7].as_str() {
        "timing" => true,
        "untimed" => false,
        _ => panic!("timing"),
    };
    assert!(
        seed > 0 && seed.is_multiple_of(GROUP) && added > 0 && added.is_multiple_of(GROUP * STAGES)
    );
    let dir = std::path::Path::new(&a[1]);
    assert!(!dir.exists(), "requires new data directory");
    println!(
        "config,seed={seed},added={added},group={GROUP},payload={PAYLOAD},mode={},keys={},options={},timing={timing}",
        a[2], a[3], a[4]
    );
    let log = SegmentLog::open(dir, options()).unwrap();
    let mut failed = false;
    for from in (0..seed).step_by(GROUP) {
        if !append(&log, &h, from, GROUP, false, &mut Distribution::default()) {
            failed = true;
            break;
        }
    }
    if failed {
        std::process::exit(1);
    }
    assert_eq!(log.watermark(), seed as u64);
    let done = AtomicBool::new(false);
    let barrier = Barrier::new(if concurrent { 2 } else { 1 });
    let per_stage = added / STAGES;
    let mut append_distributions = vec![Distribution::default(); STAGES];
    println!("phase,measurement_start");
    let (reader, cursor, elapsed, committed) = std::thread::scope(|scope| {
        let handle = concurrent.then(|| {
            scope.spawn(|| {
                let mut reads = vec![Reads::default(); STAGES];
                let mut cursor = 0;
                barrier.wait();
                let mut last = Instant::now();
                while !done.load(Ordering::Acquire) {
                    let w = log.watermark() as usize;
                    let stage = ((w.saturating_sub(seed)) / per_stage).min(STAGES - 1);
                    let r = &mut reads[stage];
                    let mut counts = (0, 0, 0, 0);
                    let mut ok = read(&log, &h, 0, 1, timing, &mut r.start, &mut counts);
                    ok &= read(&log, &h, w - 1, 1, timing, &mut r.end, &mut counts);
                    if cursor < w {
                        let count = GROUP.min(w - cursor);
                        if read(&log, &h, cursor, count, timing, &mut r.prefix, &mut counts) {
                            cursor += count;
                        } else {
                            ok = false;
                        }
                    }
                    r.calls += counts.0;
                    r.errors += counts.1;
                    r.mismatches += counts.2;
                    r.records += counts.3;
                    if timing {
                        r.max_gap_ns = r.max_gap_ns.max(last.elapsed().as_nanos());
                        last = Instant::now();
                    }
                    if !ok {
                        break;
                    }
                }
                (reads, cursor)
            })
        });
        barrier.wait();
        let started = Instant::now();
        let mut committed = 0;
        for (stage, dist) in append_distributions.iter_mut().enumerate() {
            let stage_start = Instant::now();
            for from in (seed + stage * per_stage..seed + (stage + 1) * per_stage).step_by(GROUP) {
                if !append(&log, &h, from, GROUP, timing, dist) {
                    failed = true;
                    break;
                }
                committed += GROUP;
            }
            println!(
                "stage,{stage},{},{},{}",
                stage_start.elapsed().as_nanos(),
                committed,
                log.watermark()
            );
            if failed {
                break;
            }
        }
        let elapsed = started.elapsed().as_nanos();
        done.store(true, Ordering::Release);
        let (reads, cursor) = handle.map(|j| j.join().unwrap()).unwrap_or_default();
        (reads, cursor, elapsed, committed)
    });
    println!("phase,measurement_end");
    for (stage, dist) in append_distributions.iter().enumerate() {
        dist.print(stage, "append");
    }
    for (stage, r) in reader.iter().enumerate() {
        r.start.print(stage, "start");
        r.end.print(stage, "end");
        r.prefix.print(stage, "prefix");
        println!(
            "reads,{stage},{},{},{},{},{}",
            r.calls, r.errors, r.mismatches, r.records, r.max_gap_ns
        );
        failed |= r.errors != 0 || r.mismatches != 0;
    }
    println!(
        "measurement,{elapsed},{committed},{cursor},{}",
        seed + committed
    );
    // Drain outside the measured interval and verify every offset exactly once after reopen.
    let mut drained = cursor;
    while concurrent && drained < seed + committed {
        let count = GROUP.min(seed + committed - drained);
        let records = log.read(drained as u64, count).unwrap();
        if !h.check(&records, drained, count) {
            failed = true;
            break;
        }
        drained += count;
    }
    drop(log);
    let log = SegmentLog::open(dir, options()).unwrap();
    assert_eq!(log.watermark(), (seed + committed) as u64);
    let mut validated = 0;
    while validated < seed + committed {
        let count = GROUP.min(seed + committed - validated);
        let records = log.read(validated as u64, count).unwrap();
        if !h.check(&records, validated, count) {
            failed = true;
            break;
        }
        validated += count;
    }
    assert!(log.read((seed + committed) as u64, 1).unwrap().is_empty());
    println!("validation,{validated},{drained},{}", usize::from(failed));
    if failed {
        std::process::exit(1);
    }
    println!("success_counts,append_refusals=0,commit_errors=0,read_errors=0,validation_errors=0");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_contains_samples_and_conserves_counts() {
        let mut dist = Distribution::default();
        let values = (0..65536)
            .chain((0..64).flat_map(|p| {
                let n = 1u64 << p;
                [n - 1, n, n.saturating_add(1)]
            }))
            .chain([u64::MAX]);
        let mut count = 0;
        let mut sum = 0;
        for ns in values {
            let (low, high) = Distribution::bounds(Distribution::bucket(ns));
            assert!(
                low <= ns as u128 && ns as u128 <= high,
                "{ns} outside {low}..={high}"
            );
            dist.add(ns);
            count += 1;
            sum += ns as u128;
        }
        assert_eq!(dist.bins.iter().sum::<u64>(), count);
        assert_eq!(dist.count, count);
        assert_eq!(dist.sum, sum);
        assert_eq!(dist.max, u64::MAX);
    }

    #[test]
    fn oracle_rejects_empty_truncated_wrong_offset_and_payload() {
        for keyed in [false, true] {
            let h = History { keyed };
            let mut records: Vec<_> = h
                .submit(32, GROUP)
                .records
                .into_iter()
                .enumerate()
                .map(|(j, r)| Record {
                    offset: (32 + j) as u64,
                    revision: (32 + j) as u64,
                    stream: r.stream,
                    key: r.key,
                    payload: r.payload,
                })
                .collect();
            assert!(h.check(&records, 32, GROUP));
            assert!(!h.check(&[], 32, GROUP));
            assert!(!h.check(&records[..GROUP - 1], 32, GROUP));
            records[0].offset += 1;
            assert!(!h.check(&records, 32, GROUP));
            records[0].offset -= 1;
            records[0].payload[0] ^= 1;
            assert!(!h.check(&records, 32, GROUP));
        }
    }
}
