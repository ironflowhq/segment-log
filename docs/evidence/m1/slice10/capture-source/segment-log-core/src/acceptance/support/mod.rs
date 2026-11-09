//! Counter fixtures under [ADR 0041](../../../../docs/decisions/0041-counter-exhaustion-refuses-without-poisoning.md).
//!
//! `SegmentLog::seed_counters` is a cfg(test) setup hook used only on an idle
//! log. It supplies synthetic watermark and head values because reaching
//! u64 limits through real appends is impractical. These are not recovery histories.
//! `fixtures::epoch_fixture` writes a legal header and optional legal first group
//! with extreme epochs, optionally followed by an invalid tail.
//! Counter scenarios count mutation calls to prove the mandated absence of writes;
//! results, watermarks and stream heads carry the behavioral assertions.
//!
//! `engine::admission_support` is cfg(test) only. Concrete type sizes anchor the
//! allowance without exporting those types. Scalar charge probes cover refusals
//! and shutdown, where a later admitted storage barrier cannot run. `finish` joins
//! a panicked writer while retaining its disconnected sender. For poison it closes
//! admission so the writer can finish draining without consuming old results.
//!
//! `SegmentLog::observe_drop` is a cfg(test) notification after Drop closes
//! admission. Cap deferral can reach storage before Drop starts, so a held call
//! cannot establish this boundary when testing queued failures during drain.
//! The seeded admission oracle uses the existing concrete-size probe to rebuild
//! the [charge formula](../../../../docs/architecture/api.md#admission-charge)
//! from test-owned inputs without calling the production charge implementation.
//! `admission_support::peak` retains the maximum successful reservation CAS
//! total, including temporary reservations before failed queue insertion. It is
//! exact after callers join, without sampling or changing admission decisions.
//! One relaxed atomic fetch_max per reservation exists only under cfg(test).
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

pub(crate) mod oracle;
