# Key positions: memory and append evidence for issue 20

[Issue 20](https://github.com/ironflowhq/segment-log/issues/20) leaves the engine's memory growth open. This record measures one change: the key map keeps each key's log offset and revision, and no longer a copy of its frame descriptor.

## Result

**Reduced requested heap, exactly and in every repetition.** A reopened log with 1,000,000 eight-byte keys retains 78,049,185 requested bytes, down from 169,819,633 (−54%). With 16,384 keys the retained bytes fall from 3,204,617 to 1,606,713. That is 81.9 bytes for each key instead of 179.4. Histories without keys retain the same bytes as before.

**Small append improvement on a Linux guest.** In five alternating pairs of the default benchmark, the median throughput rose from 168,148 to 176,304 records/s and the median p50 fell from 5.894 ms to 5.599 ms. The ranges of both are separated. The p99 was lower in all five pairs, but its ranges overlap. Every run still misses the PRD's p99 bound.

This supports the change. It makes no claim about a PRD target on target hardware, about resident memory, or about the M4 key lookup design.

## Scope and provenance

- `main` is `5e95407cb3e427800cdb29c687595d35bae56410`. The change is `fcca44aa272f9f65904a76ae1ae069db5caa5bf0`. Commits after it on this branch change documents only. [sources.txt](sources.txt) and [binaries.sha256](binaries.sha256) identify what ran.
- Captured on 9 November 2026 in the pinned `rust:1.99.1-bookworm` image (`sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`) on Docker Desktop for Mac. The guest runs Linux 7.0.14-linuxkit on aarch64 with 10 processors and 8 GiB. Data was on the container's overlay filesystem, on the guest's virtual disk. [environment.txt](environment.txt), [docker-info.txt](docker-info.txt) and [host.txt](host.txt) hold the details.
- The host is an Apple M4 with 10 cores and 24 GiB on macOS 27.0.1. It ran unrelated work. Each run's log is preceded by the guest's load average, which includes the run before it.
- This is a virtual Linux guest on a Mac. It is not physical local NVMe and it is not the deployment storage stack.

## Method

Memory uses the existing `memory_probe` example through `scripts/m1-memory-probe.py`. It counts requested heap bytes with a counting allocator, so the numbers are exact and do not depend on timing. "Retained" is the number of bytes that dropping the reopened engine frees (`drop_engine`). The matrix ran each case twice for each binary with 16,384 records. Two larger histories ran once for each binary: 1,000,000 records in 1,000 groups on 1,000 streams, with every record keyed and with none keyed. Keys and stream ids have eight bytes.

Append timing uses `scripts/m1-compare-latency.sh`: the default benchmark with 1,000 callers and 1,000 rounds, one warm-up for each binary, then five alternating pairs. `scripts/m1-latency-summary.py` wrote [latency/summary.md](latency/summary.md) from the retained logs. Every run validated all 1,000,000 records before and after reopen, with no refusal and no commit or correctness error.

## Memory

Requested heap bytes. Both repetitions of each matrix case gave the same value.

| History | Retained, `main` | Retained, this change | Each key, `main` | Each key, this change |
| --- | ---: | ---: | ---: | ---: |
| 16,384 records, no keys | 264,937 | 264,937 | – | – |
| 16,384 records, 8,192 keys | 1,632,553 | 892,121 | 166.9 | 76.6 |
| 16,384 records, 16,384 keys | 3,204,617 | 1,606,713 | 179.4 | 81.9 |
| 1,000,000 records, no keys | 389,425 | 389,425 | – | – |
| 1,000,000 records, 1,000,000 keys | 169,819,633 | 78,049,185 | 169.4 | 77.7 |

"Each key" is the retained bytes above the same history without keys, divided by the number of keys. It includes the key's own eight bytes and the map's node overhead.

The change adds one transient cost. While a group forms, the writer keeps 16 bytes for each of the group's new records, to find a pending record by its offset. With 1,024 records in each group, the peak extra bytes during the append phase of the unkeyed case rose from 181,540 to 197,924. The memory is released with the group.

[memory/allocations.csv](memory/allocations.csv) holds every phase of the matrix, and the four text files hold the larger histories.

## Append benchmark

Default options, 1,000 callers, 256-byte payloads, one keyed record for each submit.

| | `main` median (range) | This change median (range) |
| --- | --- | --- |
| Records/s | 168,148 (143,819–169,557) | 176,304 (174,272–177,062) |
| p50 ms | 5.894 (5.835–6.055) | 5.599 (5.575–5.659) |
| p99 ms | 8.307 (7.091–18.128) | 7.119 (6.496–7.916) |
| p999 ms | 10.369 (9.087–37.122) | 10.079 (8.022–11.611) |
| Bound for the run ms | 1.942 (1.683–1.955) | 1.949 (1.692–2.116) |
| Reopen ms | 883.9 (871.2–1,008.6) | 836.4 (825.9–848.3) |
| Peak resident set KiB | 774,944 (772,408–830,076) | 670,824 (629,336–671,956) |

- Throughput and p50 were better in all five pairs, and their ranges are separated.
- The p99 was lower in all five pairs. Its ranges overlap, because the fifth `main` run was disturbed (143,819 records/s, p99 18.128 ms). That run is kept.
- The bound is the linger plus twice the device `fdatasync` p99 that the same run measured. All ten runs miss it.
- Reopen was faster in every pair and its ranges are separated. Recovery now writes one smaller value for each key.
- The peak resident set is for the whole test process, including the benchmark's own result arrays. Its ranges are separated, which agrees with the exact figures above, but it does not measure engine ownership.

## Limits

- One guest, one session, five pairs. The host ran other work.
- Requested bytes are not resident bytes. Allocator size classes, thread stacks and kernel memory are outside the count.
- The map is still one in-memory `BTreeMap` that holds every retained key. [PRD open item 1](../../../prd.md#open-items) keeps the M4 lookup mechanism and key-cache budget open. This change does not settle either.
- No history above 1,000,000 keys, and no multi-segment history, was measured.

## Checks

The checks ran on this branch's sources (`segment-log-core` tree `5ea762220ea8`), on the Mac host and in the Linux container. All passed on both.

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`: 138 unit and scenario tests, 3 process tests and 2 other integration tests. One test is ignored: the scheduled long run.
- The long run with the hosted settings: 100 trace seeds from 10000, 2,700 scenarios.
- 30 process-kill repetitions from seed 10000.
- 9 Python script tests.

[checks.txt](checks.txt) holds the result lines. Hosted CI had not run for this branch when this record was written.

## Reproduce

Build the two commits in the pinned image with `cargo test -p segment-log-core --lib --release --no-run` and `cargo build --release -p segment-log-core --examples`. Keep the data directory on the container's own filesystem, not on a bind mount.

```sh
python3 scripts/m1-memory-probe.py base=BASE/memory_probe keys=CHANGE/memory_probe \
  --records 16384 --repetitions 2 --data /data/mem --output allocations.csv
BASE/memory_probe /data/mem 1000000 1000000 1000 1000 32
CHANGE/memory_probe /data/mem 1000000 1000000 1000 1000 32
SEGMENT_LOG_EVIDENCE_DIR=/data/bench scripts/m1-compare-latency.sh BASE_TEST_BINARY CHANGE_TEST_BINARY latency
python3 scripts/m1-latency-summary.py latency before after
```
