# Issue 20: hardware, hosted runs, and warning evidence

Investigate the M1 qualification gaps after merge without deciding the M1 gate.
This record covers [issue #20](https://github.com/ironflowhq/segment-log/issues/20)
sections 3, 4, and 5. It does not close that issue or gate #17.

## Source and environment

The merged comparison baseline is `911e809e72fc9c9ef5571f7022c89e55a48d6cb1`
(PR #19), rather than the pre-merge head linked in the issue. The simulator gate
correction is `64cc15dc250c855b28c0d012465c3e3780a382cc`.

Local checks ran on 9 November 2026 on Darwin 27.0.0, arm64, with
Rust 1.99.1 (`58cf8f9c35b355199727627d07ae34a4e2846d67`, LLVM 23.1.1),
Cargo 1.99.1 (`5f94df478`), and Python 3.14.8. This is a Mac development
worktree, not a physical Linux local-NVMe qualification host. No heavy local
benchmark ran for this slice. These checks make no performance comparison.
Raw working output is ignored under `.audit/slice3/`. The selected logs here
are direct captured output, not reconstructed behavioral failures.

## Simulator warnings

At the baseline, this command exits 101 with 24 dead-code errors:

```sh
RUSTFLAGS='--cfg fuzzing -D warnings' cargo check -p segment-log-core
```

The same command exits 0 after the correction. The
[before log](hosted/fuzz-strict-before.log) and
[after log](hosted/fuzz-strict-after.log) preserve the actual results.
The non-strict baseline command also reproduced the 24 warnings from the issue.
These are compilation failures under warnings-as-errors, not engine behavioral
failures or historical initial-red evidence.

Every warning is in `src/storage/sim.rs`. The module is private to the crate.
Its consumers are the storage unit tests and acceptance tests, all under
`cfg(test)`. The workspace contains no fuzz target or simulator adapter for a
non-test build. `--cfg fuzzing` had compiled an unused copy of the simulator.
The correction changes its gate from `any(test, fuzzing)` to `test`.

The alternatives were:

- Keep the gate and document the warnings. This preserves an unused compilation
  mode and leaves warnings-as-errors broken.
- Suppress `dead_code` for the module. This would also hide newly unused items.
- Compile the simulator with its existing consumers under `cfg(test)`. This
  removes the unused copy without changing simulator logic or lint severity.
  This is the selected correction.

This comparison concerns compilation and actual consumers, not speed.
A non-test `--cfg fuzzing` check now checks the production crate only. It must
not be described as fuzz execution or simulator coverage. Simulator execution
remains in the unit and acceptance tests. A future fuzz target needs an explicit
adapter and its own exercised build configuration.

## Hosted execution snapshot

Before dispatch, this command returned an empty list on 9 November 2026:

```sh
gh run list --repo ironflowhq/segment-log --workflow long-run.yml --limit 30 \
  --json databaseId,event,headSha,status,conclusion,url,createdAt
```

The active workflow first appeared on the default branch that day. Its cron is
`23 4 * * *` UTC. No scheduled execution was available in that snapshot. A manual
execution cannot prove that the scheduler works.

The authorized command was:

```sh
gh workflow run long-run.yml --repo ironflowhq/segment-log --ref main
```

It started [manual run 42654692594](https://github.com/ironflowhq/segment-log/actions/runs/42654692594)
on the merged baseline SHA above. The [initial API snapshot](hosted/manual-run-initial.json)
records both jobs as in progress:

- [Linux, ubuntu-24.04](https://github.com/ironflowhq/segment-log/actions/runs/42654692594/job/129289934512).
- [macOS, macos-latest](https://github.com/ironflowhq/segment-log/actions/runs/42654692594/job/129289934650).

Each job requests 100 trace seeds starting at 10000, all three crash modes and
nine variants per seed, followed by 30 process-crash repetitions starting at
10000. This is the configured scope, not a completed result count. The run tests
the merged baseline, not this PR. No sleep or polling loop was used.
The [final API snapshot](hosted/manual-run-final.json) supersedes the initial
pending snapshot. Both platform jobs completed successfully, including the trace
and process-crash steps. Failure-artifact upload steps were skipped. This proves
manual execution on the baseline, not scheduled execution or failure retention.

## Failure artifact audit and remaining proof

Source inspection at the merged baseline found:

- The trace runner saves `.trace` and `.ledger` with the same filename stem under
  `SEGMENT_LOG_TRACE_ARTIFACT_DIR`. It retains the pair on workload or replay panic
  and deletes successful pairs.
- The process step sets `TMPDIR` to the `process` child of that directory. Its
  scenario retains `protocol.txt` and the log directory when the test panics.
  The process ledger is the literal protocol transcript, not a `.ledger` file in
  the simulator replay format.
- `actions/upload-artifact@v4` runs on failure, includes hidden files, and uploads
  the entire directory with a platform-specific artifact name. `if-no-files-found`
  is `ignore`, so a failed run alone does not prove that an artifact exists.
- The process step uses the default success condition. A prior trace failure
  skips it. Artifact completeness must be judged against the stages that ran.
- The trace watchdog saves a pair only if its ledger lock is available. Runner
  termination or failures before artifact creation can leave no complete pair.

This establishes intended paths, not an observed failed hosted upload. The
historical local path-proof logs in `m1/validation/` are passing runs and cannot
prove failure retention. This slice did not inject a hosted failure or retrieve a
failure ZIP. A successful manual run may correctly have no failure artifact.

To complete the artifact claim, a controlled failure must produce a downloaded
ZIP containing a usable trace/ledger pair or the failed process scenario's
protocol and segment files, as applicable. Replay the saved simulator pair with
`SEGMENT_LOG_TRACE_REPLAY` before attributing a failure. Preserve process
artifacts in their native format. Scheduled execution and hosted failure-upload
verification remain open evidence tasks; neither is recorded as a pass.

## Hardware qualification remains unresolved

The owner confirmed that no physical local-NVMe host is available. A Hetzner VPS
fallback is authorized if local execution is unavailable, but this slice did not
provision a host. A VPS measurement would still need its virtual storage stack
identified and would not establish physical local-NVMe qualification.

The existing Mac and Docker guest evidence does not identify physical device
cache behavior or establish end-to-end flush propagation. Keep these claims
separate: syscall targets and counts, successful directory calls, workload
performance, and qualification of the actual storage stack. ADR 0032 and the
V1–V11 requirements in `docs/architecture/volume-requirements.md` still govern
actual-stack qualification before M6 guest benchmarks or production durability
claims. No target host, device, filesystem, or power-loss result is invented here.

## Evidence history

The original archive's limits remain in force. Some behavior predates its
scenario, some initial-red output is unavailable, and supplemental tests and
mutation checks do not recover that history. The existing provenance table,
transcriptions, and immutable issue links are unchanged. This new compile-failure
capture supports only the warning correction described above.

## Local verification

Commands run from the repository root against the correction commit:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 -B -m unittest discover -s scripts -p 'test_m1_*.py'
```

Results are recorded in the [validation summary](hosted/validation.md).
The workspace suite and an isolated codec rerun aborted on the heavily loaded
host. The local abort cause remains unresolved. Exact-head hosted CI subsequently
passed the full workspace suite on Linux and macOS, as recorded in the
[CI snapshot](hosted/pr21-ci.json) for
`03293670714624269d21b375a5a64cb23a1b7c01`. The targeted simulator suite
passed all 23 tests. The strict compilation, formatting, Clippy, and Python checks
passed. The normal small
measurement scenario is part of the workspace suite; no million-record or
performance diagnostic workload was requested for this slice.
