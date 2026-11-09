# Slice 10 completion evidence

This record supplies M1 completion artifacts for owner review, based on parent
`adc349bafc9d5c8b0fec1e860435a69f2caeb576` (PR #18).
It does **not** mark M1 complete. The PRD and overall coverage row are unchanged.
The [gate](../../architecture/testing.md#m1-completion-gate) remains the authority.

## Reproduce

Run from the repository root with Python 3 and the pinned Rust toolchain.
Docker must already be running for the Linux runner; the captured host used Docker Desktop.
The Linux runner pins the Rust image by digest, mounts source read-only, builds
in the disposable container, and writes only the requested output directory.
It grants ptrace access to that container and sets its PID limit to 4096.
It installs tracing tools inside that container, without host installation.
Its exit trap removes only its own uniquely named container.

```sh
scripts/m1-evidence-linux.sh "$PWD/.audit/reproduce-linux"
scripts/m1-evidence-macos.sh "$PWD/.audit/reproduce-macos"
python3 scripts/m1-verify-syscalls.py docs/evidence/m1/slice10/linux/syscalls.trace
python3 scripts/m1-summarize.py docs/evidence/m1/slice10/linux
python3 -B -m unittest discover -s scripts -p 'test_m1_*.py'
```

The Linux script uses the pinned toolchain, release profile, 1,000 caller
threads, and 1,000 records per caller per repetition. It runs a 20-round warmup
and five comparable fresh-log repetitions. A separate traced diagnostic uses
50 rounds. A separate CPU diagnostic uses 1,000 rounds and does not enter the
reported statistics. To run just the CPU diagnostic:

```sh
scripts/m1-evidence-linux.sh "$PWD/.audit/reproduce-profile" 1
```

Both normal evidence scenarios also run in `cargo test --workspace`, with a
small real-storage workload. They are not ignored or empty placeholders.
Environment variables `SEGMENT_LOG_BENCH_CALLERS`, `SEGMENT_LOG_BENCH_ROUNDS`,
and `SEGMENT_LOG_DEVICE_ROUNDS` scale the same scenario. Defaults are 4, 4, and
8. `SEGMENT_LOG_EVIDENCE_DIR` chooses a parent directory.
`SEGMENT_LOG_GROUPS_OUT` saves every measured group's submit and frame-byte
count. No public test interface or runtime dependency was added.

## Captured platform calls

[Original trace](slice10/linux/syscalls.trace),
[executable verification output](slice10/linux/syscalls-verified.txt), and
[phase results](slice10/linux/sync-phases.log) distinguish:

| Phase | Segment fdatasync | Segment fsync | Log directory fsync | Parent directory fsync |
| --- | --- | --- | --- | --- |
| Creation | 1 header | 1 recovery rewrite | 2 | 1 |
| First new committed group | 1 | 0 | 0 | 0 |
| Second new committed group | 1 | 0 | 0 | 0 |
| Fully deduplicated retry | 0 | 0 | 0 | 0 |
| Drop | 0 | 0 | 0 | 0 |
| Reopen recovery | 0 | 1 | 1 | 1 |
| Recovered Drop | 0 | 0 | 0 | 0 |

All listed calls returned success. Parent review strengthened the verifier to
check the resolved segment, log-directory and parent-directory targets, as
well as totals. Wrong-file and substituted-parent traces are rejected by
regression tests. The [reviewed Linux trace](slice10/reviewed-linux/syscalls.trace)
and [target-aware verification](slice10/reviewed-linux/syscalls-verified.txt)
repeat the check against the final helper code.
`strace -f -tt -T -yy` follows the writer
thread and resolves descriptors to paths. The phase test also validates the
literal records and returned positions. Its measured group count stays at two
after the retry. Provisioning syncs appear in a separate initial phase: the
harness makes temporary ancestors durable before calling the engine.
They are not engine initialization or group flushes.

The [macOS directory run](slice10/macos/directory.log) records successful
`Real::call` results for both log and parent directories during creation and
recovery. The forwarding wrapper calls the actual implementation first and
records its returned result. This is actual-storage call evidence on APFS,
not a macOS syscall trace. The Linux trace independently checks the kernel
calls. Neither result certifies power-loss behavior or the storage contract.

## Benchmark design and correctness

The data shape is one keyed record per submit. Each caller owns one stream and
waits for each result before its next revision. The stream is its eight-byte
little-endian caller number. The key contains that number followed by the
eight-byte revision. Payloads have 256 bytes: the first sixteen carry those
numbers and the rest are `0x5a`. No random generator or seed is involved.
Scheduling changes group boundaries and offsets, so those are recorded, not
claimed reproducible from a seed.

Every attempt is counted. `Full` increments its own counter and retries the
same literal input. Other admission or commit errors fail the run immediately.
Successful runs report zero such failures only after consuming every result,
checking revisions and deduplication flags, and validating every stored field.
The throughput numerator counts successful committed records, never refusals.
A sorted latency vector uses nearest-rank p50, p99, and p999.

The latency timer starts before constructing a submit and ends after
`Pending::wait`. It includes allocation, admission, any retries, writer work,
flush and delivery. Throughput starts before releasing the ready callers and
ends after joining them. Thread creation, device calibration, open, whole-log
validation and reopen are outside that interval. Reopen and initial open have
their own timers. Validation compares reads and heads with literal inputs and
caller-returned offsets, then repeats after dropping and reopening the engine.
A read never supplies expected contents.

Every repetition starts with a fresh log, zero records and one 64-byte segment
after open. It ends with 1,000,000 records, 1,000 streams, 1,000,000 retained
keys, and one segment. The exact sizes are in the table and raw logs.
Reopen starts and ends with those same counts and sizes. No history is
truncated, rolled or indexed. The default roll option is validated but inactive
in M1. Warm caches and an immediate full-scan reopen are intentional.
This is not a cold-cache boot measurement.

The device-only measurement runs **first** in every repetition. It appends
262,144 bytes to a growing file, then times `Real`'s `sync_data`, 1,000 times.
The file grows from zero to 262,144,000 bytes. Append time is outside the
device-sync timer. This includes the flush's new-size metadata obligation.
The measurement describes the exposed virtual storage path, not an isolated
physical device.

All options come directly from `Options::default()` and are printed in each
raw run: linger 1 ms, group cap 1,048,576 bytes, queue capacity 4096,
admission budget 16,777,216 bytes, and inactive roll setting 67,108,864 bytes.
There was no tuning experiment or default change.

The exact reservation high-water probe records `previous + charge` from
every successful reservation CAS using a relaxed atomic maximum.
A later release cannot erase an earlier peak. It is read after callers join.
It includes transient pre-queue reservations, including ones whose queue
insertion could fail. There were no such refusals in the recorded baseline.
This is a charge-accounting peak, not RSS.

Instrumentation overhead remains in the measurements: one additional atomic
maximum per reservation, caller clocks and latency storage, and a forwarding
storage wrapper with a clock, metric-name allocation and mutex updates.
Production admission, writer and recovery behavior are unchanged. The wrapper
parses the frame record count only to measure group size. Because each submit
has one record, that count is the new-submit count. It does not supply a
correctness oracle. Group append and successful flush counts must agree, and
their record and byte sums must match the caller ledger and actual file size.

A five-minute watchdog bounds each evidence test. All harnesses share an
I/O-independent deadline guard. Diagnostics run separately with at most one
additional second before abort; locked stderr or a blocked dump cannot prevent
termination. A subprocess regression covers both cases with core dumps disabled.
Scoped caller threads join
on both success and unwind. A spawn failure cancels the start channels and
joins every caller already created. Test directories are removed after the
engine closes. Timeout aborts the test process; the container exit trap removes
guest data. Host timeout leftovers, if any, stay only under the chosen audit
output directory.

## Hardware and deviations

[Linux environment](slice10/linux/environment.txt),
[Docker versions](slice10/linux/docker-version.txt),
[Docker resources](slice10/linux/docker-info.txt), and
[macOS environment](slice10/macos/environment.txt) are captured outputs.

The host is an Apple M4, Mac16,10, with 10 physical/logical cores and
25,769,803,776 bytes of memory. It runs macOS 27.0.1 build 26A434.
The macOS directory check uses the workspace's APFS volume on an external
solid-state device reported through PCI-Express, with 4096-byte blocks.

The Linux environment is a Docker Desktop 4.98.0 VM, Engine 29.9.2,
Linux 7.0.14-linuxkit on aarch64, with 10 virtual CPUs and 8,319,438,848 bytes
of guest memory. The container uses Debian 12 bookworm and Rust 1.99.1
(`58cf8f9c3`, LLVM 23.1.1), matching the pin. Rustfmt and Clippy are installed
through that pinned toolchain. The environment output records strace's version.

Benchmark data lives in the guest's writable overlay filesystem, not the
macOS source bind. Docker reports the containerd overlayfs snapshotter.
The mount output exposes ext4 on virtual `/dev/vda1` for container metadata.
The overlay backing path is captured, but the host location of Docker's disk
image and its exact physical backing device are not established. The
[conventional Docker.raw path](slice10/host-backing.txt) was absent. Host root storage is reported as
internal solid-state APFS through Apple Fabric. That does not prove where
Docker's backing image lives. Device volatile write-cache state, flush
propagation, sector atomicity, metadata-error behavior, and the hypervisor
backend are unknown. No serial numbers were collected.

There is no container CPU quota or memory limit beyond guest resources.
The task container permits 4096 PIDs; all 1,000 callers started.
Per-run load averages are saved. Host load and unrelated Docker workloads are
not controlled; no unrelated process or container was changed.

This is Linux evidence under the requested caller count, payload and default
linger, but **not** a physical Linux local-NVMe baseline or deployment
qualification. The target comparison is diagnostic under these deviations.
The M4 footer-based open and seal-to-exit targets are N/A. The M6 idle server
RSS, stripped server binary size and socket metrics are N/A. GNU time's
maximum RSS is reported for the entire test process, including caller stacks,
literal verification buffers, retained keys and reopen. It is not idle RSS.

## Results

See [five-run table and median/range summary](slice10/summary.md), the five
`slice10/linux/run-N.log` files, and `groups-N.tsv` files. Every group appears
in the TSVs, with frame bytes including the envelope. The raw logs give the
min/p50/p99/p999/max distributions and totals. All repetitions use the same
dataset dimensions. End sizes differ only with group framing.

The baseline exceeds the throughput threshold numerically, but cannot establish
a target-condition pass on this virtualized stack. Every repetition misses
the provisional latency bound computed from that repetition's measured
device p99. These are reported findings, not a change to the defaults or target.
No correctness or durability failure was observed.


Additional measured fields (each run starts with zero records and one 64-byte
segment, and reopens its entire ending dataset):

| Run | Initial open ms | Groups/flushes | Smallest group submits | Peak charges bytes | Whole-process maximum RSS KiB | Group flush time / append interval |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 1.063 | 1031 | 250 | 480000 | 866860 | 8.9% |
| 2 | 0.825 | 1016 | 189 | 480000 | 773716 | 7.0% |
| 3 | 0.753 | 1013 | 276 | 480000 | 774668 | 7.8% |
| 4 | 0.770 | 1016 | 226 | 480000 | 767428 | 7.1% |
| 5 | 0.887 | 1013 | 150 | 480000 | 774416 | 7.2% |

Every run has a group-submit p50, p99 and maximum of 1,000, and corresponding
frame-byte quantiles of 296,032. The complete per-flush distributions remain
in the TSVs. Every run has zero admission refusals, other admission errors,
commit errors and correctness errors. Peak charges are 480,000 bytes against
the 16,777,216-byte budget, with zero current charges after completion.

## Limiter and plausibility

The separate [CPU profile](slice10/linux/profile.json) measures task CPU with
Linux schedstat nanoseconds. Samples retain maximum observed cumulative CPU
per task, so exited tasks can lose their final sampling interval.
The separate [syscall profile](slice10/linux/diagnostic-strace.txt) shows
caller futex waiting and the expected flush calls; summed thread syscall time
is not elapsed time. Expected filesystem probes and futex races can report
syscall errors while all storage operations and caller results succeed.
Neither diagnostic enters throughput or latency statistics.

The diagnostic sampled at least 5.871 writer CPU-seconds and 4.377 caller
CPU-seconds over the observed task lifetimes. The append interval was 6.610
seconds. These CPU counters are not phase-gated. The harness thread also
used 4.523 CPU-seconds across calibration, validation and reopen. The writer's
CPU use relative to the append interval supports an approximate serial-writer
CPU/coordination limit. Caller CPU is spread across 1,000 threads and does not
saturate the ten virtual CPUs collectively. This does not exclude scheduling
or load-generation overhead from latency. Function-level attribution among
classification, map work, encoding, publication and acknowledgments remains
inconclusive. No profiler proved a specific function to be the bottleneck.

The raw runs report summed group-append and group-flush service times and
summed admission-call time. Compare those with the append interval, not whole
test duration. Roughly 150k records/s is about 38 MB/s of payload and 44 MB/s
of framed storage. About 1,000 submits per group means roughly 150 flushes/s.
The storage service sums occupy a small fraction of append elapsed time, so
these figures do not imply extraordinary disk bandwidth or flush rate.
A roughly 6.6 ms mean request time times 150k records/s implies about 990
outstanding requests, consistent with 1,000 closed-loop callers. The device
microbenchmark cannot alone predict that end-to-end latency.

## Checks, red evidence and remaining review

The original capture's Linux and macOS commands both return exit zero, with
138 passing Rust tests. Parent review added the blocked-diagnostic regression.
The final [Linux checks](slice10/reviewed-linux/tests.log) and
[macOS checks](slice10/reviewed-macos/tests.log) each have 135 passing core tests,
one ignored scheduled long-run test, three process tests and one real-storage
test: 139 passes total. All 135 baseline behaviors remain. Both final runs also
pass the three Python verifier regressions, formatting, strict workspace
Clippy and case-ID checking. The [final checks](slice10/final-checks.tsv) also
record diff validation, source hashes and task-container cleanup. The fuzz configuration compiles with the existing
24 unused simulator warnings. Logs remain under the platform directories.
No hosted scheduled execution is claimed.

The peak probe has an [actual assertion red](slice10/peak-red.log): zero
instead of 414 bytes for a held original and its queued retry.
The [focused green](slice10/peak-green.log) also verifies that the historical
peak survives release and a later smaller request, while caller-visible
records and retry positions remain correct. The first Position comparison
and Options print attempts had compile errors; those are not behavioral reds.
The two actual-storage scenarios are supplemental coverage of existing
behavior, not newly fixed engine defects. Historical missing initial reds
remain missing.

The final [provenance manifest](slice10/provenance.tsv) records source, category,
transformations and hashes. [Source hashes](slice10/source-sha256.tsv) identify
the uncommitted implementation used for the benchmark capture. Capture versions
of sources later changed by review are preserved under
`slice10/capture-source/`; all other capture hashes match unchanged sources.
[Reviewed source hashes](slice10/reviewed-source-sha256.tsv) identify the final
verification code. The measured workload and storage instrumentation did not
change, apart from the inactive timeout constructor and removed ordinary
comments; the [body comparison](slice10/reviewed-measurement-body.log) records
that check. No benchmark rerun is claimed for these verification-only changes.
The fresh Linux and macOS checks and Linux trace were executed after them.
Earlier exploratory
runs, script failures, and discarded diagnostic attempts remain in ignored
`.audit/slice10`; they do not enter this report. The local decision trail is
`.audit/slice10.tsv`.

| Gate item | Submitted evidence | Remaining limitation |
| --- | --- | --- |
| Linux and macOS checks | Final platform logs, exit-status records and independent reviews | Hosted scheduled execution remains unclaimed |
| One Linux data flush per new group | Raw strace, strict phase verifier, two new groups and durable retry | Finite directed workload; no power-loss qualification |
| Both directory syncs on both platforms | Actual Real storage return values; Linux syscall corroboration | macOS has no kernel trace requirement; storage assumptions unqualified |
| Linux core measurement | Five release runs, calibration first, exact charges, every group, reopen, CPU diagnostic | Local NVMe target condition unavailable; limitations above |
| Completion decision | This linked record | Owner closes the gate; no milestone status changed |

Physical Linux NVMe target-condition results and actual-stack power-loss
qualification remain unavailable. The latter is a separate pre-M6 requirement,
not something this syscall trace can establish. Historical evidence gaps and
owner review remain explicit. Parent owns issue #17 updates, independent
review, PR actions and the final gate decision.
