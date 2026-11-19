# Issue 154: paced p99 on the EPYC host

The requalified host (EPYC 7401P, 2x PM983 EDA5502Q) missed the paced p99 bound in 5 of 5 runs at
82bb5b1 (p99 29.1 ms; docs/evidence/m7/requalified-host/m1-capture). This rerun measured three
commits with [run.sh](run.sh) on 2026-11-18.

## Host

Hetzner auction #3116374: EPYC 7502P, 128 GB, 2x Samsung PM983 MZQLB1T9HAJR EDA5502Q, write cache
write through, Debian 12 (6.1.0-54), the second NVMe as ext4 on /tmp. No 7401P was offered, so
the drive matches the requalified host and the CPU does not. Each commit's `host-extra.txt` has
the rest.

## Results ([epyc7502p](epyc7502p))

| Commit | Closed-loop records/s | Paced p99 ms | Paced runs within the bound |
|---|---|---|---|
| 82bb5b1 (the requalified capture) | 178,810 | 5.47 (1.69–7.18) | 2 of 5 |
| 87ea195 (#163, hash map) | 203,976 | 27.26 (25.19–28.35) | 0 of 5 |
| 49ee5c8 (#163 and #164) | 210,175 | 9.03 (8.35–9.43) | 0 of 5 |

Medians of 5 runs, with the range.

## What the groups show

The paced group traces give each group's submits and frame bytes. A group far above the median
of about 50 submits follows a writer stall. Counting frame bytes places each stall in its segment:

- At every roll (64 MiB), on all three commits: the closing run's writes and flushes. On 49ee5c8
  the traced runs (`8b1a2d1/traced-*/stall.txt`) time it at 8–9 ms on the writer, then 30–37 ms
  without a group write; about 20% of the slowest 1% of requests start in that window.
- At about 8, 16 and 32 MiB into every segment, on 87ea195 and 49ee5c8 only: the active key map
  doubling. A `HashMap` moves every key at once when it grows; the `BTreeMap` it replaced did
  not. 32 MiB of 292-byte records is 114,688 keys, where a std `HashMap` grows from 2^17 buckets.

The 14 commits from 82bb5b1 to 87ea195 change the write path only in #163; #147 and #149 touch
the core off it. #164 then took the log filter's build off the writer, as `SegmentLog::open` does,
which is why 49ee5c8 beats 87ea195.

## Fix

The roll sizes the next segment's map for as many keys as the rolled one, so a steady load never
grows it on the writer. The roll-time closing-run stall remains; moving its flushes off the group
path needs an ADR.

## Rerun with the fix ([epyc7502p-fix](epyc7502p-fix))

Hetzner auction #3116582, 2026-11-19: EPYC 7502P, 2x Samsung PM9A3 MZQL21T9HCJR GDC5A02Q (not the
PM983 of the first box), otherwise the same setup. Both commits ran on this box.

| Commit | Closed-loop records/s | Paced p99 ms | Paced runs within the bound | Mid-segment large groups per run |
|---|---|---|---|---|
| a0c13b1 (main) | 218,022 | 8.95 (8.40–9.16) | 0 of 5 | 13–14 |
| 8effa54 (main + the fix) | 219,213 | 4.14 (2.49–5.04) | 0 of 5 | 3 |

The bound is about 1.8 ms. The 3 large groups left in the middle of a segment fall in the first
segment after open, whose map has no rolled map to take its size from. The rest of the gap to the
bound is the closing run at each roll.

## The sealer writes the closing run ([ryzen3900-sealer](ryzen3900-sealer))

Hetzner auction #3116708, 2026-11-19: Ryzen 9 3900, 2x Toshiba KXD51RUE1T92 1CEE6111, write cache
write through, otherwise the same setup. No EPYC was offered. Both commits ran on this box.

| Commit | Closed-loop records/s | Paced p99 ms | Paced p999 ms | Paced max ms | Runs within the bound | Large groups at rolls per run |
|---|---|---|---|---|---|---|
| 2d4652b (main) | 312,232 | 1.36 (1.35–1.37) | 23.3 (22.9–23.5) | 28.9 (28.0–29.7) | 5 of 5 | 13–16 |
| 50c2125 (ADR 0067) | 315,150 | 1.35 (1.34–1.37) | 15.2 (14.7–15.9) | 21.1 (20.1–21.7) | 5 of 5 | 8–11 |

The bound is about 1.9 ms. On this faster CPU main already passes, because the roll stalls hold
fewer than 1% of the records, so p99 does not move. The stalls show in p999 and the maximum, which
fall by about a third. On main the traced runs time the writer's closing run at 7.9 ms per roll.
