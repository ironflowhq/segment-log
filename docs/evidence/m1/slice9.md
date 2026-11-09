# Slice 9 trace oracle and coverage audit

This is local implementation evidence based on parent `3b2fb72ea15e41b3edd8d108bc0dd937a5d6fb5d`.
It does not close M1. The parent owns review, CI verification and issue #17.

## Harness contract

`acceptance/seeded.rs` owns scenario selection and replay. `seeded/actors.rs`
owns caller threads, reader threads, channels and the watchdog. `seeded/workloads.rs`
owns seeded inputs and storage boundaries. `support/oracle.rs` predicts from
literal requests and independently parsed written identities. `seeded/saved.rs`
pairs a versioned caller/reader ledger with the existing storage trace format.
No production code, dependency or public API changes.

Each seed runs nine workloads under each of Kill, PowerLoss and PowerLossTorn.
The seed varies payload bytes and sizes, atomic record counts, stream order,
keyed/mixed/unkeyed originals, stale expectations, retry order, short-write
length, lookup fault and pressure limits. Variants select successful completion,
crashes at append or flush, partial writes, ENOSPC, failed data sync, lookup
failure and concurrent admission pressure. The two flush-crash variants share
a boundary; they are not two distinct crash boundaries.

Callers record literal inputs before append and actual answers afterward.
`Unknown` includes an admitted abandoned handle, lost answers and Indeterminate.
Structural, size, capacity and writer refusals remain distinct recorded answers.
Reader threads record bounded reads, heads and watermarks. A reader remains active
from a confirmed initial snapshot through release of the held storage call until
callers finish. Identical complete observations are coalesced; every distinct
record, head and watermark observation is retained. Caller watermarks
retain the maximum observed value. Storage holds and admission notifications
establish states. Concurrent pressure callers race for capacity; the other
callers enqueue in seeded order while their earlier results remain outstanding.

The oracle reconstructs original submit membership from inputs and written
identities, validates full group boundaries, and compares literal contents and
independently assigned revisions. It also bounds recovery by the independently
scanned readable valid prefix. Observed recovery length only selects a prefix
within those bounds. Regrouped retry acknowledgments never define transactions.
Unknown requests replay against the reconstructed history with their original
expectations. No production codec, classification or charge helper supplies
expected data. Permanent-size checks rebuild the API charge formula from owned
input capacities and the existing test-only concrete-type-size probe. The
production charge function is never called. Pressure checks require an eligible
competitor to enter the queue, and bound admitted count and bytes. Accounting
extremes retain their directed tests.

Fault variants with odd seeds reopen in the same boot. Even seeds crash first.
Every successful recovery is followed by PowerLoss and PowerLossTorn before
retry work. Old callers finish and the writer is joined through Drop before
any new engine opens. A canceled held call poisons the old writer rather than
letting it drain unknown requests successfully. Holds disconnect on unwind,
caller guards join their actors, and a 30-second watchdog bounds a stuck test.
At expiry its artifact attempt gets at most one further second before process
abort, even if that attempt blocks or panics. Successful runs remove temporary trace pairs. Failures print the seed,
mode, variant and artifact prefix and retain both files.

Saved replay reruns storage reconstruction and the recovery oracle. It does
not reproduce thread scheduling or rerun live held-call probes. The live workload
also checks immediate terminal answers, pending waiter outcomes, queued/deferred
Poisoned outcomes and no write or flush after failure. Replay independently
checks the recorded answers and recovered history. A watchdog attempts a paired
snapshot before aborting; if the ledger lock itself is stuck, it cannot take
that snapshot.

## Reproduce

Fast default: `cargo test --workspace`. The new suite defaults to two seeds,
900 and 901, for 54 scenarios. The round-trip and reduced regression tests run
separately. `SEGMENT_LOG_TRACE_SEED` selects the first seed and
`SEGMENT_LOG_TRACE_SEEDS` selects a positive count up to 10000.

```sh
SEGMENT_LOG_TRACE_SEEDS=100 SEGMENT_LOG_TRACE_SEED=1000 cargo test -p segment-log-core --lib acceptance::seeded::threaded_seeded_traces_long_run -- --ignored --exact --nocapture
SEGMENT_LOG_TRACE_REPLAY="$PWD/docs/evidence/m1/seeded/pressure-1000" cargo test -p segment-log-core saved_trace_replays_the_oracle -- --nocapture
SEGMENT_LOG_TRACE_KEEP=1 cargo test -p segment-log-core saved_trace_replays_the_oracle -- --nocapture
```

The replay variable takes the artifact prefix or either file path. Both sibling
`.trace` and `.ledger` files must exist. `SEGMENT_LOG_TRACE_ARTIFACT_DIR` selects
the output directory. Without it, artifacts use `temp_dir()/segment-log-traces`.
The keep command prints the new pair's prefix for a second invocation.

The named `m1-long-run` job in `.github/workflows/long-run.yml` has a Linux/macOS
matrix, daily cron, manual dispatch, a 30-minute timeout, 100 seeds starting at
10000 and an explicit exact ignored-test command. It also runs 30 real-process
repetitions using the existing process settings. Failure artifacts are uploaded.
The trace artifact directory is absolute, and process `TMPDIR` points beneath
the uploaded directory. Hidden files are included. Local path checks confirmed
both trace pairs and process transcript paths resolve there under Cargo.
Normal PR CI exercises the non-ignored harness through its existing workspace
command. Cron and manual dispatch cannot be claimed before the workflow reaches
the default branch. No hosted scheduled or manual run was performed here.

## Red evidence and limitations

The new reader-ledger round-trip assertion observed zero retained observations
before reader serialization was implemented. The original output is retained
in `red/slice9-reader-ledger.log`. Initial decoder bound errors are fixture
corrections, not behavioral-red evidence. Engine scenarios are supplemental;
no production behavior was changed or claimed red.

The first 20-seed sample found a real harness failure at seed 1000, Kill,
pressure variant 8. The oracle used the decimal representation of `u64::MAX`
as a fresh stream/key identifier. Its verification submit exceeded the saved
small group cap, so the engine correctly returned TooLarge. The preserved
`seeded/pressure-1000.trace` and `.ledger` are the original small failing
scenario. `recovery_verification_respects_the_saved_small_group_cap` reduces
reproduction to deterministic replay of that pair. Its original directed red
and randomized failure output are preserved beside it. The fix uses a short
fresh identifier; it changes no saved options and drops no seed.

Parent review exposed two additional oracle blind spots. Temporary engine
mutations returning `Full` for every pressure competitor or `TooLarge` for a
small eligible stream both passed the original seeded suite. Both mutations now
fail the intended assertions. Negative tests observed behavioral reds before
the oracle rejected false permanent refusals and reader states drawn from the
middle of a committed group. These are harness corrections, not production
defects. Original replay instructions also used the wrong working directory;
the absolute-path command above was executed successfully after correction.
Review artifacts are labeled separately in the provenance table.
The blocked-dump watchdog experiment extracts the actual guard source and uses
a 20 ms deadline. The earlier synchronous dump prevented termination; the
bounded asynchronous dump still reached SIGABRT. All experiment processes were reaped.

## Coverage audit

These mappings audit applicable M1 clauses, not just case-ID presence. The
status changes reconcile existing directed coverage with the new threaded
oracle. They do not imply that every fixed boundary became randomized.

| Former Partial row | Evidence closing applicable M1 coverage |
| --- | --- |
| M1-SIMULATOR | `storage/sim/tests.rs` directly checks page choices, sector preservation, file lengths, separate directory durability, failed SyncData/SyncAll, held calls, boot cancellation and chained trace replay. `storage/tests.rs` compares real and simulated create/read/write/append/truncate/remove/list/lock results. Saved metadata and oracle replay now include the caller and reader evidence. |
| M1-CODEC | The four `acceptance/codec.rs` scenarios cover group/header round trips, the CRC check value, malformed fields, binary input and length limits. This is an audit of existing complete M1 coverage; footer/seal/locator remain M4. |
| M1-DURABILITY | `durability.rs` covers visibility before flush, zero-linger grouping, cap deferral and read boundaries. Threaded traces add recorded reader observations, maximum watermarks and crash replay for ORACLE-1/8/9. |
| M1-ATOMICITY | Written identities select literal original submits, each contained in one group. Replay checks the full prefix, original atomicity, positions and fresh post-recovery appends for ORACLE-2/3/10. Directed cap and multi-record scenarios remain. |
| M1-RECOVERY | `recovery.rs`, `recovery_fixtures.rs` and `open_faults.rs` cover all M1 hand-made tails, header/filename errors, creation/recovery call cuts, failed rewrite/sync and identical durable rewrites. Threaded traces add literal ORACLE-1 through ORACLE-5 across all crash modes and same-boot failed-sync recovery. Identity comparison waits for multiple segments under ADR 0042. |
| M1-COUNTERS | `counters.rs` covers last offsets/revisions, whole-submit refusal, input precedence, continued use and epoch refusal before mutation, including an invalid tail. Durable/pending-key modules cover counter-free matches and key precedence. Synthetic setup remains explicit; this is an audit of existing complete coverage. |
| M1-REVISIONS | `revisions.rs` covers Empty/At on absent and revision-zero streams, same-submit/pending visibility, first input failure and no consumed keys/offsets. Seeded traces vary same-stream atomic and later submits, stale matches and whole-submit rejections. |
| M1-RETRY-CLASSIFICATION | `admission.rs`, `durable_keys.rs` and `pending_keys.rs` cover repeated keys, whole-log scope, first conflict, keyed/unkeyed partials in either order, conflict precedence and regrouped/subset/reordered pending/durable/mixed matches. Threaded traces check these against varied literal histories. |
| M1-UNKNOWN-OUTCOMES | `unknown_retries.rs` and `durable_keys.rs` cover all eight table rows and distinguish regrouped selections from atomic originals. Seeded faults add unknown keyed/mixed/unkeyed originals, mixed pending/durable selections and abandoned stale requests whose absent keys still produce WrongRevision. No impossible partial atomic history is constructed. |
| M1-REOPEN-HISTORY | Durable-key and pending-key scenarios retain original key positions and complete history after reopen. The seeded oracle discards engine caches, validates all input-derived heads/reads/watermarks, retries retained keys, and performs immediate chained power losses. No nonexistent M4 cache file is deleted. |

Previously Implemented pending retries, admission, poison, Drop, open boundaries
and process crash rows stay Implemented. Their directed scenarios remain
unchanged. M1's overall row stays Partial. Missing completion artifacts remain
Linux group-flush syscall evidence, actual directory-sync evidence on both
platforms and the reproducible Linux benchmark under real hardware/software
conditions. M2's exhaustive single-thread revision/key model and all later
milestone cases remain outside this change. Historical missing raw reds remain
missing; this slice does not repair that provenance.

## Local validation

The pre-review delegate run had 133 passing tests. Parent review added two
negative oracle tests. The final workspace run has 131 passing core tests, one
ignored long-run core test, three passing process tests and one passing real-storage
test. This is 135 passing tests, including all 129 baseline tests unchanged,
plus six new non-ignored tests. The seventh new test is the scheduled long run.
The fast trace suite uses seeds 900 and 901 for 54 scenarios. The saved round-trip
uses seed 919 and the reduced pressure fixture uses seed 1000.

The larger local command above uses seeds 1000 through 1099, for 2700 scenarios.
The first failed sample used seed 1000; the corrected 20-seed sample covered
1000 through 1019 for 540 scenarios. Both later samples include the failure seed.
The reviewed 100-seed sample reran all 2700 scenarios with continuous readers and
the stricter admission and snapshot checks. It includes concurrent pressure,
same-stream pending advances, abandoned stale requests and storage-flush checks.

Validation logs are under `validation/slice9-*`. Formatting, strict Clippy,
workspace tests, focused seeded tests, the saved replay command, case-ID
validation and diff checks passed locally. `RUSTFLAGS='--cfg fuzzing' cargo check
-p segment-log-core` compiled with the same 24 unused simulator warnings as the
baseline. No Linux execution or hosted workflow execution is claimed. The
[environment record](validation/slice9-environment.log) identifies the local
macOS host and pinned toolchain. No performance speedup was measured.

The unknown-ID check reports only later-milestone or optional uncited cases:
FUZZ-DECODER, FUZZ-RECOVERY, HANDOFF, MODEL, ORACLE-6, ORACLE-13,
PROC-LAZYFS, READER, ROLL-FOOTER and the three TRANSPORT cases. The audit found
no remaining untested applicable M1 case clause. That assessment is submitted
for parent review, separate from the still-missing completion artifacts.
