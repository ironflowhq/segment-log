use super::*;

use super::super::support::deadline::Deadline;

pub(super) fn watchdog(label: String) -> Deadline {
    watchdog_with_failure(label, || {})
}
pub(super) fn watchdog_with_failure(
    label: String,
    failure: impl FnOnce() + Send + 'static,
) -> Deadline {
    Deadline::new(Duration::from_secs(30), move || {
        eprintln!("seeded scenario watchdog {label}");
        failure();
    })
}

pub(super) enum Command {
    Probe,
    Finish,
    Abandon,
}
pub(super) struct Caller {
    gate: Option<mpsc::Sender<()>>,
    pub(super) id: usize,
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<bool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Caller {
    pub(super) fn prepare(
        log: &Arc<SegmentLog>,
        ledger: &Arc<Mutex<Ledger>>,
        inputs: Vec<Input>,
        original: bool,
    ) -> Self {
        let id = {
            let mut l = ledger.lock().unwrap();
            let id = l.requests.len();
            l.requests.push(Request {
                inputs: inputs.clone(),
                original,
                answer: Answer::Unknown,
                watermark: 0,
            });
            id
        };
        let log = log.clone();
        let ledger = ledger.clone();
        let (tx, commands) = mpsc::channel();
        let (ready, rx) = mpsc::channel();
        let (gate, start) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = start.recv();
            let mut pending = match log.append(oracle::submit(&inputs)) {
                Ok(p) => Some(p),
                Err(e) => {
                    ledger.lock().unwrap().requests[id].answer = Answer::admission(e);
                    None
                }
            };
            ready.send(pending.is_some()).unwrap();
            while let Ok(command) = commands.recv() {
                let result = match command {
                    Command::Probe => pending.as_ref().and_then(Pending::probe),
                    Command::Finish => pending.take().map(Pending::wait),
                    Command::Abandon => {
                        drop(pending.take());
                        None
                    }
                };
                if let Some(result) = result {
                    pending = None;
                    ledger.lock().unwrap().requests[id].answer = Answer::commit(result);
                }
                let watermark = log.watermark();
                let mut ledger = ledger.lock().unwrap();
                let previous = ledger.requests[id].watermark;
                ledger.requests[id].watermark = previous.max(watermark);
                assert!(watermark >= previous, "caller watermark regressed");
                drop(ledger);
                let _ = ready.send(pending.is_some());
                if !matches!(command, Command::Probe) {
                    break;
                }
            }
        });
        Self {
            gate: Some(gate),
            id,
            tx,
            rx,
            worker: Some(worker),
        }
    }
    pub(super) fn release(&mut self) {
        self.gate.take().unwrap().send(()).unwrap();
    }
    pub(super) fn ready(&self) {
        self.rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    pub(super) fn start(
        log: &Arc<SegmentLog>,
        ledger: &Arc<Mutex<Ledger>>,
        inputs: Vec<Input>,
        original: bool,
    ) -> Self {
        let mut caller = Self::prepare(log, ledger, inputs, original);
        caller.release();
        caller.ready();
        caller
    }
    pub(super) fn command(&self, c: Command) -> bool {
        self.tx.send(c).unwrap();
        self.rx.recv_timeout(Duration::from_secs(5)).unwrap()
    }
    pub(super) fn finish(mut self) {
        self.command(Command::Finish);
        self.worker.take().unwrap().join().unwrap();
    }
    pub(super) fn abandon(mut self) {
        self.command(Command::Abandon);
        self.worker.take().unwrap().join().unwrap();
    }
}
impl Drop for Caller {
    fn drop(&mut self) {
        drop(self.gate.take());
        if let Some(worker) = self.worker.take() {
            let _ = self.tx.send(Command::Abandon);
            let _ = worker.join();
        }
    }
}

fn snapshot(log: &SegmentLog, ledger: &Mutex<Ledger>, from: u64, max: usize) -> Observation {
    let streams: std::collections::BTreeSet<_> = ledger
        .lock()
        .unwrap()
        .requests
        .iter()
        .flat_map(|r| r.inputs.iter().map(|r| r.stream.clone()))
        .collect();
    let before = log.watermark();
    let records = log.read(from, max).unwrap();
    let heads = streams
        .into_iter()
        .map(|s| {
            let h = log.stream_head(&s).map(|h| (h.offset, h.revision));
            (s, h)
        })
        .collect();
    let after = log.watermark();
    Observation {
        before,
        after,
        from,
        max,
        records,
        heads,
    }
}

fn same(a: &Observation, b: &Observation) -> bool {
    (a.before, a.after, a.from, a.max, &a.heads) == (b.before, b.after, b.from, b.max, &b.heads)
        && a.records
            .iter()
            .map(|r| (r.offset, r.revision, &r.stream, &r.key, &r.payload))
            .eq(b
                .records
                .iter()
                .map(|r| (r.offset, r.revision, &r.stream, &r.key, &r.payload)))
}

pub(super) fn observe(log: &Arc<SegmentLog>, ledger: &Arc<Mutex<Ledger>>, from: u64, max: usize) {
    let log = log.clone();
    let ledger = ledger.clone();
    thread::spawn(move || {
        let read = snapshot(&log, &ledger, from, max);
        ledger.lock().unwrap().reads.push(read);
    })
    .join()
    .unwrap();
}

pub(super) struct Reader {
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Reader {
    pub(super) fn start(log: &Arc<SegmentLog>, ledger: &Arc<Mutex<Ledger>>) -> Self {
        let log = log.clone();
        let ledger = ledger.clone();
        let (stop, commands) = mpsc::channel();
        let (ready, started) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut ready = Some(ready);
            loop {
                let read = snapshot(&log, &ledger, 0, 100);
                {
                    let mut ledger = ledger.lock().unwrap();
                    if !ledger.reads.last().is_some_and(|last| same(last, &read)) {
                        assert!(
                            ledger.reads.len() < 4096,
                            "bounded workload produced excessive reader states"
                        );
                        ledger.reads.push(read);
                    }
                }
                if let Some(ready) = ready.take() {
                    let _ = ready.send(());
                }
                match commands.try_recv() {
                    Ok(()) | Err(mpsc::TryRecvError::Disconnected) => return,
                    Err(mpsc::TryRecvError::Empty) => thread::yield_now(),
                }
            }
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        Self {
            stop,
            worker: Some(worker),
        }
    }
    pub(super) fn finish(mut self) {
        let _ = self.stop.send(());
        self.worker.take().unwrap().join().unwrap();
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
