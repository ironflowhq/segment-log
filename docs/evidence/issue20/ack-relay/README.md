# Ack relay: 1,000-caller attribution and append evidence for issue 20

[Issue 20](https://github.com/ironflowhq/segment-log/issues/20) lists two open items that this record addresses: the latency attribution at the default 1,000 callers, and the lack of a throughput or p99 improvement. It measures one change, the ack relay of [ADR 0044](../../../decisions/0044-woken-callers-relay-group-acks.md).

## Result

**Attribution.** At 1,000 callers on a Linux guest, the writer on `main` spends 3.23 ms of each 5.87 ms group cycle waking callers one at a time. That is 55% of the cycle. The write and the `fdatasync` together take 0.45 ms.

**Improved throughput and p50.** In five alternating pairs of the default benchmark, the median throughput rose from 167,928 to 323,552 records/s and the median p50 fell from 5.868 ms to 2.908 ms. Both were better in all five pairs and their ranges are separated.

**Lower p99 in four of five pairs, and the target is still missed.** The median p99 fell from 7.073 ms to 5.468 ms, but its ranges overlap because one run of the change was disturbed. Every run of both binaries misses the PRD bound, which was about 2 ms in these runs.

This supports the change. It does not show that a PRD target passes, and it is not evidence for target hardware.

## Scope and provenance

- `main` is `5e95407cb3e427800cdb29c687595d35bae56410`. The change is `aeb542c34d3e135a8aacf942608dba028fab2bbc`. Commits after it on this branch change documents and scripts only. [sources.txt](sources.txt) and [binaries.sha256](binaries.sha256) identify what ran.
- Captured on 9 November 2026 in the pinned `rust:1.99.1-bookworm` image (`sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`) on Docker Desktop for Mac. The guest runs Linux 7.0.14-linuxkit on aarch64 with 10 processors and 8 GiB. Data was on the container's overlay filesystem, on the guest's virtual disk. [environment.txt](environment.txt), [docker-info.txt](docker-info.txt) and [host.txt](host.txt) hold the details.
- The host is an Apple M4 with 10 cores and 24 GiB on macOS 27.0.1. It ran unrelated work. Each run's log is preceded by the guest's load average, which includes the run before it.
- This is a virtual Linux guest on a Mac. It is not physical local NVMe and it is not the deployment storage stack. The guest has 10 processors for 1,000 caller threads.

## Method

- **Benchmark.** `scripts/m1-compare-latency.sh` ran the default benchmark with 1,000 callers and 1,000 rounds: one warm-up for each binary, then five alternating pairs. `scripts/m1-latency-summary.py` wrote [latency/summary.md](latency/summary.md).
- **Trace.** The benchmark's request trace (`SEGMENT_LOG_PIPELINE_OUT`) ran with 1,000 callers and 200 rounds, three alternating repetitions for each binary. `scripts/m1-group-phases.py --min-count 900` split the writer's cycle. It counts only groups of at least 900 requests, so the warm-up and partial groups do not dilute the steady state. The trace files are large and are not retained. The benchmark output and the phase tables are in [trace/](trace/).
- **Wake-up strategies.** [wake/wake.rs](wake/wake.rs) parks 1,000 threads and measures how long each strategy takes to wake them, 200 rounds for each strategy, three repetitions. It uses only `std`.
- **Fan-out.** Four builds of the change that differ only in the relay's fan-out, compared with the default benchmark.

Every benchmark and trace run validated all its records before and after reopen, with no refusal and no commit or correctness error.

## Where the group cycle goes

Medians over the counted groups of each repetition, then the median of the three repetitions, in milliseconds. The three repetitions of each phase differ by at most 0.05 ms. The relay's cycle differs by 0.15 ms.

| Phase of one group | `main` | Ack relay |
| --- | ---: | ---: |
| Formation, with the 1 ms linger | 1.202 | 1.044 |
| Encode and CRC | 0.626 | 0.633 |
| Write | 0.041 | 0.039 |
| `fdatasync` | 0.411 | 0.390 |
| Publish | 0.309 | 0.346 |
| Delivery by the writer | 3.233 | 0.258 |
| **Cycle** | **5.871** | **2.792** |

On `main`, delivery is 55.0% of the cycle. With the relay, the writer records the 1,000 answers and wakes four callers in 0.26 ms.

The same traces split each request's latency. These are means, in milliseconds.

| Interval of one request | `main` | Ack relay |
| --- | ---: | ---: |
| In the accept queue | 2.072 | 0.424 |
| In the forming group | 0.775 | 0.553 |
| Encode, write, `fdatasync`, publish | 1.406 | 1.340 |
| Published, answer not yet recorded | 1.637 | 0.113 |
| Answer recorded, caller not yet running | 0.011 | 0.600 |
| **Total** | **5.898** | **3.023** |

- On `main` a request waits 1.64 ms on average for the writer to reach it in the delivery loop, and 2.07 ms in the queue while the writer delivers the group before.
- With the relay, a caller runs 0.60 ms on average after its answer is recorded, and 1.33 ms at the p99. This is the relay's own latency: the time for the wake-ups to spread through the callers.
- Tracing changes the workload. The traced relay runs formed groups below 900 requests more often than `main` (100 to 154 of about 250 groups counted, against 193 to 199 of about 200).

## Append benchmark

Default options, 1,000 callers, 256-byte payloads, one keyed record for each submit.

| | `main` median (range) | Ack relay median (range) |
| --- | --- | --- |
| Records/s | 167,928 (164,045–169,750) | 323,552 (190,629–327,736) |
| p50 ms | 5.868 (5.844–5.966) | 2.908 (2.864–3.153) |
| p99 ms | 7.073 (7.037–9.580) | 5.468 (5.320–37.686) |
| p999 ms | 9.448 (8.553–15.405) | 5.905 (5.680–74.729) |
| Bound for the run ms | 1.966 (1.926–2.192) | 1.954 (1.942–2.223) |
| Groups | 1,008 (1,006–1,017) | 1,108 (1,091–1,324) |
| Submits in a group, p50 | 1,000 (1,000–1,000) | 983 (835–989) |
| Peak reserved bytes | 480,000 | 528,000 |
| Reopen ms | 860.2 (851.7–866.9) | 921.1 (908.6–1,154.5) |

- The first run of the relay was disturbed: 190,629 records/s with a p99 of 37.686 ms. The other four runs are within 323,513–327,736 records/s with a p99 of 5.320–5.506 ms. The disturbed run is kept in every range.
- The bound is the linger plus twice the device `fdatasync` p99 that the same run measured. All ten runs miss it.
- The peak reserved bytes grow by 48 for each request, because the admission charge now counts the reply slot.
- Reopen was slower after every relay run. The relay changes no recovery, codec or key code. Its runs wrote about 10% more groups, and callers that wake in parallel submit in a different order, so the reopened log has more frames and a different key order. This record did not test which of these explains the difference.

## Why the p99 still misses the bound

The bound is the linger plus two flushes, about 2 ms here. It leaves no time for processor work or wake-ups.

- With the relay, the writer's cycle is 2.79 ms: 1.04 ms of formation, 1.06 ms of encode, write and flush, 0.35 ms of publish and 0.26 ms of delivery. A request that arrives just after a group closes waits most of one cycle and then takes part in the next.
- The benchmark is a closed loop: each caller submits again as soon as it is answered. Its mean latency is then the number of callers divided by the throughput. A mean of 2 ms with 1,000 callers needs 500,000 records/s, ten times the PRD's throughput target.
- Waking 1,000 parked threads takes about 2 ms on this guest even when the wake-ups run in parallel, as the next section shows. The guest has 10 processors.

## Wake-up strategies

Medians of 200 rounds for 1,000 parked threads, then the median and range of three repetitions, in microseconds. "Waker" is the time the waking thread spends before it can do other work. [wake/](wake/) holds the three outputs.

| Strategy | Waker | Mean wake | Last wake |
| --- | ---: | ---: | ---: |
| One thread unparks every thread | 3,407 (3,346–3,469) | 1,739 (1,708–1,759) | 3,430 (3,365–3,482) |
| Relay, fan-out 2 | 24 (21–25) | 900 (682–932) | 1,794 (1,350–1,890) |
| Relay, fan-out 4 | 46 (42–46) | 1,026 (909–1,051) | 2,105 (1,926–2,109) |
| Relay, fan-out 8 | 78 (78–83) | 815 (579–828) | 1,649 (1,322–1,726) |
| Relay, fan-out 16 | 250 (224–332) | 760 (520–765) | 2,066 (1,508–2,139) |
| Relay, fan-out 32 | 444 (350–518) | 735 (677–751) | 1,934 (1,707–2,098) |
| `Condvar::notify_all`, one lock | 3,470 (1,627–3,531) | 1,798 (1,122–1,832) | 3,483 (2,748–3,544) |
| `Condvar::notify_all`, 128 locks | 3,419 (3,347–3,924) | 1,730 (1,704–2,028) | 3,432 (3,363–3,944) |

- One thread that wakes every other thread needs 3.4 ms, with or without a condition variable. This matches the 3.23 ms of delivery in the trace.
- Every relay fan-out wakes the last thread in 1.6 to 2.1 ms and frees the waker in less than 0.5 ms.
- The microbenchmark's relay looks up thread handles under one shared lock. The engine's relay does not, so these relay figures are an upper estimate, and the order of the fan-outs here does not carry over to the engine.

## Fan-out

The engine's fan-out is four. To check that choice, the same commit was built with fan-outs 2, 8 and 16, and the default benchmark ran five rounds with one run of each build in every round, through [gated.sh](gated.sh). The other containers of the guest were idle in this period. [fanout/](fanout/) holds the twenty logs.

| Fan-out | Records/s median (range) | p50 ms median (range) | p99 ms median (range) |
| ---: | --- | --- | --- |
| 2 | 315,437 (311,269–320,212) | 2.925 (2.879–2.982) | 5.399 (5.316–5.568) |
| 4 | 316,963 (274,823–322,680) | 2.914 (2.892–3.057) | 5.523 (5.428–10.761) |
| 8 | 300,382 (297,228–301,070) | 3.037 (3.026–3.064) | 5.979 (5.886–5.999) |
| 16 | 299,868 (293,879–301,274) | 3.051 (3.038–3.123) | 5.879 (5.832–6.015) |

- Fan-outs 2 and 4 are the same within the spread of the runs. Fan-outs 8 and 16 are about 5% slower, with a p99 about 0.4 ms higher.
- One run with fan-out 4 was disturbed (274,823 records/s, p99 10.761 ms). Its other four runs are within 314,548–322,680 records/s.
- Four halves the depth of the tree against two, so fewer callers stand between the writer and the last caller. That is why the engine uses four and not two.

## Limits

- One guest with 10 processors, one session, five pairs, three trace repetitions.
- The relay moves work to caller threads. This record does not measure callers at different scheduling priorities, or fewer callers than processors.
- No paced, open-loop or mixed read and append workload ran with the relay alone.
- No run above 1,000 callers and no history above 1,000,000 records.

## Checks

The checks ran on this branch's sources (`segment-log-core` tree `20b6837999cf`), on the Mac host and in the Linux container. All passed on both.

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`: 140 unit and scenario tests, 3 process tests and 2 other integration tests. One test is ignored: the scheduled long run.
- The long run with the hosted settings: 100 trace seeds from 10000, 2,700 scenarios.
- 30 process-kill repetitions from seed 10000.
- 12 Python script tests.

[checks.txt](checks.txt) holds the result lines. Hosted CI had not run for this branch when this record was written.

## Reproduce

Build both commits in the pinned image with `cargo test -p segment-log-core --lib --release --no-run`. Keep the data directory on the container's own filesystem.

```sh
SEGMENT_LOG_EVIDENCE_DIR=/data/bench scripts/m1-compare-latency.sh BASE_TEST_BINARY CHANGE_TEST_BINARY latency
python3 scripts/m1-latency-summary.py latency before after

SEGMENT_LOG_EVIDENCE_DIR=/data/bench SEGMENT_LOG_BENCH_CALLERS=1000 SEGMENT_LOG_BENCH_ROUNDS=200 \
  SEGMENT_LOG_DEVICE_ROUNDS=200 SEGMENT_LOG_PIPELINE_OUT=/data/trace.tsv \
  TEST_BINARY --exact acceptance::evidence::measured_appends_survive_reopen --nocapture
python3 scripts/m1-group-phases.py /data/trace.tsv --min-count 900

rustc -O --edition 2024 docs/evidence/issue20/ack-relay/wake/wake.rs -o wake && ./wake 1000 200
```
