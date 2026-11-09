# Mac request pipeline diagnostic

The user wants evidence for the remaining p99 attribution gap in issue #20 before choosing an optimization.
This spike adds test-only request timelines and a bounded Mac observation.
It does not complete the 1,000-caller attribution task, establish a Linux target result, or propose a production change.

## Result and uncertainty

Observer overhead is inconclusive.
Five alternating unchanged-control and traced repetitions have overlapping p99 and throughput ranges.
The traced p999 median is higher, and the difference cannot be separated from run variation.
We stopped escalation to million-record traces after these observations.
The parent explicitly accepted that boundary and released the interval to the memory investigation.
No 1,000-caller baseline or profiler run was attempted in this spike.

Within the five traced 64-caller runs, remaining group formation plus flush accounts for 93.93–96.03% of summed latency among the slowest 1% of requests in each run.
That is a measured partition of those requests, not a sum of stage p99s.
Formation alone varies from 30.22% to 81.43%, and flush from 14.48% to 65.81%.
These large variations rule out a single stable division between those two contributors in this sample.
Queue plus channel-send time accounts for 0.29–1.74%, and pre-send to caller-wakeup time accounts for 0.095–1.439%.
The latter includes reservation release, the send operation, and caller scheduling. It is an upper bound on scheduling after notification, not an isolated scheduling measurement.

The intervals are wall time. They include descheduling and storage-service delays.
No writer CPU, caller CPU, runnable-delay, physical flush latency, or storage bottleneck claim follows from them.
The whole-process `time -l` counters include setup, validation, trace output, and recovery and cannot separate those roles.
The trace identifies where this small workload waits, but it does not establish why that interval is that long.
A lower linger, different caller arrangement, or faster checksum remains an untested alternative, not a justified production fix.

## Workload and populations

All options use `Options::default()` and actual `Real` storage.
The control binary is built from unchanged `e409b1d88bdbbe7dbbb36ae7b08c54861e744a2a`.
The traced binary runs the same release acceptance workload with the test-only observations enabled.
The comparison uses an entirely uninstrumented source control, not merely disabled tracing in the candidate.
Both retain the existing outer request timer, admission timer, and reservation-peak instrumentation.
Thus the comparison estimates the incremental pipeline observer, not the effect of all benchmark measurement.

Each repetition starts a fresh directory and 64 closed-loop caller threads.
Each caller writes 100 keyed 256-byte records to its own stream, waiting for every result.
There are 6,400 accepted requests per run and 32,000 per side across five repetitions.
The caller gates, literal payloads, stream distribution, and default configuration are identical on both sides.
The whole append timer includes gate release, caller work, joins, and result-vector merging.
The per-request timer includes input creation, append, and result waiting.
No concurrent read workload runs during append. Record validation and recovery are outside the append timer.

Each traced run sorts its own 6,400 request durations and selects the largest 64 for its tail population.
Each stage fraction divides summed time in that stage by total latency of those same 64 requests.
We report the range across five such populations, rather than pool requests or treat correlated requests in one group as independent trials.
Quantiles use nearest rank, `ceil(N * q) - 1`, on the sorted sample.
`p99_request` contains the single request at that rank and its disjoint intervals.
The outer benchmark latency starts slightly earlier and ends slightly later than the trace's `start` and `wake` stamps.
The comparison below uses the unchanged outer benchmark metric on both sides.

| Metric | Control median | Control range | Trace median | Trace range |
| --- | ---: | ---: | ---: | ---: |
| p50, ms | 2.770 | 2.443–3.067 | 2.758 | 2.610–2.804 |
| p99, ms | 3.376 | 2.932–4.551 | 3.270 | 2.971–4.900 |
| p999, ms | 4.559 | 4.229–4.946 | 4.919 | 4.614–5.996 |
| Records/s | 22,869 | 21,013–25,291 | 23,266 | 22,638–24,078 |

There are five runs per side. No speedup, regression, or zero-overhead conclusion is established.
The load generator is unchanged across the pairs, so its incremental scheduling response to tracing is included in those ranges.
The experiment does not independently vary caller count while preserving stream distribution.
It cannot isolate load-generator overhead from engine cost.

The earlier 8×8 and 64×100 traced pilots remain separate from these ten repetitions.
All 12 runs passed. All 70,464 records were checked before and after reopen for count, offsets, revisions, keys, payloads, and stream heads.
All runs reported zero Full refusals, other admission errors, commit errors, and correctness errors.
All seven traced runs passed the request/group join and monotonic timeline checks.
A fatal error fails the test process and remains in its log; error fields are not counters that discard failed attempts.
The directory fixture deletes its data after each successful test, so retained raw artifacts are observations, not retained log files.

## Timestamp boundaries

The test-only module allocates fixed atomic slots before the timed append interval.
It indexes them by the fixture's caller/revision key, avoiding a per-event global lock and preserving `Request`, `Pending`, and admission-charge sizes.
A shared monotonic `Instant` origin timestamps all threads.
Group rows use the group's base offset, and each caller records its returned offset.
The summary checks unique request identities and offsets, complete caller/revision coverage, group membership and counts, positive clocks, timestamp ordering, and exact duration conservation.
Six verifier tests include known-bad missing clocks, reversed clocks, duplicated requests, wrong group counts, and swapped committed-offset identities.

The disjoint intervals follow this sequence.

| Interval | Boundaries and limitations |
| --- | --- |
| Input and retries | Caller start to the final append entry. Includes input allocation and any previous refusal attempts. No refusals occurred here. |
| Admission before send | Append entry to immediately before `try_send`. Excludes the channel operation. |
| Queue and send | Before `try_send` to first writer receipt. Includes send cost and queue residence. Exact successful insertion is inside the channel implementation and is not observed. |
| Writer before classify | First receipt to final classification start. Includes the first request's published-state snapshot. Deferred requests would include deferred waiting here. |
| Classification | Final classification start to successful staging, revision assignment, and head updates. |
| Remaining formation | Request staging to group formation end. Includes subsequent classifications and channel waits, and any writer descheduling. |
| Encode | Formation end to completed `codec::encode`, including entry-vector construction. |
| Write | Encoded frame to successful append return. |
| Flush | Append return to data-sync return, including the storage dispatch and scheduling. |
| Publication | Flush return through key and published-state updates. |
| Delivery prefix | Publication to this request's pre-send stamp. Includes delivery work for earlier requests in the group. |
| Send to wake | Pre-send stamp through reservation release and notification to caller return from `Pending::wait`. |

The separate `admitted` stamp follows `append` return.
It can overlap writer work, so the verifier checks its bounds but does not add it to the disjoint sequence.
The group `delivered` stamp follows all new-request sends and also is not added to a request's duration.
Trace output happens after joining the writer, outside append timing, so the last group's row cannot race the dump.
The probe supports this one-new-keyed-record workload, not arbitrary retries, rejection workloads, or a public tracing API.

Ten thousand adjacent-clock samples precede each traced run.
The paired runs report clock-sample p50 of 41–42 ns and p99 of 42–83 ns.
One run has a 77,792 ns maximum, retained in `1-trace/run.log`.
Some samples are zero because consecutive observations land on the same clock tick.
This clock microcheck does not include atomic stores, key decoding, cache traffic, or altered scheduling.
Only the full paired workload can assess those combined effects, and that result remains inconclusive.

## Host and qualification limits

The host is an Apple M4 Mac16,10 with 10 logical cores and 24 GiB RAM, on macOS 27.0.1.
The pinned toolchain is Rust/Cargo 1.99.1, with optimized release tests and two build jobs.
The evidence directory is on `/Volumes/p7`, an APFS volume. This is Mac development evidence only.
`diskutil` observations do not qualify the underlying storage or its volatile-cache and power-loss behavior.
No Linux, physical NVMe, or target-pass claim is made.

Preparation observed an unrelated Go linter at about 819% CPU, so compilation and benchmarks waited.
It was not stopped or suspended. No unrelated process or system setting was changed.
The parent then granted an exclusive interval for our measurements.
Before/after each paired run, `top` recorded two samples one second apart.
The second sample showed 80.66–95.20% CPU idle across the observations.
Load averages still reflected earlier contention. Each run retains load, process CPU/RSS, VM counters, and filesystem observations.
These boundary snapshots do not prove an absence of contention during every request.
The two initial pilots preceded the addition of interval-based `top` samples and retain `ps` and load observations instead.

`pmset` reported no recorded thermal or performance warnings and no CPU power status.
This is not a temperature measurement or proof of no throttling.
Every run used fresh files, without cache eviction. Toolchain and OS caches could be warm.
Reopen is in the same process after validation and is explicitly a warm-cache observation.
The runner records process RSS, not requested allocator bytes or isolated engine memory.

`sample` was available, but the subsecond append intervals made sampling unattractive for this bounded spike.
`xcrun --find xctrace` failed because the utility was unavailable; the failure is retained in `environment.txt`.
No Linux `/proc`, `perf`, sudo, system-wide change, or hosted workflow was used.

## Reproduce and audit

Build the current test binary with `cargo test --release -p segment-log-core --lib --no-run -j2`.
Build the control from a clean archive of the base SHA above with the same command and toolchain.
Cargo prints each test-binary path.
The runner refuses an existing output directory and applies a 240-second process-group timeout.
Run only during an exclusive, suitably idle host interval.

```sh
python3 scripts/m1-pipeline-run.py CONTROL_BINARY OUT/1-control --callers 64 --rounds 100
python3 scripts/m1-pipeline-run.py TRACE_BINARY OUT/1-trace --callers 64 --rounds 100 --trace
```

Repeat the same control-then-trace order for repetitions 2 through 5.
Run `scripts/m1-pipeline-summary.py OUT` to regenerate the comparison.
Run `scripts/m1-pipeline-summary.py OUT/1-trace/requests.tsv` to regenerate one joined attribution.
Run the runner at 8 callers and 8 rounds first for its clock/order pilot.

[raw.tar.gz](raw.tar.gz) preserves every pilot and pair log, raw request/group TSV, per-run manifest and host observation, build output, and validation output.
[raw-hashes.tsv](raw-hashes.tsv) hashes every archived file.
[build-manifest.json](build-manifest.json) pins the base SHA, source-file hashes, release profile, and binary hashes.
[comparison.json](comparison.json) is generated from all ten pair logs.
Full original copies and both executables also remain in ignored `.audit/p99` in the producing worktree.
No failed run or clock outlier was removed. No controlled benchmark run failed.

`cargo fmt --all --check` and workspace clippy with warnings denied passed.
Workspace tests passed 142 tests, with one scheduled test ignored. Nine Python verifier tests passed.
The six new Python tests validate trace handling; the existing Rust acceptance workload exercises the clocks and validates persisted records.
No production behavior change or new dependency is present.
All heavy processes exited before interval release. Subsequent work is documentation and PR preparation only.

The Explain the Number principle led to the alternating unchanged-source control and the decision to stop escalation.
Model the Domain led to request slots joined to group timelines rather than independent stage histograms.
Build the Lever led to the committed runner and verifier with known-bad controls.
The ignored decision trail records the same choices. Independent code and trail review remains with the parent, as this assignment forbids child delegates.
