# Paced benchmark: evidence for issue 20

[Issue 20](https://github.com/ironflowhq/segment-log/issues/20) reports that the append p99 misses its bound. The default benchmark cannot show whether the miss comes from the log or from the workload, because it is a closed loop. This record measures a paced variant of the benchmark on `main`. It changes no engine code.

## Result

**A closed loop ties latency to throughput.** In the default benchmark each of the 1,000 callers submits again as soon as it is answered. The mean latency is then 1,000 divided by the records per second. The five gated default runs of `main` at 165,235–168,230 records/s predict 5.944–6.052 ms and measured 5.931–6.038 ms. A mean of 2 ms would need 500,000 records/s, ten times the PRD's throughput target.

**At the PRD's throughput target, `main` is close to the bound when the guest is quiet.** With 1,000 callers that each submit once in 20 ms, the log is offered 50,000 records/s and keeps up. The p50 was 1.595–2.001 ms in all ten runs. The p99 was 2.627–3.871 ms in the four runs without a stall, against a bound of 1.955–2.415 ms in those runs. All ten runs miss the bound.

**The paced p99 is fragile on this guest.** Six of the ten runs had a p99 between 27 ms and 606 ms. In the first batch, other containers were busy. In the gated batch they were idle, and two runs still lost more than a second to slow device flushes.

**The default run is unchanged.** With the variable unset, the benchmark binary of this change and of `main` are within 2% in a gated batch.

This supports adding the paced variant as a separate, named workload. It does not change a target, and the PRD does not yet say which load model its latency target assumes.

## Scope and provenance

- `main` is `5e95407cb3e427800cdb29c687595d35bae56410`. The change is `18e3581a812a490505ac4aaf9cdb9b14c9b7165c`. It touches the benchmark test and testing.md. Commits after it on this branch change documents only. [sources.txt](sources.txt), [binaries.sha256](binaries.sha256) and [paced/binary.sha256](paced/binary.sha256) identify what ran.
- Captured on 9 November 2026 in the pinned `rust:1.99.1-bookworm` image (`sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`) on Docker Desktop for Mac. The guest runs Linux 7.0.14-linuxkit on aarch64 with 10 processors and 8 GiB. Data was on the container's overlay filesystem, on the guest's virtual disk. [environment.txt](environment.txt), [docker-info.txt](docker-info.txt) and [host.txt](host.txt) hold the details.
- The host is an Apple M4 with 10 cores and 24 GiB on macOS 27.0.1. Unrelated work ran on the host and in other containers of the same guest. For the gated batches, [gated.sh](gated.sh) waited before each run until the other containers used less than 10% of a processor, and recorded their use in the `others` files. It cannot see work on the host outside the guest. Nothing outside this experiment was stopped.
- This is a virtual Linux guest on a Mac. It is not physical local NVMe and it is not the deployment storage stack.

## Method

`SEGMENT_LOG_BENCH_INTERVAL_US=20000` selects the paced variant. All other settings are the benchmark's defaults for the target comparison: 1,000 callers, 1,000 rounds, 256-byte payloads, default options.

- Each caller has a schedule: one send every 20 ms, with the callers spread evenly over the interval. The offered rate is 50,000 records/s.
- A caller sleeps until its next scheduled send. If its previous answer came late, it sends at once.
- Latency runs from the scheduled send to the answer. A caller that is late for a send still counts the delay, so a stall is not hidden by the callers that could not send during it.
- A caller has one request outstanding at a time. This is a paced closed loop, not a true open loop: under a long stall the offered rate falls.

Ten paced runs of the change's benchmark binary ran in two batches of five. Each alternated with a run of another binary that is not part of this record. The first batch used the procedure of `scripts/m1-compare-latency.sh`, the second used [gated.sh](gated.sh). [paced/summarize.py](paced/summarize.py) wrote [paced/summary.md](paced/summary.md). Every run validated all 1,000,000 records before and after reopen, with no refusal and no commit or correctness error.

The default run was compared between `main` and the change with the variable unset: five alternating pairs in each of two batches.

## Paced runs on `main`

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Bound ms | Mean flush µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| first-1 | 49,993 | 2.461 | 1.792 | 27.396 | 47.148 | 69.216 | 1.772 | 387 |
| first-2 | 49,993 | 2.938 | 1.856 | 34.619 | 49.920 | 67.598 | 1.893 | 412 |
| first-3 | 49,994 | 3.015 | 1.841 | 35.243 | 62.733 | 76.559 | 1.783 | 416 |
| first-4 | 48,329 | 37.411 | 1.944 | 606.060 | 681.340 | 732.629 | 1.755 | 537 |
| first-5 | 49,994 | 1.615 | 1.595 | 2.627 | 7.246 | 16.807 | 1.955 | 281 |
| gated-1 | 49,991 | 17.095 | 1.974 | 312.949 | 380.532 | 443.418 | 2.168 | 516 |
| gated-2 | 49,980 | 2.049 | 2.001 | 3.871 | 7.337 | 16.497 | 2.415 | 403 |
| gated-3 | 49,984 | 1.858 | 1.837 | 3.106 | 5.380 | 13.659 | 2.006 | 326 |
| gated-4 | 49,994 | 32.588 | 1.890 | 414.700 | 444.348 | 491.661 | 1.950 | 491 |
| gated-5 | 49,993 | 1.785 | 1.735 | 3.174 | 13.131 | 26.236 | 2.155 | 351 |

- The bound is the linger plus twice the device `fdatasync` p99 that the same run measured before its appends.
- The p50 is stable: 1.595–2.001 ms in every run. A paced request waits for the rest of the 1 ms linger and for one flush, in a group of about 100.
- The p99 is not stable. In the first batch the other containers were bursting. In the gated batch they used less than 5% of a processor around every run, and runs 1 and 4 still had requests that waited up to 0.44 s and 0.49 s. Their mean flush time is 0.1 to 0.2 ms above the other runs over about 9,000 flushes, which is more than one second in total. So the lost time was in device flushes.
- One stall of 0.4 s delays every send that falls into it, about 20,000 requests. That is 2% of the run, so it sets the p99.

## Default run, variable unset

| Batch | | `main` median (range) | This change median (range) |
| --- | --- | --- | --- |
| First | Records/s | 156,497 (126,062–167,462) | 141,829 (120,350–166,093) |
| First | p50 ms | 6.025 (5.895–6.719) | 5.946 (5.902–6.151) |
| First | p99 ms | 12.630 (8.347–26.917) | 22.099 (8.435–34.939) |
| Gated | Records/s | 165,648 (165,235–168,230) | 168,595 (168,123–169,084) |
| Gated | p50 ms | 5.915 (5.896–5.992) | 5.876 (5.861–5.879) |
| Gated | p99 ms | 8.482 (6.784–9.313) | 7.027 (6.870–8.079) |

- The first batch was disturbed for both binaries and separates nothing.
- In the gated batch the change was 1.8% faster at the median, and its p50 was lower in every pair, by 0.02 to 0.13 ms. The default path differs only in how a caller reads its start time. The positioned-reads change for the same issue, which also leaves the benchmark's write path alone, shows the same 2% for the second run of a pair. So this difference is the order of the runs, not the change.

[latency/summary.md](latency/summary.md) and [latency-gated/summary.md](latency-gated/summary.md) list every run. No run was removed.

## Limits

- One guest, whose virtual disk stalls. The paced p99 needs a device without such stalls before it can be compared with a target.
- One offered rate, 50,000 records/s. No sweep over rates, and no rate above what `main` can serve.
- The variant paces callers. It does not model arrivals that are independent of answers.
- Which load model the PRD's latency target assumes is an open question for an ADR. This record does not answer it.

## Checks

The checks ran on this branch's sources (`segment-log-core` tree `37f2b6a985d6`), on the Mac host and in the Linux container. All passed on both.

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`: 138 unit and scenario tests, 3 process tests and 2 other integration tests. One test is ignored: the scheduled long run.
- 9 Python script tests.

[checks.txt](checks.txt) holds the result lines. Hosted CI had not run for this branch when this record was written.

## Reproduce

Build the change in the pinned image with `cargo test -p segment-log-core --lib --release --no-run`. Keep the data directory on the container's own filesystem.

```sh
SEGMENT_LOG_EVIDENCE_DIR=/data/bench SEGMENT_LOG_BENCH_CALLERS=1000 SEGMENT_LOG_BENCH_ROUNDS=1000 \
  SEGMENT_LOG_DEVICE_ROUNDS=1000 SEGMENT_LOG_BENCH_INTERVAL_US=20000 \
  TEST_BINARY --exact acceptance::evidence::measured_appends_survive_reopen --nocapture > paced/gated-1.log
python3 docs/evidence/issue20/paced-benchmark/paced/summarize.py paced first gated
SEGMENT_LOG_EVIDENCE_DIR=/data/bench scripts/m1-compare-latency.sh BASE_TEST_BINARY CHANGE_TEST_BINARY latency
python3 scripts/m1-latency-summary.py latency before after
```
