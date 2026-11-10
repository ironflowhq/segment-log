use super::*;

#[derive(Default)]
struct Metrics {
    groups: Vec<(u64, u64)>,
    flushes: usize,
    flush_ns: Vec<u64>,
    directories: Vec<PathBuf>,
    timings: std::collections::BTreeMap<String, (u64, u64)>,
}
pub(super) struct Measured {
    real: Real,
    verbose: bool,
    metrics: Mutex<Metrics>,
}
impl Measured {
    pub(super) fn new(verbose: bool) -> Self {
        Self {
            real: Real::default(),
            verbose,
            metrics: Mutex::new(Metrics::default()),
        }
    }
    pub(super) fn groups(&self) -> Vec<(u64, u64)> {
        self.metrics.lock().unwrap().groups.clone()
    }
    pub(super) fn group_flushes(&self) -> usize {
        self.metrics.lock().unwrap().flushes
    }
    /// How long each successful group flush took, in the order of the groups.
    pub(super) fn group_flush_ns(&self) -> Vec<u64> {
        self.metrics.lock().unwrap().flush_ns.clone()
    }
    pub(super) fn assert_directories(&self, path: &Path, log: usize, parent: usize) {
        let metrics = self.metrics.lock().unwrap();
        assert_eq!(
            metrics.directories.iter().filter(|p| *p == path).count(),
            log
        );
        assert_eq!(
            metrics
                .directories
                .iter()
                .filter(|p| *p == path.parent().unwrap())
                .count(),
            parent
        );
    }
    pub(super) fn report(&self) {
        for (kind, (count, ns)) in &self.metrics.lock().unwrap().timings {
            println!("STORAGE kind={kind} count={count} sum_ns={ns}");
        }
    }
}
impl Storage for Measured {
    fn call(&self, call: Call) -> io::Result<Value> {
        let name = match &call.op {
            Op::Append(_) => "Append",
            Op::SyncData => "SyncData",
            Op::SyncAll => "SyncAll",
            Op::SyncDir => "SyncDir",
            _ => "Other",
        };
        let kind = format!("{:?}.{name}", call.purpose);
        let path = call.path.clone();
        let group = match &call.op {
            Op::Append(bytes) if call.purpose == Purpose::Group => Some((
                u32::from_le_bytes(bytes[28..32].try_into().unwrap()) as u64,
                bytes.len() as u64,
            )),
            _ => None,
        };
        let now = Instant::now();
        let result = self.real.call(call);
        let ns = now.elapsed().as_nanos() as u64;
        if self.verbose && matches!(name, "SyncData" | "SyncAll" | "SyncDir") {
            eprintln!(
                "STORAGE {kind} path={} result={:?}",
                path.display(),
                result.as_ref().map(|_| ())
            );
        }
        let mut metrics = self.metrics.lock().unwrap();
        let timing = metrics.timings.entry(kind.clone()).or_default();
        timing.0 += 1;
        timing.1 += ns;
        if result.is_ok() {
            if let Some(group) = group {
                metrics.groups.push(group);
            }
            if kind == "Group.SyncData" {
                metrics.flushes += 1;
                metrics.flush_ns.push(ns);
            }
            if name == "SyncDir" {
                metrics.directories.push(path);
            }
        }
        result
    }
    fn lock(&self, path: &Path) -> io::Result<Box<dyn Lock>> {
        self.real.lock(path)
    }
}
