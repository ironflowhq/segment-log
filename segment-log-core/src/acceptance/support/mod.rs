//! Counter fixtures under [ADR 0041](../../../../docs/decisions/0041-counter-exhaustion-refuses-without-poisoning.md).
//!
//! `SegmentLog::seed_counters` is a cfg(test) setup hook used only on an idle,
//! log. It supplies synthetic watermark and head values because reaching
//! u64 limits through real appends is impractical. These are not recovery histories.
//! `fixtures::epoch_fixture` writes a legal header and optional legal first group
//! with extreme epochs, optionally followed by an invalid tail.
//! Counter scenarios count mutation calls to prove the mandated absence of writes;
//! results, watermarks and stream heads carry the behavioral assertions.
mod coverage;

pub(crate) fn bounded(f: impl FnOnce() + Send + 'static) {
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = tx.send(result);
    });
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("scenario watchdog");
    worker.join().unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
pub(crate) mod trace;

pub(crate) mod fixtures;

pub(crate) mod raw;
