# The six issue-20 changes together: append evidence

[Issue 20](https://github.com/ironflowhq/segment-log/issues/20) listed no proven throughput or p99 improvement. Six changes then landed, each with its own record: the [ack relay](../ack-relay/README.md), the [CRC by eight bytes](../crc-slicing/README.md), [stream heads](../stream-heads/README.md), [key positions](../key-positions/README.md), [positioned reads](../positioned-reads/README.md) and the [paced benchmark](../paced-benchmark/README.md). This record measures them together against the `main` before them.

## Result

**Closed loop.** In five alternating pairs of the default benchmark, the median throughput rose from 156,239 to 414,634 records/s, the median p50 fell from 6.024 ms to 2.118 ms and the median p99 fell from 9.915 ms to 4.275 ms. All three were better in every pair and their ranges are separated. The writer's group cycle fell from 5.9–6.7 ms to 1.97 ms.

**Paced at 50,000 records/s.** The median p50 fell from 1.890 ms to 1.210 ms and the median p99 from 3.871 ms to 2.090 ms. Both were better in every pair with separated ranges.

**The p99 bound is still missed in every run.** In the closed loop the p99 is about twice the bound. In the paced run it is 0.13–0.25 ms above the bound in four runs and 1.07 ms above it in one.

This is evidence from a virtual Linux guest on a Mac. It does not show that a PRD target passes or fails on target hardware.

## Scope and provenance

- The earlier `main` is `5e95407cb3e427800cdb29c687595d35bae56410`. The measured build is a local merge of the six code commits in [sources.txt](sources.txt). Its `segment-log-core` tree is `5157c788a84c8f781e31809e230e06148d36f78f`, the same tree as `main` at `e712813cf8308c49f4f999e55311ed4493f0e5cf`, after the six changes were squash-merged as #28 to #33. [binaries.sha256](binaries.sha256) identifies the binaries.
- The paced runs compare the earlier `main` with only the paced benchmark added (`18e3581a812a490505ac4aaf9cdb9b14c9b7165c`) against the merged build.
- Captured on 9 November 2026 in the pinned `rust:1.99.1-bookworm` image (`sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`) on Docker Desktop for Mac. The guest runs Linux 7.0.14-linuxkit on aarch64 with 10 processors and 8 GiB. Data was on the container's overlay filesystem, on the guest's virtual disk. [environment.txt](environment.txt), [docker-info.txt](docker-info.txt) and [host.txt](host.txt) hold the details.
- The host is an Apple M4 with 10 cores and 24 GiB on macOS 27.0.1. Unrelated work ran on the host and in other containers of the same guest. [gated.sh](gated.sh) waited before each run until the other containers used less than 10% of a processor, and recorded their use in the `others` files. They used less than 7% before every run of this record. The sample after a run was higher three times in the closed-loop batch: 44% and 28% after the third and fourth runs of the earlier `main`, and 9% after the fourth run of the merged build. The script cannot see work on the host outside the guest. Nothing outside this experiment was stopped.
- This is not physical local NVMe and it is not the deployment storage stack. The guest has 10 processors for 1,000 caller threads.

## Method

All runs use the default options, 1,000 callers, 1,000 rounds, 256-byte payloads and one keyed record for each submit, with the procedure of `scripts/m1-compare-latency.sh` through [gated.sh](gated.sh): one warm-up for each binary, then five alternating pairs.

- **Closed loop.** The default benchmark. `scripts/m1-latency-summary.py` wrote [latency/summary.md](latency/summary.md).
- **Paced.** `SEGMENT_LOG_BENCH_INTERVAL_US=20000`: each caller submits once in 20 ms, so the log is offered 50,000 records/s, and latency runs from the scheduled send. The paced benchmark's `summarize.py` wrote [paced/summary.md](paced/summary.md).
- **Trace.** The request trace with 1,000 callers and 200 rounds, three alternating repetitions for each binary, split by `scripts/m1-group-phases.py --min-count 900`. The phase tables are in [trace/](trace/).

Every run validated all its records before and after reopen, with no refusal and no commit or correctness error.

## Closed loop

| | Earlier `main` median (range) | Six changes median (range) |
| --- | --- | --- |
| Records/s | 156,239 (88,153–164,994) | 414,634 (361,408–423,812) |
| p50 ms | 6.024 (5.912–8.396) | 2.118 (2.104–2.212) |
| p99 ms | 9.915 (8.675–37.545) | 4.275 (4.100–7.890) |
| p999 ms | 13.517 (11.118–62.824) | 5.584 (4.656–42.629) |
| Bound for the run ms | 2.118 (1.815–2.277) | 1.981 (1.884–2.223) |
| Groups | 1,047 (1,014–1,835) | 1,253 (1,209–1,355) |
| Reopen ms | 1,063.1 (910.2–1,299.2) | 504.8 (476.6–580.0) |

- The first two runs of the earlier `main` were slow (88,153 and 130,233 records/s) although the other containers were idle, and the first run of the merged build had a p99 of 7.890 ms. They are kept in every range.
- The bound is the linger plus twice the device `fdatasync` p99 that the same run measured. All ten runs miss it.
- In a closed loop the mean latency is the number of callers divided by the throughput. The five runs of the merged build predict 2.360–2.767 ms and measured 2.302–2.692 ms. That mean is already above the bound of about 2 ms. See [ADR 0045](../../../decisions/0045-latency-target-is-measured-at-a-paced-rate.md).

## Where the group cycle goes

Median over the counted groups of each repetition, in milliseconds.

| Phase of one group | Earlier `main`, three repetitions | Six changes, three repetitions |
| --- | --- | --- |
| Formation, with the 1 ms linger | 1.208, 1.145, 1.158 | 1.001, 1.001, 1.001 |
| Encode and CRC | 0.641, 0.659, 0.660 | 0.185, 0.192, 0.183 |
| Write | 0.051, 0.049, 0.049 | 0.040, 0.042, 0.040 |
| `fdatasync` | 0.422, 0.422, 0.421 | 0.387, 0.385, 0.377 |
| Publish | 0.311, 0.338, 0.330 | 0.097, 0.097, 0.099 |
| Delivery by the writer | 3.236, 4.011, 3.977 | 0.251, 0.242, 0.245 |
| **Cycle** | **5.926, 6.678, 6.649** | **1.971, 1.978, 1.963** |

With the six changes, the 1 ms linger is half of the cycle and the `fdatasync` a fifth. The processor work that is left in the writer is about 0.5 ms for a group of 1,000.

## Paced at 50,000 records/s

| Run | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Bound ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Earlier `main`, 1 | 17.095 | 1.974 | 312.949 | 380.532 | 443.418 | 2.168 |
| Earlier `main`, 2 | 2.049 | 2.001 | 3.871 | 7.337 | 16.497 | 2.415 |
| Earlier `main`, 3 | 1.858 | 1.837 | 3.106 | 5.380 | 13.659 | 2.006 |
| Earlier `main`, 4 | 32.588 | 1.890 | 414.700 | 444.348 | 491.661 | 1.950 |
| Earlier `main`, 5 | 1.785 | 1.735 | 3.174 | 13.131 | 26.236 | 2.155 |
| Six changes, 1 | 1.203 | 1.196 | 2.066 | 5.703 | 9.635 | 1.925 |
| Six changes, 2 | 1.250 | 1.236 | 2.285 | 6.868 | 14.270 | 2.037 |
| Six changes, 3 | 1.217 | 1.210 | 2.090 | 4.770 | 10.502 | 1.957 |
| Six changes, 4 | 1.316 | 1.288 | 2.958 | 5.550 | 8.900 | 1.884 |
| Six changes, 5 | 1.195 | 1.188 | 2.045 | 5.475 | 8.475 | 1.909 |

- Every run kept up with the offered rate.
- Two runs of the earlier `main` lost more than a second to slow device flushes and have a p99 above 300 ms. The [paced benchmark record](../paced-benchmark/README.md) describes them. No run of the merged build had such a stall in this batch, so the p99 ranges compare a stalled and an unstalled sample. Without those two runs the earlier `main` has a p99 of 3.106–3.871 ms.
- The merged build misses the bound by 0.141, 0.248, 0.133, 1.074 and 0.136 ms.

## Limits

- One guest with 10 processors and a virtual disk that stalls, one session, five pairs, three trace repetitions.
- One offered rate. The paced variant has one outstanding request for each caller, so it is a paced closed loop and not a true open loop.
- No run above 1,000 callers, no history above 1,000,000 records and no physical NVMe.

## Checks

The merged tree passed `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the long run with 100 trace seeds from 10000 (2,700 scenarios) and 30 process-kill repetitions on the Mac host. Each of the six changes passed the same checks alone on the Mac host and in the Linux container; their records hold the result lines.

## Reproduce

Build `5e95407` and `main` at `e712813` in the pinned image with `cargo test -p segment-log-core --lib --release --no-run`. Keep the data directory on the container's own filesystem.

```sh
SEGMENT_LOG_EVIDENCE_DIR=/data/bench scripts/m1-compare-latency.sh BASE_TEST_BINARY MAIN_TEST_BINARY latency
python3 scripts/m1-latency-summary.py latency before after
SEGMENT_LOG_EVIDENCE_DIR=/data/bench SEGMENT_LOG_BENCH_INTERVAL_US=20000 \
  scripts/m1-compare-latency.sh PACED_BASE_TEST_BINARY MAIN_TEST_BINARY paced
python3 docs/evidence/issue20/paced-benchmark/paced/summarize.py paced before after
```
