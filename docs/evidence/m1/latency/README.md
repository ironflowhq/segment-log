# Append latency investigation

This investigation supports issue #20. It does not decide the M1 gate in #17.

The portable CRC32C table improves append p50 and warm reopen on this VM.
Five paired runs give median p50 of 16.551 ms before and 15.558 ms after, with nonoverlapping run ranges.
The p99 and throughput ranges overlap. No p99 improvement or target pass is established.
The comparison and its limits are [recorded below](#candidate-selection).


The unchanged baseline source is `911e809e72fc9c9ef5571f7022c89e55a48d6cb1`, the merge of PR #19.
The diagnostic source is `e9f3a4a53fd8afdc89998a848e6745ea16871fff`.
The CRC candidate source is `8277b69cd8f1b5464fa48c82b0ee9eadc6e72532`.
The engine scheduling, defaults, and durable format remain unchanged.

## Measurement boundaries

The benchmark uses 1,000 closed-loop caller threads, 1,000 submits per caller, and one keyed 256-byte record per submit.
Every repetition checks all acknowledged offsets, revisions, stream heads, keys, and payloads before and after reopen.
Failures panic and make the process fail. Zero error fields describe successful runs, not caught and discarded errors.

The append timer starts before opening the callers' gates and ends after joining them and merging their results.
Per-request latency starts before input allocation and ends after `Pending::wait` returns.
Admission time covers `append`, excluding input allocation and the wait.
The sum of admission times overlaps across callers and is not elapsed wall time.

`SEGMENT_LOG_BENCH_STORAGE=real` bypasses `Measured` with the existing `Real` storage implementation.
The normal variant retains `Measured` and its clocks, strings, mutex, and per-group storage metrics.
Both variants still record request latencies, admission timings, and the test-only reservation maximum.
The real-storage variant cannot report group timings or counts. It retains complete record validation and size checks across reopen.
This experiment tests only the storage wrapper's contribution, not all measurement overhead.

`SEGMENT_LOG_BENCH_CPU=1` reads Linux `schedstat` counters in a separate diagnostic run.
Each caller reads its own counters after its start gate and immediately after its final result.
The writer counters bracket gate release and all caller joins, before validation, Drop, and reopen.
The writer is alive at both observations, so no final sampling interval disappears on thread exit.
Counters include small boundary-reading costs. Runnable time requires `kernel.sched_schedstats=1`.
Blocked channel and storage waits are not runnable time.

The function profile uses a separate optimized build with debug information and `perf` CPU-clock sampling.
The whole-test recording includes validation and recovery, so use the writer-filtered report for append attribution.
Whole-process percentages would conflate checksum validation with append encoding.

## Reproduction

The exact disposable-host setup and unchanged five-run sequence are in [baseline.sh](baseline.sh).
It installs packages only on that disposable Ubuntu host, not on the developer's Mac.
Source arrived through `git archive` and then a Git bundle without credentials.
The parent supplied the VM and owns its teardown.

After checking out the diagnostic source, run the interleaved comparison on Linux with the pinned Rust toolchain:

```sh
sudo sysctl -w kernel.sched_schedstats=1
SEGMENT_LOG_EVIDENCE_DIR=/root/latency/data scripts/m1-latency-linux.sh /root/latency/ablation
```

[profile.sh](profile.sh) records functions separately. Neither profiled run enters the five-repetition comparison.
The output directories must be fresh so a failed rerun cannot leave old successful logs mixed with new output.

## Environment limits

The benchmark host is a Hetzner CCX23 VM in hel1 with four dedicated virtual CPUs, 16 GB RAM, and a 160 GB virtual local disk.
The filesystem, kernel, CPU model, toolchain, and load observations are preserved in the environment output.
Dedicated virtual CPUs do not establish exclusive physical cache, memory bandwidth, or storage service.
The physical backing device, volatile-cache policy, and end-to-end power-loss flush behavior are unverified.
No physical NVMe qualification is claimed.

An earlier Docker Desktop attempt on an Apple M4 was interrupted during its first full repetition.
The Mac had 200 unrelated `yes` processes and host load above 170 on 10 cores.
Its tests and syscall checks passed, but its incomplete benchmark is excluded from timing comparisons.

## Baseline and rejected wrapper alternative

The [unchanged baseline](baseline-summary.md) validates five million records across five runs.
Every run misses the numerical relative latency bound. The VM does not meet the physical target conditions.
The slower first run remains in the report.

The [wrapper comparison](ablation-summary.md) alternates measured and real storage five times with the same diagnostic binary.
The throughput ranges overlap. Bypassing the wrapper gives no measurable improvement in this sample.
The p99 ranges also overlap, so their median difference does not support a latency claim.
Keep the existing wrapper and its useful group observations.

## Writer CPU is a measured contributor

The [separate phase diagnostic](ablation/cpu.log) measures 13.815 writer CPU-seconds in a 17.145-second append interval.
The writer consumes about 81% of one CPU during that interval.
The sum of group flush service times is 3.363 seconds, about 20% of the interval.
Service time includes some CPU time, so these shares are not disjoint and must not be added.
The writer's summed runnable delay is 0.136 seconds.
This run supports writer CPU as a limiter rather than host CPU starvation alone.

Callers consume 12.168 CPU-seconds and accumulate 21.025 runnable seconds across 1,000 threads.
Those sums overlap. They are neither wall-clock delay nor a per-request latency quantile.
The 0.878-second sum of admission calls is less than one microsecond per accepted request on average.
This does not bound admission p99 or its impact during an individual scheduling stall.

The [writer-filtered profile](profile/writer.txt) attributes 24.45% of writer self samples to `frame_crc`.
The call graph resolves CRC32C inside that function. The current implementation processes eight bits per input byte.
Other observed costs include key-map insertion, allocation, notification, and futex wakeups.
Some addresses remain unresolved. Sampling percentages are approximate.
The [recording](profile/perf.log) contains 1,922 samples and reports no lost samples in the report.
The recording covers the whole test. The writer filter excludes validation and reopen on the acceptance-test thread.

## Queue and acknowledgment limits

Admission, queue residence, classification, grouping, encoding, append, flush, publication, and result delivery all precede the observed acknowledgment.
The workload keeps up to 1,000 requests outstanding. Each caller submits its next record only after its previous acknowledgment.
The test timer also includes input creation and caller scheduling after the reply becomes available.

Neither the storage wrapper nor `schedstat` timestamps individual queue entries or replies.
The remaining latency cannot be divided into exact queue, grouping, publication, and acknowledgment percentiles from these artifacts.
The function profile shows notification work but does not measure sleep time in `Pending::wait`.
A caller-count experiment would also change the stream distribution in this workload, so it was not treated as a clean scheduling ablation.
No complete attribution of the p99 miss is claimed.

## CRC candidate and compatibility checks

A constant 256-entry table replaces eight bit steps with one byte lookup.
The reflected polynomial remains `0x82f63b78`. Initialization and final inversion remain all-one and bitwise complement.
The table occupies 1 KiB and uses only portable safe Rust and `std`.
The candidate retains the frame checksum copy so that only one suspected cost changes.
Hardware intrinsics would require platform dispatch and unsafe code. They were not selected.

`scripts/m1-compare-codec.sh` compiles the old source and current source into separate fixture executables.
It compares 451,781 output bytes containing all two-byte checksums, empty and variable payload checksums, keyed and unkeyed frames, and headers.
Both outputs have SHA256 `243f2d693417c052d3bc42798211d22740f37c00075ef5e3a06d738ccaefe842`.
The fixture also covers a 1,024-byte stream, empty records, variable payloads up to 65,537 bytes, and extreme epochs.
This comparison does not depend on the new decoder agreeing with the new encoder.

The acceptance tests compare all 65,536 two-byte inputs and variable frames with the existing independent bitwise fixture.
A new simulation case crashes at each storage boundary for a submit containing empty and variable binary payloads across sector and page boundaries.
It checks all-or-none recovery and retention after a completed flush under Kill, PowerLoss, and PowerLossTorn.
Both new tests pass on the old implementation as expected. They are compatibility checks, not a claimed initial behavioral failure.
The observed failure motivating the candidate is the measured latency-bound miss.

## Commands and checks

The baseline uses `baseline.sh` from this directory on the unchanged source.
The storage experiment uses `scripts/m1-latency-linux.sh` at the diagnostic SHA.
The CRC comparison uses `scripts/m1-compare-latency.sh BEFORE_BINARY AFTER_BINARY OUTPUT_DIRECTORY`.
Both binaries use release optimization and the same diagnostic workload. Their SHA256 digests are recorded with the outputs.
Every comparison runs before then after, five times, following a short warmup for each side.
The separate CPU run follows the repetitions. No profile or other benchmark runs concurrently on the VM.

Generate the reports with:

```sh
python3 scripts/m1-latency-summary.py docs/evidence/m1/latency/baseline run
python3 scripts/m1-latency-summary.py docs/evidence/m1/latency/ablation measured real
python3 scripts/m1-latency-summary.py docs/evidence/m1/latency/comparison before after
scripts/m1-compare-codec.sh .audit/crc-equivalence
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 -B -m unittest discover -s scripts -p 'test_m1_*.py'
SEGMENT_LOG_TRACE_SEEDS=100 SEGMENT_LOG_TRACE_SEED=1000 cargo test -p segment-log-core --lib acceptance::seeded::threaded_seeded_traces_long_run -- --ignored --exact --nocapture
```

The macOS checks at the candidate SHA pass 141 Rust tests, with one scheduled test ignored, and three Python verifier tests.
The separately invoked long run passes 100 seeds and 2,700 scenarios.
The compatibility and crash test initial passes are saved with the candidate checks.
These finite samples do not prove that all failures or crashes are covered.

Review of the implementation and decision trail remains assigned to the parent task.
No child reviewers were spawned because this assignment forbids subagents.

## Candidate selection

Keep the CRC byte table for the supported p50 and warm-reopen improvement.
The [five paired repetitions](comparison-summary.md) retain all runs, including the slow first candidate run.
Append p50 has median 16.551 ms before and 15.558 ms after, a 6.0% reduction.
Before ranges from 16.372 to 16.785 ms. After ranges from 15.383 to 16.008 ms.
The median difference exceeds each side's run-to-run spread.
Warm-cache reopen also has separated ranges, with median 2048.094 ms before and 1365.791 ms after.
That improvement applies to this fully retained million-record fixture, not cold-cache or later-milestone open targets.

Append p99 ranges overlap, as do throughput ranges. Those improvements remain inconclusive despite better medians.
Every candidate repetition still misses its numerical relative latency bound.
Keep the observed default configuration and report the miss rather than changing the target.
The storage-wrapper ablation remains rejected. No other writer optimization was attempted.

The [candidate function profile](profile-after/writer.txt) reduces `frame_crc` self sample share from 24.45% to 15.09% of writer samples.
The separate phase diagnostics measure writer CPU of 13.815 seconds before and 12.700 seconds after.
Their append intervals are 17.145 and 16.005 seconds. Each is one diagnostic run, not a repeated speedup estimate.
Together these observations support the CRC mechanism, but cannot explain all remaining tail latency.

The candidate Linux checks pass 141 Rust tests, with one scheduled test ignored, plus three Python verifier tests.
The syscall verifier confirms one data flush per new group, none for the durable retry, and successful creation and recovery directory syncs.
The first Linux workspace run [fails the coverage scanner](checks-linux/tests-transfer-failure.log) on non-UTF-8 AppleDouble files.
My Mac tar transfer created `._evidence.rs` and `._cpu.rs`. Removing only those two files makes the same full workspace command pass.
No coverage scanner or runtime change masks that failure. Git bundles avoid the transfer metadata on reproduction.

## Evidence and remaining gaps

Committed output copies preserve all full repetitions, phase counters, profile summaries, environment observations, and check outcomes.
Only trailing whitespace and terminal blank lines were removed. [output-hashes.tsv](output-hashes.tsv) records original and committed SHA256 values.
The original output copies remain under `.audit/latency`.
The large raw `perf.data` recordings and build outputs stay in ignored `.audit/latency/remote` in this worktree.
The scripts regenerate them. The report does not imply that these large local artifacts are in Git.
The VM was released to the parent task after all runs completed, with no benchmark, compiler, or profiler process left running.

The physical-storage qualification gap remains. Exact per-request queue and acknowledgment distributions remain unmeasured.
Caller thread count, other payload sizes, and other CPU architectures may change the performance result.
This experiment does not prove that CRC is the root cause of the p99 miss or that the candidate fixes it.
The original evidence archive's missing initial-red outputs remain missing. These later compatibility passes do not replace them.
