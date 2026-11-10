# Positioned reads: read and append evidence for issue 20

The [Mac concurrent evidence](../concurrent-macos/README.md) for [issue 20](https://github.com/ironflowhq/segment-log/issues/20) found readers blocked in the storage mutex, which real storage held across every read, write and flush. This record measures one change: real storage reads at a position on a shared handle, and the lock covers only the handle lookup.

## Result

**A read no longer waits for a group flush.** With one caller appending 1,024 groups, 1,023 or 1,024 reads took longer than 100 µs in every run on `main`: one read for each group. With the change, none or one did. The reader's p99 bucket fell from 238–393 µs to 22–24 µs in every one of five runs. The read p50 did not change. The reader completed 86,894–88,508 calls in the append interval, up from 73,590–78,664.

**No change for appends.** The probe's append throughput and latency buckets are the same within the spread of the runs. The default 1,000-caller benchmark has no reader. Its first two batches were disturbed, and three slow runs of the change in the first batch looked like a regression. A third batch on a quiet guest shows the two binaries within 3% of each other, as [Append benchmark](#append-benchmark) shows.

This supports the change for readers. It makes no claim about a PRD target, about many readers, or about target hardware.

## Scope and provenance

- `main` is `5e95407cb3e427800cdb29c687595d35bae56410`. The change is `975482446031f649f9d4f1637a3157283e96b171`. Commits after it on this branch change documents only. [sources.txt](sources.txt) and [binaries.sha256](binaries.sha256) identify what ran.
- Captured on 9 November 2026 in the pinned `rust:1.99.1-bookworm` image (`sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`) on Docker Desktop for Mac. The guest runs Linux 7.0.14-linuxkit on aarch64 with 10 processors and 8 GiB. Data was on the container's overlay filesystem, on the guest's virtual disk. [environment.txt](environment.txt), [docker-info.txt](docker-info.txt) and [host.txt](host.txt) hold the details.
- The host is an Apple M4 with 10 cores and 24 GiB on macOS 27.0.1. Unrelated containers on the same guest ran database tests in bursts during this session. They share its processors and its disk. For the third benchmark batch, [gated.sh](gated.sh) waited before each run until those containers used less than 10% of a processor and recorded their use in the `others` files. Nothing outside this experiment was stopped.
- This is a virtual Linux guest on a Mac. It is not physical local NVMe and it is not the deployment storage stack.

## Method

Reads use the existing `concurrent_probe` example: `concurrent_probe DIR MODE keyed default 1024 32768 timing`. One caller appends 32 keyed records for each submit under all default options, from 1,024 to 33,792 records. In `concurrent` mode one reader thread reads the first record and the last record below the watermark in a loop, and catches up 32 records at a time. In `append` mode no reader runs. Five alternating repetitions ran for each binary and mode. Every run validated all 33,792 records after reopen, with no refusal and no append, read or validation error.

Latency values are histogram buckets, not exact nanoseconds. [reads/summarize.py](reads/summarize.py) pools the buckets of the first-record and last-record reads within one run. It never pools runs.

The append benchmark uses `scripts/m1-compare-latency.sh` with 1,000 callers and 1,000 rounds: one warm-up for each binary, then five alternating pairs. It ran twice with that script and a third time with the same procedure through [gated.sh](gated.sh).

## Reads during appends

`base` is `main` and `reads` is this change. [reads/summary.md](reads/summary.md) is the generated table.

| Binary | Run | Append records/s | Append p50 µs | Read calls | Read p50 µs | Read p99 µs | Reads over 100 µs |
| --- | ---: | ---: | --- | ---: | --- | --- | ---: |
| base | 1 | 20,612 | 1507–1573 | 73,958 | 17–18 | 238–246 | 1,024 |
| base | 2 | 20,793 | 1507–1573 | 73,590 | 16–17 | 238–246 | 1,024 |
| base | 3 | 20,887 | 1507–1573 | 73,656 | 16–17 | 238–246 | 1,023 |
| base | 4 | 18,156 | 1769–1835 | 78,450 | 16–17 | 377–393 | 1,024 |
| base | 5 | 18,234 | 1704–1769 | 78,664 | 16–17 | 377–393 | 1,024 |
| reads | 1 | 20,443 | 1507–1573 | 88,508 | 17–18 | 22–23 | 1 |
| reads | 2 | 20,778 | 1507–1573 | 87,408 | 17–18 | 23–24 | 0 |
| reads | 3 | 20,646 | 1507–1573 | 88,078 | 17–18 | 23–24 | 0 |
| reads | 4 | 20,679 | 1507–1573 | 87,700 | 17–18 | 23–24 | 1 |
| reads | 5 | 20,981 | 1507–1573 | 86,894 | 17–18 | 23–24 | 0 |

- Each run appends 1,024 groups. On `main` one read for each group waits for that group's `fdatasync`, which takes about 0.3 ms on this guest. With the change, at most one read in a run took longer than 100 µs, and the p99 bucket is within 7 µs of the p50 bucket.
- The read p99 on `main` depends on how many reads fit between two flushes, so it is a weaker measure than the count of slow reads.
- The fourth and fifth `main` runs were slower for the appender too. The append throughput of the other runs is the same for both binaries within 3%.
- Without a reader, both binaries appended 12,261–14,974 records/s in every run. That is slower than with a reader for both binaries, so it is a property of this guest when it is otherwise idle and not of the change.

## Append benchmark

The default benchmark has no reader, so its writer never competes for the removed lock. It was run to look for a regression.

| Batch | | `main` median (range) | This change median (range) |
| --- | --- | --- | --- |
| First | Records/s | 168,192 (167,375–169,037) | 150,114 (146,956–168,351) |
| First | p50 ms | 5.889 (5.869–5.901) | 5.918 (5.883–6.071) |
| First | p99 ms | 7.684 (7.020–8.191) | 18.611 (7.189–20.615) |
| Repeat | Records/s | 111,034 (49,701–161,337) | 78,153 (40,531–165,439) |
| Repeat | p50 ms | 8.361 (5.933–16.948) | 12.167 (5.887–24.876) |
| Repeat | p99 ms | 15.749 (10.690–68.761) | 31.982 (9.398–48.214) |
| Gated | Records/s | 165,295 (163,247–168,111) | 169,053 (163,532–169,403) |
| Gated | p50 ms | 5.913 (5.894–5.989) | 5.876 (5.863–5.896) |
| Gated | p99 ms | 8.409 (7.122–9.430) | 6.860 (6.618–9.649) |

- **First.** Three runs of the change had a p99 near 20 ms and about 148,000 records/s. Its other two runs match `main`: 167,944 and 168,351 records/s with a p99 of 7.2 ms. The other containers' use was not recorded for this batch.
- **Repeat.** Both binaries were disturbed. Two `main` runs fell to about 50,000 records/s, and the device `fdatasync` p99 that each run measures before its appends ranged from 0.45 ms to 26.7 ms.
- **Gated.** The other containers used less than 7% of a processor around every run. The change was faster in four of five pairs, by 2% at the median, and no range is separated. The paced-benchmark change for the same issue, which also leaves the write path alone, shows the same 2% for the second run of a pair. So this difference is the order of the runs, not the change.
- The slow runs of the first batch did not return on a quiet guest. The code gives no path for a regression either: the writer makes the same `lseek`, `write` and `fdatasync` calls as before, and the only added work is one reference count for each storage call.

[latency/summary.md](latency/summary.md), [latency-repeat/summary.md](latency-repeat/summary.md) and [latency-gated/summary.md](latency-gated/summary.md) list every run. No run was removed.

## Limits

- One reader and one appender. Many readers, larger histories and multiple segments were not measured.
- A positioned read still competes with the writer for the page cache and the device. This change removes only the lock.

## Checks

The checks ran on this branch's sources (`segment-log-core` tree `b868ea313081`), on the Mac host and in the Linux container. All passed on both.

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`: 138 unit and scenario tests, 3 process tests and 3 other integration tests. One test is ignored: the scheduled long run.
- The long run with the hosted settings: 100 trace seeds from 10000, 2,700 scenarios.
- 30 process-kill repetitions from seed 10000.
- 9 Python script tests.

[checks.txt](checks.txt) holds the result lines. Hosted CI had not run for this branch when this record was written.

## Reproduce

Build both commits in the pinned image with `cargo test -p segment-log-core --lib --release --no-run` and `cargo build --release -p segment-log-core --examples`. Keep the data directory on the container's own filesystem.

```sh
BASE/concurrent_probe /data/c concurrent keyed default 1024 32768 timing > reads/base-concurrent-1.txt
CHANGE/concurrent_probe /data/c concurrent keyed default 1024 32768 timing > reads/reads-concurrent-1.txt
python3 docs/evidence/issue20/positioned-reads/reads/summarize.py reads base reads
SEGMENT_LOG_EVIDENCE_DIR=/data/bench scripts/m1-compare-latency.sh BASE_TEST_BINARY CHANGE_TEST_BINARY latency
python3 scripts/m1-latency-summary.py latency before after
```
