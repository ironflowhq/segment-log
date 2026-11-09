use segment_log_core::*;
use std::{
    hint::black_box,
    io::{self, Write},
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

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

    fn submits(&self) -> Vec<Submit> {
        (0..self.records)
            .step_by(self.group)
            .map(|start| Submit {
                records: (start..(start + self.group).min(self.records))
                    .map(|i| self.record(i))
                    .collect(),
            })
            .collect()
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

#[derive(Default)]
struct Counts {
    accepted: usize,
    committed: usize,
    admission_errors: usize,
    commit_errors: usize,
    validated: usize,
}

fn barrier(phase: &str) -> Result<(), String> {
    println!("barrier\t{phase}");
    io::stdout().flush().map_err(|e| e.to_string())?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if line.trim() != "go" {
        return Err("expected go at phase barrier".into());
    }
    Ok(())
}

fn work<T>(phase: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    println!(
        "begin\t{phase}\t{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    io::stdout().flush().map_err(|e| e.to_string())?;
    let start = Instant::now();
    let value = f();
    println!(
        "end\t{phase}\t{}\t{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        start.elapsed().as_nanos()
    );
    value
}

fn run(c: &mut Counts) -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 8 {
        return Err("usage: resident_probe MODE DIR RECORDS KEYS STREAMS GROUP PAYLOAD; MODE = empty|caller|append|recover|touch".into());
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
    let dir = Path::new(&args[2]);
    let options = || Options {
        linger: Duration::ZERO,
        ..Options::default()
    };
    barrier("baseline")?;
    match args[1].as_str() {
        "touch" => {
            let bytes = h
                .records
                .checked_mul(h.payload)
                .ok_or("touch size overflow")?;
            let buffer = work("touch", || Ok(vec![0xa5u8; bytes]))?;
            black_box(&buffer);
            barrier("touched_held")?;
            if !buffer.iter().all(|&b| b == 0xa5) {
                return Err("touch mismatch".into());
            }
            drop(buffer);
            barrier("touched_dropped")?;
        }
        "caller" => {
            let submits = work("caller_build", || Ok(h.submits()))?;
            black_box(&submits);
            barrier("caller_held")?;
            if submits.iter().map(|s| s.records.len()).sum::<usize>() != h.records {
                return Err("caller count mismatch".into());
            }
            drop(submits);
            barrier("caller_dropped")?;
        }
        mode @ ("empty" | "append" | "recover") => {
            if (mode == "recover") != dir.exists() {
                return Err(
                    "recover needs an existing directory; empty/append need a new one".into(),
                );
            }
            let log = work("open", || {
                SegmentLog::open(dir, options()).map_err(|e| format!("open: {e:?}"))
            })?;
            barrier(if mode == "recover" {
                "recovered_held"
            } else {
                "empty_held"
            })?;
            if mode == "append" {
                work("append", || {
                    for start in (0..h.records).step_by(h.group) {
                        let count = h.group.min(h.records - start);
                        let submit = Submit {
                            records: (start..start + count).map(|i| h.record(i)).collect(),
                        };
                        let pending = log.append(submit).map_err(|e| {
                            c.admission_errors += 1;
                            format!("append: {e:?}")
                        })?;
                        c.accepted += count;
                        let committed = pending.wait().map_err(|e| {
                            c.commit_errors += 1;
                            format!("commit: {e:?}")
                        })?;
                        if committed.deduplicated || committed.records.len() != count {
                            return Err("commit count/dedup mismatch".into());
                        }
                        for (i, p) in (start..).zip(&committed.records) {
                            if p.offset != i as u64 || p.revision != (i / h.streams) as u64 {
                                return Err("commit position mismatch".into());
                            }
                        }
                        c.committed += count;
                    }
                    Ok(())
                })?;
                barrier("appended_held")?;
            }
            let expected = if mode == "empty" { 0 } else { h.records };
            if log.watermark() != expected as u64 {
                return Err("watermark mismatch".into());
            }
            if expected > 0 {
                let records = work("read_one", || {
                    log.read(0, 1).map_err(|e| format!("read: {e:?}"))
                })?;
                barrier("read_one_held")?;
                h.check(&records, 0, 1)?;
                drop(records);
                barrier("read_one_dropped")?;
            }
            let records = work("validation_read", || {
                log.read(0, usize::MAX)
                    .map_err(|e| format!("validation read: {e:?}"))
            })?;
            barrier("validation_held")?;
            h.check(&records, 0, expected)?;
            c.validated = records.len();
            drop(records);
            barrier("validation_dropped")?;
            work("drop_engine", || {
                drop(log);
                Ok(())
            })?;
            barrier("engine_dropped")?;
        }
        _ => return Err("unknown mode".into()),
    }
    Ok(())
}

fn main() {
    let mut c = Counts::default();
    let result = run(&mut c);
    println!(
        "result\taccepted={}\tcommitted={}\tadmission_errors={}\tcommit_errors={}\tvalidated={}\tfailures={}",
        c.accepted,
        c.committed,
        c.admission_errors,
        c.commit_errors,
        c.validated,
        usize::from(result.is_err())
    );
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
