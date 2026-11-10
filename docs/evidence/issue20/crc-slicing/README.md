# CRC by eight bytes: encode, reopen and read evidence for issue 20

[Issue 20](https://github.com/ironflowhq/segment-log/issues/20) lists no proven throughput improvement, and the [earlier latency work](../../m1/latency/README.md) replaced the bitwise CRC with a one-byte table. This record measures the next step: the checksum takes eight input bytes for each step, and a frame is checked in two parts instead of being copied first.

## Result

**Same durable bytes.** `scripts/m1-compare-codec.sh` compiled the codec of `main` and of this change separately. Both wrote the same 451,781 bytes of checksums, group frames and headers.

**Less work in the writer, in recovery and in reads.**

- The writer's encode phase for a group of 1,000 records fell from 0.64–0.66 ms to 0.20–0.21 ms in three traced repetitions.
- Reopening 1,000,000 records took 476–567 ms instead of 861–919 ms. The ranges are separated, and a second, disturbed batch shows the same.
- A reader's p50 bucket for one record fell from 17–18 µs to 4–5 µs in every run.
- Every frame check allocates one frame less. Reopening 1,000,000 records requested 120,530,836 heap bytes in total instead of 176,558,836.

**Small append improvement, not established for the p99.** In the first batch the median p50 fell from 5.957 ms to 5.453 ms with separated ranges, and throughput was higher in all five pairs. The second batch was disturbed and separates nothing. No undisturbed run meets the PRD's p99 bound.

This supports the change. It makes no claim about a PRD target or about target hardware.

## Scope and provenance

- `main` is `5e95407cb3e427800cdb29c687595d35bae56410`. The change is `de94d4265adf867062b56227fdc65aaf55e545b4`. Commits after it on this branch change documents only. [sources.txt](sources.txt) and [binaries.sha256](binaries.sha256) identify what ran.
- Captured on 9 November 2026 in the pinned `rust:1.99.1-bookworm` image (`sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`) on Docker Desktop for Mac. The guest runs Linux 7.0.14-linuxkit on aarch64 with 10 processors and 8 GiB. Data was on the container's overlay filesystem, on the guest's virtual disk. [environment.txt](environment.txt), [docker-info.txt](docker-info.txt) and [host.txt](host.txt) hold the details.
- The host is an Apple M4 with 10 cores and 24 GiB on macOS 27.0.1. Unrelated work ran on the host and in other containers of the same guest. For the trace and read runs, [gated.sh](gated.sh) waited before each run until the other containers used less than 10% of a processor, and recorded their use in the `others` files. It cannot see work on the host outside the guest. Nothing outside this experiment was stopped.
- This is a virtual Linux guest on a Mac. It is not physical local NVMe and it is not the deployment storage stack. The checksum runs in software on an Apple M4 core; other processors will give other figures.

## Method

- **Equivalence.** `scripts/m1-compare-codec.sh OUT 5e95407cb3e427800cdb29c687595d35bae56410` on the Mac host. [codec-equivalence.txt](codec-equivalence.txt) is its output.
- **Benchmark.** `scripts/m1-compare-latency.sh` with 1,000 callers and 1,000 rounds: one warm-up for each binary, then five alternating pairs. It ran twice.
- **Trace.** The benchmark's request trace with 1,000 callers and 200 rounds, three alternating repetitions for each binary. `scripts/m1-group-phases.py --min-count 900` split the writer's cycle and counted only groups of at least 900 requests. That script arrives with the ack-relay change for the same issue and is not part of this branch. The phase tables are in [trace/](trace/).
- **Reads.** `concurrent_probe DIR concurrent keyed default 1024 32768 timing`: one caller appends 32 keyed records for each submit while one reader reads the first and the last record in a loop. Five alternating repetitions. Latency values are histogram buckets.
- **Memory.** `memory_probe` through `scripts/m1-memory-probe.py`, which counts requested heap bytes exactly, and two histories of 1,000,000 records.

Every benchmark, trace and probe run validated all its records, with no refusal and no commit, read or correctness error.

## Encode phase

One group of 1,000 records is one frame of 296,032 bytes. Median over the counted groups of each repetition, in milliseconds.

| Phase | `main`, three repetitions | This change, three repetitions |
| --- | --- | --- |
| Encode and CRC | 0.641, 0.659, 0.660 | 0.214, 0.202, 0.206 |
| Whole cycle | 5.926, 6.678, 6.649 | 5.424, 5.479, 5.427 |

The encode phase is about a tenth of the cycle on `main`. The largest part of the cycle is the writer waking callers, which this change does not touch. Two `main` repetitions had a longer cycle because that delivery phase took 4.0 ms instead of 3.2 ms.

## Append benchmark

Default options, 1,000 callers, 256-byte payloads.

| Batch | | `main` median (range) | This change median (range) |
| --- | --- | --- | --- |
| First | Records/s | 162,534 (129,134–169,318) | 180,589 (160,467–181,684) |
| First | p50 ms | 5.957 (5.865–6.750) | 5.453 (5.439–5.559) |
| First | p99 ms | 10.776 (6.820–23.243) | 7.481 (7.458–11.187) |
| First | Reopen ms | 878.1 (860.8–919.3) | 536.4 (475.5–567.1) |
| Repeat | Records/s | 127,795 (118,034–135,770) | 133,450 (122,674–141,167) |
| Repeat | p50 ms | 6.832 (6.648–7.430) | 6.789 (6.492–7.466) |
| Repeat | p99 ms | 21.095 (14.201–25.043) | 22.216 (10.916–24.951) |
| Repeat | Reopen ms | 1,224.4 (1,058.2–1,270.2) | 559.9 (539.5–651.9) |

- First batch: throughput and p50 were better in all five pairs. The p50 ranges are separated. The throughput ranges overlap, because the first and fifth pairs were disturbed for both binaries. The p99 was lower in three of five pairs.
- Repeat: both binaries were disturbed in every run, with a p99 of 11–25 ms and a device `fdatasync` p99 of up to 9.7 ms. It shows no difference for appends.
- Reopen was faster in all ten pairs and its ranges are separated in both batches. Recovery checks every frame, so it gains the most.
- Nineteen of the twenty runs miss the p99 bound. One disturbed `main` run of the repeat is inside it only because that run measured a device `fdatasync` p99 of 9.7 ms, which puts its bound at 20.5 ms.

[latency/summary.md](latency/summary.md) and [latency-repeat/summary.md](latency-repeat/summary.md) list every run. No run was removed.

## Reads during appends

`base` is `main` and `crc` is this change. [reads/summary.md](reads/summary.md) is the generated table.

| Binary | Run | Read calls | Read p50 µs | Read p99 µs | Reads over 100 µs |
| --- | ---: | ---: | --- | --- | ---: |
| base | 1 | 74,010 | 17–18 | 254–262 | 1,023 |
| base | 2 | 68,546 | 17–18 | 279–295 | 1,025 |
| base | 3 | 70,904 | 17–18 | 410–426 | 1,024 |
| base | 4 | 69,424 | 17–18 | 262–279 | 1,025 |
| base | 5 | 72,742 | 17–18 | 238–246 | 1,024 |
| crc | 1 | 254,086 | 5–5 | 10–11 | 1,031 |
| crc | 2 | 253,748 | 5–5 | 7–8 | 1,024 |
| crc | 3 | 261,760 | 5–5 | 6–7 | 1,024 |
| crc | 4 | 254,254 | 5–5 | 7–8 | 1,026 |
| crc | 5 | 294,412 | 4–4 | 7–7 | 1,024 |

- A read decodes and checks the whole frame that holds its record. With the change the reader completed about 3.6 times as many calls in the same append interval.
- Each run appends 1,024 groups, and about 1,024 reads in each run still take longer than 100 µs. Those reads wait for a group flush behind the storage mutex, which this change does not touch. The lower p99 bucket only shows that the waiting reads are now less than 1% of all reads.
- The appender's throughput was 17,980–20,797 records/s on `main` and 18,211–20,598 with the change.

## Memory

Requested heap bytes, exact and the same in both repetitions of each matrix case. A frame check on `main` copied the frame without its checksum field.

| Measure | `main` | This change |
| --- | ---: | ---: |
| One-record read from a frame of 1,024 records, 32-byte payloads | 172,558 | 115,186 |
| One-record read from a frame of 1,024 records, 256-byte payloads | 631,534 | 344,786 |
| Peak extra while appending 16 groups of 1,024 records | 181,540 | 124,202 |
| Total requested while reopening 1,000,000 records in 1,000 groups | 176,558,836 | 120,530,836 |
| Retained after that reopen | 389,425 | 389,425 |

The tables add 7,168 bytes of constant data to the binary: eight tables of 1,024 bytes instead of one.

## Limits

- One guest and one processor type. The gain depends on the processor's cache and on the frame size.
- The checksum is still software. A version on processor instructions needs `unsafe` or a dependency, and so an ADR. It was not measured.
- The append gain is small beside the cost of waking callers, and the p99 is not established.

## Checks

The checks ran on this branch's sources (`segment-log-core` tree `263a3e7dc4c4`), on the Mac host and in the Linux container. All passed on both.

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`: 140 unit and scenario tests, 3 process tests and 2 other integration tests. One test is ignored: the scheduled long run.
- The long run with the hosted settings: 100 trace seeds from 10000, 2,700 scenarios.
- 30 process-kill repetitions from seed 10000.
- 9 Python script tests.

[checks.txt](checks.txt) holds the result lines. Hosted CI had not run for this branch when this record was written.

## Reproduce

Build both commits in the pinned image with `cargo test -p segment-log-core --lib --release --no-run` and `cargo build --release -p segment-log-core --examples`. Keep the data directory on the container's own filesystem.

```sh
scripts/m1-compare-codec.sh .audit/codec-compare 5e95407cb3e427800cdb29c687595d35bae56410
SEGMENT_LOG_EVIDENCE_DIR=/data/bench scripts/m1-compare-latency.sh BASE_TEST_BINARY CHANGE_TEST_BINARY latency
python3 scripts/m1-latency-summary.py latency before after
BASE/concurrent_probe /data/c concurrent keyed default 1024 32768 timing > reads/base-concurrent-1.txt
python3 docs/evidence/issue20/crc-slicing/reads/summarize.py reads base crc
python3 scripts/m1-memory-probe.py base=BASE/memory_probe crc=CHANGE/memory_probe \
  --records 16384 --repetitions 2 --data /data/mem --output allocations.csv
BASE/memory_probe /data/mem 1000000 0 1000 1000 32
```
