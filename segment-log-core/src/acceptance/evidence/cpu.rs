use std::ops::{AddAssign, Sub};

#[derive(Default)]
pub(super) struct Times {
    running_ns: u64,
    runnable_ns: u64,
}
impl AddAssign for Times {
    fn add_assign(&mut self, other: Self) {
        self.running_ns += other.running_ns;
        self.runnable_ns += other.runnable_ns;
    }
}
impl Sub for Times {
    type Output = Self;
    fn sub(self, earlier: Self) -> Self {
        Self {
            running_ns: self.running_ns.checked_sub(earlier.running_ns).unwrap(),
            runnable_ns: self.runnable_ns.checked_sub(earlier.runnable_ns).unwrap(),
        }
    }
}
impl Times {
    pub(super) fn report(&self, role: &str) {
        println!(
            "PHASE_CPU role={role} running_ns={} runnable_ns={}",
            self.running_ns, self.runnable_ns
        );
    }
}
fn read(task: impl AsRef<std::path::Path>) -> Times {
    let stat = std::fs::read_to_string(task.as_ref().join("schedstat"))
        .expect("SEGMENT_LOG_BENCH_CPU requires Linux procfs schedstat");
    let mut fields = stat.split_whitespace().map(|s| s.parse::<u64>().unwrap());
    Times {
        running_ns: fields.next().unwrap(),
        runnable_ns: fields.next().unwrap(),
    }
}
pub(super) fn current() -> Times {
    read("/proc/thread-self")
}
pub(super) fn writer() -> Times {
    let tasks = std::fs::read_dir("/proc/self/task").expect("CPU diagnostic requires Linux");
    let writers: Vec<_> = tasks
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            std::fs::read_to_string(path.join("comm"))
                .is_ok_and(|name| name.trim() == "segment-log-wri")
        })
        .collect();
    assert_eq!(writers.len(), 1, "run only the benchmark test");
    read(&writers[0])
}
