# Random reads over many sealed segments, before and after sparse frame tables (issue 177)

[Issue 177](https://github.com/ironflowx/segment-log/issues/177) found that random reads over more than 16 sealed segments reread frame headers on nearly every call. [ADR 0069](../../decisions/0069-reads-keep-sparse-frame-tables-under-a-byte-budget.md) replaces the 16 dense tables with sparse tables under a 24 MiB budget, and sets three done-when checks. [ADR 0070](../../decisions/0070-read-checks-are-judged-on-logs-the-page-cache-holds.md) amends check 1 for logs that the page cache does not hold. This record runs those checks.

## Verdict

- **Pass on the M4 Mac, under ADR 0069 as ADR 0070 amends it.** Every log passes checks 2 and 3. Every log passes check 1, except the 331-segment log of 1,000 callers, which half fits in memory. A fully resident log of the same shape (103 segments) passes check 1 and judges that case.
- **Random reads past 16 segments no longer walk.** On the 43-segment log, random p50 fell from 1,191 to 209 µs. On the 199-segment log, from 1,268 to 214 µs. On the 1,000-caller logs, from about 5,000 µs to 51 µs (103 segments, resident) and 216 µs (331 segments, half resident).
- **Scans are not slower.** Scan p50 and p99 are within 10% of the pre-change scan on every log. On the logs of small groups, scans got faster: 703,000 → 1,116,000 records/s on the 331-segment 1,000-caller log, and 139,000 → 1,371,000 records/s on the one-caller log, because one window read replaces many frame reads.
- **The 331-segment 1,000-caller log fails check 1 as ADR 0069 wrote it.** Its random p50 is 216 µs against a bar of 88 µs (2 × a 44 µs scan). Its frames are about 160 KiB, so every frame is kept and a random read is one direct frame read, the same as an old table hit. The 23.5 GB log is 0.91 of RAM, and half of it was resident, so a random read often reads from the SSD. The same shape at 7.3 GB, fully resident, reads at 51 µs.

The qualified-host rerun ADR 0069 asks for has not run yet. It applies the same rule.

## Host and provenance

- 2026-11-20, Apple M4 Mac mini: 10 cores, 24 GiB, Darwin 27.2.0, APFS on the internal SSD (Apple Fabric, PCI-Express). rustc 1.100.0, release build.
- Before: `0a11998`, the commit before the change. After: `9cd3ace` for the full matrix, and `62a6589` for the 103-segment log. The two after commits differ only in the driver and a test.
- Both commits read with the probe source of the after commit. It uses only the public API, which the change leaves alone, so only the library differs.
- The host ran an interactive session but no other benchmark. Raw output was not kept, per the [evidence rules](../README.md).
- To reproduce:
  - Full matrix: `docs/evidence/issue177/run.sh OUT_DIR 3 10`, about 45 minutes.
  - 103-segment log: `CASES=callers:100 CALLER_SECONDS=15 docs/evidence/issue177/run.sh OUT_DIR 3 10`.
  - Verdict: `summarize.py OUT_DIR/reads.txt OTHER/reads.txt`.

## Method

[run.sh](run.sh) derives from the [issue 156 driver](../issue156/run.sh). It builds each log once with the after commit's `readers_probe` and seals it. Then, on that same log:

1. [logstat.py](logstat.py) counts the group frames per segment from the frame envelopes.
2. One discarded `sealed` pass per commit walks every segment before any timed run.
3. Three rounds run random and scan reads at both commits, in an order shuffled with the round as seed. Each run uses one reader through `LogReader`, 64 records per call, for 10 s. Each run first reads one record at each segment's base and reports those first walks apart from the timed reads.
4. After the timed runs, `logstat.py` maps each segment file and asks `mincore(2)` how much of it is resident.

[summarize.py](summarize.py) takes the median over rounds of each run's quantile and applies the checks.

The logs:

- `s7`, `s18`, `s43`: the issue 156 logs, 1,500,000, 4,000,000 and 10,000,000 keyed 256-byte records over 1,000 streams, in submits of 256 records.
- `s200`: 46,500,000 of the same records, at the default 64 MiB roll.
- `callers`: 1,000 closed-loop callers, each sending one keyed 256-byte record per submit, run in rounds until the log reached 300 segments (331) or, for the resident log, 100 segments (103). The writer packs submits from several callers into each group.
- `single`: one caller that waits for each submit, so each group holds one record: 254,000 records at a 4 MiB roll, in 599 s.

## Logs

| Log | Group frames per segment (median) | Log GB | Log / RAM | Resident after |
| --- | ---: | ---: | ---: | ---: |
| s7 (7 segments) | 101 | 0.5 | 0.02 | 1.00 |
| s18 (17 segments) | 101 | 1.2 | 0.05 | 1.00 |
| s43 (43 segments) | 97 | 3.1 | 0.12 | 1.00 |
| s200 (199 segments) | 96 | 14.3 | 0.55 | 0.72 |
| callers (331 segments) | 443 | 23.5 | 0.91 | 0.50 |
| callers (103 segments) | 446 | 7.3 | 0.28 | 1.00 |
| single (20 segments) | 12,827 | 0.1 | 0.00 | 1.00 |

## Reads (one reader, 64 records per call)

| Log | Commit | Random p50 µs | Random p99 µs | Scan p50 µs | Scan p99 µs | Scan records/s | First walks: total ms, p50 µs, max µs |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| s7 | before | 207 | 294 | 180 | 212 | 408,156 | 8, 1,334, 1,372 |
| s7 | after | 205 | 292 | 182 | 222 | 401,138 | 8, 1,323, 1,373 |
| s18 | before | 210 | 294 | 184 | 216 | 401,909 | 22, 1,344, 1,405 |
| s18 | after | 210 | 294 | 184 | 212 | 401,778 | 22, 1,340, 1,412 |
| s43 | before | 1,191 | 1,421 | 191 | 218 | 391,548 | 55, 1,303, 1,413 |
| s43 | after | 209 | 306 | 191 | 219 | 392,102 | 55, 1,289, 1,406 |
| s200 | before | 1,268 | 1,732 | 180 | 267 | 398,756 | 318, 1,418, 13,958 |
| s200 | after | 214 | 628 | 180 | 269 | 398,901 | 257, 1,292, 1,459 |
| callers (331) | before | 5,053 | 96,189 | 44 | 280 | 702,638 | 26,342, 88,664, 109,434 |
| callers (331) | after | 216 | 444 | 45 | 236 | 1,116,023 | 10,230, 9,225, 105,547 |
| callers (103) | before | 4,930 | 5,177 | 44 | 86 | 1,254,135 | 504, 4,988, 5,266 |
| callers (103) | after | 51 | 91 | 44 | 86 | 1,285,338 | 500, 4,956, 5,123 |
| single | before | 458 | 1,281 | 455 | 492 | 138,578 | 15, 808, 839 |
| single | after | 24 | 461 | 22 | 462 | 1,370,656 | 14, 741, 765 |

The three rounds of each run agree closely. For example, callers (103) random p50 was 50.4–50.7 µs after and 4,888–4,936 µs before.

- **First walks.** The first read of each segment walks its frame headers. That costs about the same at both commits, 1.3 ms per 64 MiB segment of large groups and 5 ms for the 1,000-caller segments, where it is cached. On the 331-segment log the walk pass often reads from the SSD, which explains its larger totals.
- **The one-caller log** passes check 1 before the change too, because a 4 MiB walk is cheap. It is there to show that windows do not slow reads: random p50 fell from 458 to 24 µs.

## Checks

| Log | Check 1 before | Check 1 after | Judged by (ADR 0070) | Check 2 | Check 3 | In benefit verdict |
| --- | --- | --- | --- | --- | --- | --- |
| s7 | pass | pass | itself: pass | pass | pass | no |
| s18 | pass | pass | itself: pass | pass | pass | no |
| s43 | FAIL | pass | itself: pass | pass | pass | yes |
| s200 | FAIL | pass | itself: pass | pass | pass | yes |
| callers (331) | FAIL | FAIL | callers (103): pass | pass | pass | yes |
| callers (103) | FAIL | pass | itself: pass | pass | pass | yes |
| single | pass | pass | itself: pass | pass | pass | no |

- On this host, the 18-segment log passes check 1 before the change, on p99 too (294 µs against 648 µs). The issue 156 record saw a p99 of 1,628 µs at 18 segments. That log had 18 segments, this one 17, so it fits in 16 tables plus one walk more often.
- The s200 log is 72% resident and passes check 1 anyway, so ADR 0070 does not replace it.

## Limits of this run

- A Mac on APFS is not the deployment stack. Its page cache, readahead and SSD differ from the qualified Linux host. The qualified-host rerun confirms or reopens ADR 0069.
- One reader per run. Several readers on one `LogReader` share its tables, and were not measured here.
- The measured logs stop at 331 segments. The 24 MiB budget covers about 1,000 segments at the default roll. That figure is arithmetic, and the simulation tests eviction at the budget.
