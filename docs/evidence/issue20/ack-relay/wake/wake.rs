// Microbenchmark: how long does it take to wake N parked threads?
// Strategies: serial unpark by one thread, relay tree with fan-out k, striped Condvar broadcast.
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::*};
use std::sync::{Arc, Barrier, Condvar, Mutex};
use std::thread::{self, Thread};
use std::time::Instant;

struct Shared {
    origin: Instant,
    round: AtomicU64,     // generation the waiters wait for
    woke: Vec<AtomicU64>, // ns at which waiter i observed the round
    done: AtomicUsize,
    threads: Mutex<Vec<Option<Thread>>>,
    fanout: AtomicUsize, // 0 = no relay
    stripes: Vec<(Mutex<u64>, Condvar)>,
}
fn now(s: &Shared) -> u64 {
    s.origin.elapsed().as_nanos() as u64
}
fn median(v: &mut Vec<u64>) -> u64 {
    v.sort_unstable();
    v[v.len() / 2]
}

fn main() {
    let n: usize = std::env::args().nth(1).map_or(1000, |s| s.parse().unwrap());
    let reps: u64 = std::env::args().nth(2).map_or(200, |s| s.parse().unwrap());
    let s = Arc::new(Shared {
        origin: Instant::now(),
        round: AtomicU64::new(0),
        woke: (0..n).map(|_| AtomicU64::new(0)).collect(),
        done: AtomicUsize::new(0),
        threads: Mutex::new(vec![None; n]),
        fanout: AtomicUsize::new(0),
        stripes: (0..256).map(|_| (Mutex::new(0), Condvar::new())).collect(),
    });
    let mode = Arc::new(AtomicUsize::new(0)); // 0 = park, s>0 = condvar with s stripes
    let ready = Arc::new(Barrier::new(n + 1));
    let handles: Vec<Thread> = (0..n)
        .map(|i| {
            let (s, mode, ready) = (s.clone(), mode.clone(), ready.clone());
            thread::Builder::new()
                .stack_size(64 * 1024)
                .spawn(move || {
                    ready.wait();
                    let mut seen = 0;
                    loop {
                        let stripes = mode.load(Acquire);
                        if stripes == 0 {
                            while s.round.load(Acquire) == seen {
                                thread::park();
                            }
                        } else {
                            let (lock, cv) = &s.stripes[i % stripes];
                            let mut guard = lock.lock().unwrap();
                            while s.round.load(Acquire) == seen {
                                guard = cv.wait(guard).unwrap();
                            }
                        }
                        seen = s.round.load(Acquire);
                        if seen == u64::MAX {
                            return;
                        }
                        let k = s.fanout.load(Acquire);
                        if k > 0 {
                            let first = (i + 1) * k;
                            let threads = s.threads.lock().unwrap();
                            let kids: Vec<Thread> = (first..(first + k).min(n))
                                .map(|c| threads[c].clone().unwrap())
                                .collect();
                            drop(threads);
                            for kid in kids {
                                kid.unpark();
                            }
                        }
                        s.woke[i].store(now(&s), Release);
                        s.done.fetch_add(1, AcqRel);
                    }
                })
                .unwrap()
                .thread()
                .clone()
        })
        .collect();
    *s.threads.lock().unwrap() = handles.iter().cloned().map(Some).collect();
    ready.wait();
    let settle = || {
        thread::sleep(std::time::Duration::from_millis(3));
    };
    let mut run = |label: &str, stripes: usize, fanout: usize| {
        mode.store(stripes, Release);
        // switch waiters to the new mode with one throwaway round
        s.fanout.store(0, Release);
        s.done.store(0, Release);
        s.round.fetch_add(1, AcqRel);
        for h in &handles {
            h.unpark();
        }
        for (l, c) in &s.stripes {
            let _g = l.lock().unwrap();
            c.notify_all();
        }
        while s.done.load(Acquire) < n {
            thread::yield_now();
        }
        s.fanout.store(fanout, Release);
        let (mut waker, mut last, mut mean) = (vec![], vec![], vec![]);
        for _ in 0..reps {
            settle();
            s.done.store(0, Release);
            let t0 = now(&s);
            s.round.fetch_add(1, AcqRel);
            if stripes > 0 {
                for (l, c) in &s.stripes[..stripes] {
                    drop(l.lock().unwrap());
                    c.notify_all();
                }
            } else if fanout > 0 {
                for h in &handles[..fanout.min(n)] {
                    h.unpark();
                }
            } else {
                for h in &handles {
                    h.unpark();
                }
            }
            let t1 = now(&s);
            while s.done.load(Acquire) < n {
                std::hint::spin_loop();
            }
            let times: Vec<u64> = s.woke.iter().map(|w| w.load(Acquire) - t0).collect();
            waker.push(t1 - t0);
            last.push(*times.iter().max().unwrap());
            mean.push(times.iter().sum::<u64>() / n as u64);
        }
        println!(
            "{label:28} waker_us={:8.1} mean_wake_us={:8.1} last_wake_us={:8.1}",
            median(&mut waker) as f64 / 1e3,
            median(&mut mean) as f64 / 1e3,
            median(&mut last) as f64 / 1e3
        );
    };
    println!("threads={n} reps={reps} (medians)");
    run("serial unpark", 0, 0);
    for k in [2, 4, 8, 16, 32] {
        run(&format!("relay tree fanout={k}"), 0, k);
    }
    for st in [1, 8, 32, 128] {
        run(&format!("condvar notify_all x{st}"), st, 0);
    }
    run("serial unpark (again)", 0, 0);
    mode.store(0, Release);
    s.round.store(u64::MAX, Release);
    for h in &handles {
        h.unpark();
    }
}
