# Mac simultaneous read and append diagnostic protocol

The user wants evidence for sustained simultaneous reads and appends against a growing history after PR22's bounded frame snapshots. This diagnostic changes no production code, options, dependencies, API, or format. It does not qualify storage or complete issue20. The root coordinator granted and released the exclusive interval on November 9, 2026. See the adjacent README for results.

## Claim and decision

The intended claim is limited to this machine and workload: whether adding a single closed-loop reader changes append throughput or latency across four history-growth stages. A repeatable gap larger than run-to-run spread could justify further attribution. No optimization follows without the root's decision. If load drift, observer effects, missing overlap, or an unidentified limiter prevents attribution, the result is inconclusive.

The starting production source is `e409b1d88bdbbe7dbbb36ae7b08c54861e744a2a`. The runner records the actual source commit, dirty diff, exact diagnostic source copies and hashes, binary hash, compiler, OS, filesystem, CPU count, RAM, load, process list, and available thermal observations. Missing or failed observations remain recorded. There is no cache eviction. Each run uses a fresh directory and naturally warm system caches.

## Workload and validation

The bounded matrix uses one append caller, one stream, 256-byte payloads, 32 records per submit, 1,024 seed records and 32,768 additional records. Every submit waits for its commit before the next. There are four equal growth stages. Primary runs use `Options::default()`, including 1 ms linger, for both append-only and concurrent workloads. Keyed runs use unique eight-byte keys for every record. Unkeyed runs omit keys. These are workload comparisons, not competing implementations or a tuning recommendation. The pilot passed at these counts, so the final matrix retained them.

The reader repeatedly snapshots the published watermark and requests one record at the start, one at the sampled end, and up to 32 records at its sequential cursor. The cursor advances only after every returned record matches its expected offset, revision, stream, key, length, and payload bytes. This is bounded sequential catch-up, not a full-prefix read per iteration. The already-verified cursor range is an exact contiguous published prefix. Its length at append completion is retained. Remaining prefix work drains after measurement, followed by reopen and a complete bounded validation pass. Unexpected empty or truncated reads fail validation. All appends must acknowledge the expected positions without deduplication.

The append caller and reader start at a barrier. The reader stops after observing the writer's completion flag. A final reader cycle can straddle the end of the append interval. Reader operation counts therefore include that final cycle. Reader stages describe the watermark at cycle start; up to three reads can cross a growth boundary. They are not exact per-read growth stages. Per-stage cycle gaps include read calls, payload checks, scheduling, and watermark acquisition. They are an observational starvation signal, not a lock-wait measurement or a proof of fairness. Stages with no reader work cannot support an overlap claim.

## Measurement costs and bounds

Append distributions time admission through commit completion. Input construction and acknowledgment validation are outside those per-call timers but inside end-to-end append throughput. Read distributions time the read API call; payload validation and watermark acquisition affect closed-loop rate but are outside per-read timers. Fixed-size histograms retain counts, sums, exact maxima, and bucket endpoints. Quantiles must be reported as bucket intervals. Histograms have sixteen subdivisions per power of two and fixed memory independent of operation count. The unit tests verify bucket containment, count conservation, and rejection of incorrect records before measurement.

The matrix has five repetitions per case, alternating append-only and concurrent order. It includes keyed and unkeyed primary comparisons, unkeyed timing-disabled controls for both workloads, and unkeyed concurrent controls without periodic RSS sampling. Timing-enabled and timing-disabled order also alternates. There are 35 matrix runs, not a payload, history, and concurrency cross-product. Controls do not establish observer effects for keyed histories.

Disabling timing changes the reader's closed-loop request rate and therefore its contention. Compare operation counts and achieved load with wall throughput; do not subtract the difference as pure clock overhead. The separate `calibrate` mode measures input construction and clock-plus-histogram costs without storage. It is a client-cost diagnostic, not a service capacity benchmark. A profiler run, if needed and granted, must remain separate from reported matrix runs.

The external runner samples process RSS and process CPU observations every 250 ms. `ps` CPU percentage is lifetime-smoothed. CPU time is not attributed to a phase without matching phase boundaries. Sample command durations quantify the observer's own wall cost, not its total system perturbation. Progress markers bound which history stage was active at observation; they do not give exact simultaneous record counts. Runs without RSS sampling retain the same process-launch and timeout machinery. The runner's polling itself remains present.

RSS is whole-process resident memory, not requested heap or engine ownership. It includes stacks, allocator retention, fixed observer state and runtime pages. `/usr/bin/time -l` records process-lifetime resource usage, including seed, recovery, and final validation. Its maximum RSS cannot be called the peak during concurrent work. No allocator hook or replacement is used. Library heap ownership is the separate memory workstream.

Each run preserves stdout, stderr, command, exit code, timeout status, before/after host observations, sampled RSS, and its dataset locally. A refusal or commit failure stops appending and prints its category. Read errors and mismatches are counted. Failed processes and panics retain stderr and a nonzero result. The runner stops the matrix on failure instead of silently retrying or discarding it. Its timeout kills only the new process group created for that invocation. Before and after each run, a missing memory-pressure observation or a reported free-percentage label below 30% stops the runner. This is a conservative run guard, not a general memory-health claim. All artifacts stay local until copied into the reviewable evidence set.

## Reproduction

Do not run these commands during another worker's interval. Compilation and execution require an explicit root grant. Use a new output directory.

```sh
cargo test -j 2 -p segment-log-core --example concurrent_probe
cargo build -j 2 --release -p segment-log-core --example concurrent_probe
target/release/examples/concurrent_probe calibrate
python3 scripts/m1-concurrent-probe.py --binary target/release/examples/concurrent_probe --output .audit/concurrent-pilot --repetitions 1 --seed 32 --added 128
python3 scripts/m1-concurrent-probe.py --binary target/release/examples/concurrent_probe --output .audit/concurrent-final
```

Compilation, functional execution, histogram tests, and the matrix completed. Results and check counts are in the adjacent README. Independent root review remains pending.
