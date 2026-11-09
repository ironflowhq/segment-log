# Implement a behavior with acceptance tests

This document is the source for the scenario-first workflow ([ADR 0039](decisions/0039-scenario-first-acceptance-tests-in-native-rust.md)). It applies to every implementation milestone, starting with M1. M0's CI evidence covers repository readiness.

Other facts have their own sources. The [test contract](architecture/testing.md) defines the crash model, the oracle, the required cases and their case IDs, and the [coverage status](architecture/testing.md#milestone-acceptance-coverage). The [API](architecture/api.md) defines signatures, defaults and outcomes. The [format](architecture/format.md) defines field widths.

## Write scenarios as Rust tests

Write core scenarios as ordinary Rust `#[test]` functions. Do not add a Gherkin runner or a macro-based BDD framework. M6 transport scenarios use the [Go client](#m6-transport-scenarios).

Each scenario is one test, named for the behavior it shows. Put scenarios for one coverage group in one module. The doc comment holds the Given/When/Then example and a `Cases:` line with the case IDs from testing.md:

```rust
/// Given an empty log whose next group flush is held
/// When a caller submits one record and the flush has started
/// Then the caller has no answer, `read` returns nothing and the watermark is 0
/// When the flush succeeds
/// Then the caller gets offset 0 at revision 0, and `read` returns the record
///
/// Cases: ORACLE-8, GROUP-FORMATION, READ-BOUNDED
#[test]
fn append_is_invisible_until_its_group_flush_succeeds() {
    // ...
}
```

Test names carry no milestone or requirement IDs. A later milestone can extend the scenario without renaming it. Requirements reach a test through its case IDs.

Use a small fixture builder for repeated setup, such as a log with given options on fresh simulated storage. Build fixtures from `Options`, so that a later milestone can rerun the same scenarios under other settings.

## Implement one behavior

1. Select a case from a group in the [coverage tables](architecture/testing.md#milestone-acceptance-coverage). Read its requirements, invariants and ADRs.
2. Check that the contract defines every expected outcome. If it does not, settle the outcome in api.md, or in an ADR when it changes behavior, before writing the test. Do not settle it in the test. See [boundary outcomes](#boundary-outcomes).
3. Write the scenario's doc comment and its test. Derive every expected value from the contract, not from the code under test.
4. Run the test and confirm that it fails on its behavioral assertion. If the API is missing, add only enough structure to reach that assertion. Note the observed failure in the pull request description.
5. Implement the smallest complete behavior that passes the scenario.
6. Add the rejection, crash and I/O-failure variants that the case lists. Every write-path or recovery change adds crash cases to the simulation.
7. Run the focused scenarios, the earlier scenarios and the repository checks below. Run the milestone's other checks, such as the model test, fuzzing or Go-client conformance.
8. Change the group's status in testing.md only when it moves between `Planned`, `Partial` and `Implemented`.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Do not add empty passing tests, ignored placeholders or a failing milestone suite as a substitute for implementation. Passing locally does not establish cross-platform or milestone completion evidence.

### Check coverage

The first scenario also adds a coverage check. It is a `std`-only test in the core crate. It reads the case IDs from testing.md and the `Cases:` lines from the test sources. It fails on a case ID that testing.md does not define. It prints each case ID that no test cites yet. A group can be marked `Implemented` only when the check shows a test for every applicable case in the group.

### Keep runs fast and long runs scheduled

Keep `cargo test --workspace` fast enough to run on every pull request on both platforms. Read randomized seed counts and process-kill repetitions from environment variables, with small defaults. Mark a long-running variant with `#[ignore = "long run"]` only when a named scheduled CI job runs it with `--ignored` on Linux and macOS. Add that job with the first long-running variant.

## Place the code

These paths are planned. Nothing exists yet.

| Path | Responsibility |
| --- | --- |
| `segment-log-core/src/storage/` | The private storage trait, real storage, and simulated storage under `cfg(any(test, fuzzing))` |
| `segment-log-core/src/acceptance/mod.rs` | In-crate scenarios, loaded through `#[cfg(test)] mod acceptance;` |
| `segment-log-core/src/acceptance/support/` | Fixtures, held-call helpers, the trace oracle and the coverage check |
| `segment-log-core/src/acceptance/<group>.rs` | One module per coverage group, such as `durability`, `recovery`, `revisions`, `retries`, `admission`, `poison`, `open` and `drop` |
| `segment-log-core/tests/real_storage.rs` | Public API scenarios against real storage in temporary directories |
| `segment-log-core/tests/process_crash.rs` | The [real-process harness](#real-process-harness) for `SIGKILL` and locking |
| `fuzz/` (M5) | `cargo fuzz` targets, built with a nightly toolchain |

Simulator self-tests (`SIM-MODEL`) sit next to the simulated storage. The real-versus-simulated check (`SIM-CONFORMANCE`) sits in the storage module's tests, because it needs both private implementations. M6 chooses paths for the Go client and the shared conformance cases with its protocol ADR.

## Inject storage

The [storage trait](architecture/testing.md#storage-trait) section defines how the log holds its storage. In-crate scenarios open a log through the private constructor with simulated storage. They use the same `SegmentLog`, writer and recovery code as `SegmentLog::open`. Do not add a public testing API or a second writer for tests. From M5, fuzz targets reach only the items compiled under `cfg(fuzzing)`.

The suite lives in the crate for two reasons: it can pass simulated storage through the private constructor, and it can use the test-only probes below. Assert through results that callers can see: answers, reads, watermarks and stream heads. Use storage observations only where the contract requires them, such as one data flush for each group, no extra flush for a deduplicated retry, or no write after poisoning. A probe sets up or observes a state. It never replaces an assertion on the contract.

## Control concurrency explicitly

Stop the writer at a boundary by holding a storage call. Hold the call, wait until the simulated storage reports that the call has started, assert, then release it with success or an injected error. A held call covers the flush, write and key-lookup boundaries without a hook in the writer. Hold a call only after `open` returns, so that the syncs done during creation and recovery pass.

`Pending::wait` blocks, so the suite adds a non-blocking check on `Pending` that is compiled only for tests. The check is deterministic. Once the writer has started the held flush, an answer sent too early is already waiting in the reply channel, so "no answer yet" is a reliable result.

Add a probe inside the writer only for a boundary that no storage call reaches, such as the moment after dequeue. List each such probe in the support module, compile it only for tests, and keep it free of behavior.

Do not add a clock to the storage trait. To make the members of a group deterministic, hold the previous group's flush while later submits enter the queue. Use the [zero-linger rule](architecture/api.md#option-validation) to gather queued submits. Test linger itself with one real-time liveness scenario under a watchdog.

Do not use a sleep to assume that a request reached a state or joined a group. For a negative assertion, first establish that the work reached the held call, then check that no success or reader visibility occurred. Use bounded watchdog timeouts to fail a hung scenario, never as evidence of ordering.

Give each scenario its own simulated storage. Real-storage tests use their own temporary directory.

## Record and replay randomized runs

Thread interleavings in a run with several threads cannot be replayed. Split each randomized crash test into three stages:

1. Run the workload with real threads. The simulated storage records its trace, and each caller records what it sent and what it got back.
2. Derive the crash state from the trace and a seed. This stage is deterministic.
3. Reopen and run the oracle. Recovery runs on one thread, so this stage is deterministic too.

On failure, print the case IDs, the crash mode and the seed, and save the trace to a file. The failure message names the file. Stages 2 and 3 replay from that file. Reduce a failing trace to a directed regression scenario and keep it.

Simulated `Kill`, `PowerLoss` and `PowerLossTorn` and real-process `SIGKILL` have distinct roles under [ADR 0014](decisions/0014-crash-testing-strategy.md). Do not treat a process kill as a power-loss test.

## Real-process harness

The test binary starts itself again as the child: it runs `std::env::current_exe()` with the child test's exact name and an environment variable that selects child mode. The child runs with `--nocapture`. It writes each request before it calls `append`, and each result after, as lines on its standard output with a fixed prefix, so that the parent can skip the test runner's own output. The parent stops it with `Child::kill`, which sends `SIGKILL` on Unix, then reopens the directory and runs the oracle checks that `PROC-SIGKILL` lists. A request with no recorded result is an unknown outcome. The same harness checks that a second process gets `Locked` while the child holds the log. No new dependency is needed.

Run `cargo test -p segment-log-core --test process_crash` for the process scenarios.
`SEGMENT_LOG_PROCESS_SEED` selects the first seed (default 808), and
`SEGMENT_LOG_PROCESS_REPETITIONS` selects consecutive seeds (default 3).
Each run kills at a seeded request-event boundary while six callers continue working.
This varies the kill point without using a sleep to infer ordering.
The protocol serializes and flushes each line; request fields use hex encoding.
Only complete result lines count as answers. The oracle uses the pre-recovery
file to constrain written group order, and the request ledger supplies expected
record contents, revisions, atomic submit membership and retry classification.
It does not infer a transaction from a regrouped retry acknowledgment.
One admitted result handle is deliberately abandoned, so unknown-outcome coverage
does not depend on whether a pipe event still describes unfinished work at kill time.
The parent waits for its post-admission abandonment event before enabling the kill.

Each process repetition prints its seed, mode and transcript path. A failed repetition keeps
its temporary directory and `protocol.txt`; a passing scenario removes them.
The transcript records the pipe, including test-runner output and any partial
last line. It supports ledger inspection, not deterministic replay of scheduling.
Child cleanup kills and reaps the process before joining its pipe reader.
Pipe and exit waits have watchdogs. A scenario-wide watchdog also bounds open,
recovery and retry waits; on expiry it cleans up the child and aborts the test
binary so a stuck local writer cannot outlive the test.

## Choose test techniques

- Start each behavior with directed scenarios. Randomized and model tests broaden a covered contract; they do not replace its directed examples.
- Use `proptest` for deterministic, single-threaded properties, such as codec round trips, charge arithmetic and the M2 model driven one submit at a time. Commit its regression files.
- Use the seeded trace harness above for crash runs with several threads.
- Do not add loom or shuttle. They replace the synchronization types inside the core, which needs an ADR under [ADR 0012](decisions/0012-no-log-crate-dependency.md).
- `cargo-nextest` is optional. It gives per-test timeouts that end the process, which helps when a watchdog leaves a hung writer thread.
- `cargo-mutants` is optional. Run it now and then on the writer and recovery modules to find logic that the scenarios do not catch.

## Keep earlier scenarios

Keep earlier scenarios as regression tests, and extend them when a later milestone adds behavior. When a milestone changes a supported outcome, keep the scenario's intent and update its expectation under the governing ADR. For example, M1 refuses multiple segments with `Unsupported`, and M3 replaces that refusal with multi-segment recovery. Do not keep an obsolete refusal as a permanent contract.

From M3, rerun the M1 scenarios through their fixtures with a small `roll_bytes`, so that retries, recovery and crash cases also run across rolls. From M4, extend them with sealing where it applies.

## Boundary outcomes

[ADR 0043](decisions/0043-zero-linger-and-empty-read-boundaries.md) settles the boundary outcomes needed by deterministic scenarios. Use their definitions in api.md:

- [Zero linger](architecture/api.md#option-validation) governs queued submits in deterministic group scenarios.
- [Read boundaries](architecture/api.md#target-signatures-and-defaults) govern `READ-BOUNDED`.

## Start M1

Build the simulated storage, its self-tests and the first scenario together. The first scenario is the example under [Write scenarios as Rust tests](#write-scenarios-as-rust-tests). It needs the minimal open, append, wait, read and watermark path, and `Empty` at revision 0. It also checks that the group makes exactly one data flush.

Then add slices in dependency order:

1. Crash and reopen after an ack under each simulated crash mode. The record keeps its offset, revision, stream, key and payload. A `PowerLoss` right after reopen gives the same log. Add the hand-made recovery fixtures and the faults during open.
2. Expected revisions and whole-submit rejection, including a rejected revision that uses no offset.
3. Key classification against durable records: repeated keys, key scope, conflicts, mixed submits and fully matching retries.
4. The same classification against pending records, then pending and regrouped retries that wait for durability.
5. Admission: options, precedence, budgets, size refusals, charges and reservation lifetime.
6. Poisoning and lookup failures, then `Drop`.
7. Open boundaries and locking, then the real-process harness.
8. The randomized trace oracle across all groups, and its scheduled long run.

Keep roll, sealing, `LogReader` and transport behavior in their [assigned milestones](prd.md#milestones).

## M6 transport scenarios

M6 adds Go-client scenarios for the cases in [M6 transport order and conformance](architecture/testing.md#m6-transport-order-and-conformance). Keep the conformance cases as data that both transports and the Go client share. Do not port the core's crash scenarios to Go. CI gains a Go toolchain job when the first transport scenario lands.

## Evidence

Acceptance scenarios give correctness evidence. Each milestone also needs its own completion artifacts. M1 needs the syscall, directory-sync and Linux benchmark artifacts in the [completion gate](architecture/testing.md#m1-completion-gate). Later gates keep their own measurement, fuzzing, transport and deployment evidence. Keep missing evidence separate from a measured performance miss, under the milestone's pass criteria.
