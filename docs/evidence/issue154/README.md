# Issue 154: paced p99 on the EPYC host

The requalified host (EPYC 7401P, 2x PM983 EDA5502Q) missed the paced p99 bound in 5 of 5 runs at
386695f (p99 29.1 ms; docs/evidence/m7/requalified-host/m1-capture). This rerun measured three
commits with [run.sh](run.sh) on 2026-11-18.

## Host

Hetzner auction #3116374: EPYC 7502P, 128 GB, 2x Samsung PM983 MZQLB1T9HAJR EDA5502Q, write cache
write through, Debian 12 (6.1.0-54), the second NVMe as ext4 on /tmp. No 7401P was offered, so
the drive matches the requalified host and the CPU does not. Each commit's `host-extra.txt` has
the rest.

## Results ([epyc7502p](epyc7502p))

| Commit | Closed-loop records/s | Paced p99 ms | Paced runs within the bound |
|---|---|---|---|
| 386695f (the requalified capture) | 178,810 | 5.47 (1.69–7.18) | 2 of 5 |
| ab86546 (#163, hash map) | 203,976 | 27.26 (25.19–28.35) | 0 of 5 |
| 8b1a2d1 (#163 and #164) | 210,175 | 9.03 (8.35–9.43) | 0 of 5 |

Medians of 5 runs, with the range.

## What the groups show

The paced group traces give each group's submits and frame bytes. A group far above the median
of about 50 submits follows a writer stall. Counting frame bytes places each stall in its segment:

- At every roll (64 MiB), on all three commits: the closing run's writes and flushes. On 8b1a2d1
  the traced runs (`8b1a2d1/traced-*/stall.txt`) time it at 8–9 ms on the writer, then 30–37 ms
  without a group write; about 20% of the slowest 1% of requests start in that window.
- At about 8, 16 and 32 MiB into every segment, on ab86546 and 8b1a2d1 only: the active key map
  doubling. A `HashMap` moves every key at once when it grows; the `BTreeMap` it replaced did
  not. 32 MiB of 292-byte records is 114,688 keys, where a std `HashMap` grows from 2^17 buckets.

The 14 commits from 386695f to ab86546 change the write path only in #163; #147 and #149 touch
the core off it. #164 then took the log filter's build off the writer, as `SegmentLog::open` does,
which is why 8b1a2d1 beats ab86546.

## Fix

The roll sizes the next segment's map for as many keys as the rolled one, so a steady load never
grows it on the writer. The roll-time closing-run stall remains; moving its flushes off the group
path needs an ADR.
