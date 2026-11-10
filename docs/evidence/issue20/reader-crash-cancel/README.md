# A long-run failure at seed 10030: a crash canceled a reader's read

A long run failed for the first time. [Issue 20](https://github.com/ironflowhq/segment-log/issues/20) section 4 asks that such a failure is reproduced from its saved pair before anyone decides where the defect is. This record does that.

## Result

**The defect was in the test harness, not in the engine, the simulator or the oracle.** The seeded workloads keep a reader thread running while they crash the simulated storage. The simulated storage cancels a call that a crash overtakes and returns `Interrupted`. The reader unwrapped that error and panicked. The window is the few instructions between the start and the end of one simulated call, so it is rare.

**The saved pair replays clean.** Recovery and every oracle check pass on the trace and the ledger that the failed run saved. Three more runs of the same seed passed, because thread timing cannot be replayed.

**A directed scenario now reproduces it every time.** It holds one read of the reader, crashes, and releases the read. Before the correction it failed with the same message at the same line as the long run. The reader now skips a canceled read and counts it, and each workload checks that the count is at most the number of canceled reads in the trace.

No acknowledged record, watermark or recovered state was wrong at any point.

## The failure

The long run for another change, with 100 trace seeds from 10000, ran in the pinned `rust:1.99.1-bookworm` container on Docker Desktop for Mac (Linux 7.0.14-linuxkit, aarch64) while the host was saturated by unrelated work. [long-run.log](long-run.log) is its output:

```text
thread '<unnamed>' panicked at segment-log-core/src/acceptance/seeded/actors.rs:145:39:
called `Result::unwrap()` on an `Err` value: Io(Kind(Interrupted))
cases=ORACLE-1..5,7..12 seed=10030 mode=PowerLossTorn variant=1 artifacts=/tmp/segment-log-traces/133740-10030-PowerLossTorn-1
```

Line 145 is the reader's `log.read(from, max).unwrap()`. Variants 1 to 3 call `sim.crash` while the reader runs.

The tree under test differed from `main` in the writer's group formation only. The reader, the simulated storage and the read path were those of `main`, and the directed scenario below fails on `main` itself.

## Replay of the saved pair

[133740-10030-PowerLossTorn-1.trace](133740-10030-PowerLossTorn-1.trace) and [133740-10030-PowerLossTorn-1.ledger](133740-10030-PowerLossTorn-1.ledger) are the pair as the failed run saved it.

```sh
SEGMENT_LOG_TRACE_REPLAY="$PWD/docs/evidence/issue20/reader-crash-cancel/133740-10030-PowerLossTorn-1" \
  cargo test -p segment-log-core --lib acceptance::seeded::saved_trace_replays_the_oracle -- --exact --nocapture
```

[replay.log](replay.log): one test passed, on the Mac host. The pair came from the Linux container, so it does not depend on the platform that wrote it.

```sh
SEGMENT_LOG_TRACE_SEEDS=1 SEGMENT_LOG_TRACE_SEED=10030 \
  cargo test -p segment-log-core --lib acceptance::seeded::threaded_seeded_traces_long_run -- --ignored --exact
```

Three runs of the seed passed on the Mac host.

## The directed scenario

`a_reader_outlives_a_read_that_a_crash_cancels` in `segment-log-core/src/acceptance/seeded.rs` commits one record, starts a reader, holds the reader's next storage read, crashes with `PowerLoss`, and releases the read. [directed-red.log](directed-red.log) is its output on `main` with only the scenario added: the same panic at `actors.rs:145`.

After the correction it passes: the reader reports one canceled read, and every state that it recorded holds the one durable record.

## The correction

- The reader's snapshot returns nothing when its read fails with `Interrupted`. Every other read error still fails the run.
- The reader counts those reads. Each workload asserts that the count is at most the number of canceled lookup reads in the storage trace. A workload with no crash while the reader runs therefore still requires zero.
- The one-shot observations that run at quiet points keep failing on any read error.

## Checks

On the Mac host at this commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (148 library tests passed, one ignored), and the long run with 100 trace seeds from 10000 (2,700 scenarios).

## Limits

- The failing interleaving was seen once. The directed scenario fixes the read that a crash overtakes. It does not show that no other harness thread can meet a canceled call.
- A reader that loses a read to a crash observes nothing for that read. The oracle's check that readers see only durable records then rests on the reader's other reads.
