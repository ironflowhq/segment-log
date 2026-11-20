---
id: "0070"
title: "Read checks are judged on logs that the page cache holds, not on SSD misses"
status: accepted
date: 2026-11-20
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [read-path, performance, evidence, release-0.1]
touches: [docs/evidence/issue177/**, docs/decisions/0069-reads-keep-sparse-frame-tables-under-a-byte-budget.md]
enforced_by: none
---

# 0070. Read checks are judged on logs that the page cache holds, not on SSD misses

## Context

[ADR 0069](0069-reads-keep-sparse-frame-tables-under-a-byte-budget.md) sets three done-when checks for sparse frame tables ([issue #177](https://github.com/ironflowx/segment-log/issues/177)). Check 1 asks that random-read p50 be at most 2× the pre-change scan p50, and p99 at most 3× the pre-change scan p99. It says that a random read that misses the page cache can fail check 1 for reasons unrelated to tables, and that such a failure still counts.

The [issue 177 record](../evidence/issue177/README.md) ran the checks on the M4 Mac (24 GiB). These facts are fixed:

- Five of six cases pass every check. The 43-segment and 199-segment cases pass check 1 after the change and fail it before.
- The log of 1,000 single-record callers, 331 segments and 23.5 GB, is 0.91 of RAM. After the timed runs, half of it was resident. Its random p50 fell from 5,053 µs to 216 µs, but its pre-change scan p50 is 44 µs, so check 1 needs 88 µs or less. It fails on p50 and passes on p99.
- Its group frames average about 160 KiB, more than 64 KiB, so after the change every frame is a kept entry. A random read is one direct read of one frame, as a hit in the old dense table was. No window is walked. A scan stays inside one cached frame for several calls, so cache misses barely touch it.
- A log of the same shape that fits in memory, 103 segments and 7.3 GB (0.28 of RAM, fully resident), passes every check: random p50 4,930 → 51 µs, p99 5,177 → 91 µs, scan unchanged at 44 µs and 86 µs.

No change to frame tables can make a read of a page that is not resident as fast as a scan of a cached frame. As written, check 1 can pass on this case only on a host where the log fits in memory.

## Options & trade-offs

### Option A - keep check 1 as written

- **For:** no change to a bar after the results came in.
- **Against:** the change cannot pass on the M4. It passes only on a host with enough memory for the log. Check 1 then measures the host's RAM and SSD, not the read path.

### Option B - judge check 1 on logs the page cache holds, and report the rest

- **For:** check 1 measures what frame tables can change. A log larger than the cache still runs, and its numbers and cache state are reported beside the verdict.
- **Against:** the bar changes after the results. A regression that shows only on logs larger than memory would not fail check 1. Checks 2 and 3 still run on those logs.

### Option C - drop the 1,000-caller case

- **For:** the smallest change to ADR 0069's text.
- **Against:** it loses the case with the most frames per segment at the default roll, the one the issue 156 record measured at 4–6 ms.

## Decision

We will build Option B. ADR 0069's checks stay as written, with one amendment to check 1:

- A case whose log is fully resident after its timed runs, a resident fraction of 0.99 or more, is judged on check 1 as written.
- A case whose log is not fully resident and passes check 1 passes.
- A case whose log is not fully resident and fails check 1 is reported, with its log bytes against RAM and its resident fraction, and is judged on check 1 by a log of the same shape that is fully resident: the same probe and build, with fewer segments. That smaller log must also fail check 1 before the change, under check 3.
- Checks 2 and 3 apply to every case as written, whether its log is resident or not.

The qualified-host rerun applies the same rule.

We chose this because check 1 exists to show that frame tables no longer cost a random read more than a scan. A page-cache miss is a cost of the host, which ADR 0069 already names as a reason a case can fail. We accept that a regression seen only on logs larger than memory is caught by checks 2 and 3, and by the reported numbers, not by check 1.

## Consequences

- Accepted by the owner on 2026-11-20 ([PR #185](https://github.com/ironflowx/segment-log/pull/185)).
- The issue 177 record passes on the M4. The 331-segment 1,000-caller log is reported as a cache miss beside the 103-segment log that judges check 1.
- `docs/evidence/issue177/summarize.py` reads several runs and applies this rule, so a rerun reproduces the verdict.
- ADR 0069 gains a bullet at the top of its Consequences that links this ADR.
- Random reads of logs larger than memory still cost an SSD read on a miss. That is a property of the host, outside the read path.

## Enforcement

`docs/evidence/issue177/summarize.py` applies the rule and prints which log judges check 1 for each case. Nothing in CI runs it; a reviewer of a later read-path record has to apply it.

## Revisit if

- A read-path change makes logs larger than memory slower while logs in memory stay within check 1.
- Deployments routinely read at random from logs larger than the host's memory. Then a bar for SSD reads needs its own ADR.
