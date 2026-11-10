# Stream heads: many-stream append evidence for issue 20

[Issue 20](https://github.com/ironflowhq/segment-log/issues/20) leaves performance on larger histories open. This record measures one change: before it forms a group, the writer no longer copies the head of every stream in the log. A group now keeps only the revisions it assigns and reads the durable head of each stream it touches.

## Result

**The cost of a group no longer grows with the number of streams.** Appending 2,000 groups of 100 records took 7.4–11.5 s on `main` when the log held 100,000 streams, and 0.60–0.73 s with the change. With one stream both binaries took about 0.55–0.86 s. The transient heap for each group fell from 3,028,848 requested bytes to 26,472 at 100,000 streams, exactly and in every run.

**No established change in the default benchmark.** It has 1,000 streams. The median throughput was 5% higher and the median p50 5% lower, but no range is separated and the host was disturbed.

Reopen time did not change. This supports the change for logs with many streams. It makes no claim about a PRD target or about target hardware.

## Scope and provenance

- `main` is `5e95407cb3e427800cdb29c687595d35bae56410`. The change is `870349a319896f8b692ee1b9e6423fb742688a29`. Commits after it on this branch change documents only. [sources.txt](sources.txt) and [binaries.sha256](binaries.sha256) identify what ran.
- Captured on 9 November 2026 in the pinned `rust:1.99.1-bookworm` image (`sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`) on Docker Desktop for Mac. The guest runs Linux 7.0.14-linuxkit on aarch64 with 10 processors and 8 GiB. Data was on the container's overlay filesystem, on the guest's virtual disk. [environment.txt](environment.txt), [docker-info.txt](docker-info.txt) and [host.txt](host.txt) hold the details.
- The host is an Apple M4 with 10 cores and 24 GiB on macOS 27.0.1. Unrelated work ran on the host and in other containers of the same guest. [gated.sh](gated.sh) waited before each run until the other containers used less than 10% of a processor, and recorded their use before and after the run in the `others` files. It cannot see work on the host outside the guest. Nothing outside this experiment was stopped.
- This is a virtual Linux guest on a Mac. It is not physical local NVMe and it is not the deployment storage stack.

## Method

Many streams use the existing `growth_probe` example: `growth_probe DIR 200000 0 STREAMS 100 32`. One caller appends 200,000 unkeyed 32-byte records in 2,000 submits of 100 records, with zero linger, so every submit is one group. Stream ids cycle round-robin over the given number of streams. The probe counts requested heap bytes with a counting allocator and times each phase. Three alternating repetitions ran for each binary and stream count. Every run validated all 200,000 records after reopen.

The append benchmark follows `scripts/m1-compare-latency.sh` through [gated.sh](gated.sh): 1,000 callers and 1,000 rounds, one warm-up for each binary, then five alternating pairs.

## Many streams

`base` is `main` and `heads` is this change. [streams/summarize.py](streams/summarize.py) wrote the table from the retained outputs.

| Streams | Binary | Append ms, three runs | Peak extra heap bytes while appending | Reopen ms, three runs |
| ---: | --- | --- | ---: | --- |
| 1 | base | 812, 595, 548 | 18,916 | 28, 27, 27 |
| 1 | heads | 556, 862, 559 | 18,916 | 28, 27, 26 |
| 1,000 | base | 658, 674, 644 | 32,552 | 37, 36, 37 |
| 1,000 | heads | 540, 546, 612 | 26,472 | 36, 36, 37 |
| 10,000 | base | 1,473, 1,392, 1,350 | 261,032 | 44, 44, 44 |
| 10,000 | heads | 562, 569, 574 | 26,472 | 44, 44, 44 |
| 100,000 | base | 7,431, 7,550, 11,518 | 3,028,848 | 50, 52, 52 |
| 100,000 | heads | 597, 628, 726 | 26,472 | 49, 54, 52 |

- On `main` the append time grows with the stream count: each of the 2,000 groups copies every head while it holds the lock that readers also take. At 100,000 streams that is about 3.4 ms for each group.
- With the change the append time is 0.54–0.73 s from 1,000 to 100,000 streams. The ranges at 10,000 and at 100,000 streams are separated from `main` by a factor of two and of ten.
- The peak extra heap is the largest number of requested bytes above the start of the append phase. It is the same in all three runs of a case.
- At one stream the two binaries do the same work. Their first or second run was slower in both, which shows the spread of this guest.

## Append benchmark

Default options, 1,000 callers on 1,000 streams, 256-byte payloads.

| | `main` median (range) | This change median (range) |
| --- | --- | --- |
| Records/s | 147,022 (102,800–155,560) | 155,030 (145,002–156,163) |
| p50 ms | 6.703 (6.127–7.742) | 6.338 (5.978–6.605) |
| p99 ms | 10.750 (9.809–43.629) | 10.147 (9.540–10.736) |
| Bound for the run ms | 1.927 (1.893–1.994) | 1.929 (1.867–2.245) |
| Reopen ms | 961.8 (930.4–1,002.2) | 955.7 (929.1–961.7) |

- Throughput was higher in three of five pairs, and the p50 and p99 were lower in four. No range is separated.
- Both binaries were slower here than `main` in a quieter hour of the same session (about 168,000 records/s with a p50 of 5.87 ms), although the other containers were idle. Work on the host outside the guest is the likely cause, and it is not recorded for each run.
- All ten runs miss the p99 bound.

[latency/summary.md](latency/summary.md) lists every run. No run was removed.

## Rejected alternative

An earlier version of this change also updated an existing stream's head in place when a group is published, to avoid one allocation for each record. A new stream then needs two map lookups instead of one. With 100,000 streams and two records for each stream, reopen took 58, 68 and 60 ms against 51, 50 and 50 ms on `main`. With 1,000 streams it took 32, 31 and 32 ms against 36, 36 and 36 ms. The gain for a few streams did not justify the loss for many, and the lookup is not needed to remove the copy, so it was dropped. [candidates/](candidates/) holds the patch and the six outputs at 100,000 streams.

## Limits

- One caller, groups of 100 records, and at most 100,000 streams. More streams, more callers and keyed records were not measured.
- The durable heads are still one in-memory map with every stream of the log. This change removes the copy of it for each group, not the map.
- The default benchmark needs a quiet host to show a small difference.

## Checks

The checks ran on this branch's sources (`segment-log-core` tree `8c34ed74ea54`), on the Mac host and in the Linux container. All passed on both.

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`: 138 unit and scenario tests, 3 process tests and 2 other integration tests. One test is ignored: the scheduled long run.
- The long run with the hosted settings: 100 trace seeds from 10000, 2,700 scenarios.
- 30 process-kill repetitions from seed 10000.
- 9 Python script tests.

[checks.txt](checks.txt) holds the result lines. Hosted CI had not run for this branch when this record was written.

## Reproduce

Build both commits in the pinned image with `cargo test -p segment-log-core --lib --release --no-run` and `cargo build --release -p segment-log-core --examples`. Keep the data directory on the container's own filesystem.

```sh
BASE/growth_probe /data/g 200000 0 100000 100 32 > streams/base-100000-1.txt
CHANGE/growth_probe /data/g 200000 0 100000 100 32 > streams/heads-100000-1.txt
python3 docs/evidence/issue20/stream-heads/streams/summarize.py streams base heads
SEGMENT_LOG_EVIDENCE_DIR=/data/bench scripts/m1-compare-latency.sh BASE_TEST_BINARY CHANGE_TEST_BINARY latency
python3 scripts/m1-latency-summary.py latency before after
```
