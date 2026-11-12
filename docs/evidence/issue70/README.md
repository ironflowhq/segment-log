# What the key lookup costs as sealed history grows (issue 70)

Two captures of the [`m4-evidence` workflow](../m4/README.md) at `15dd21d`, on hosted ubuntu-24.04 runners. Each compares the M4 code ("after") with the M3 code at `0ffc772` ("before"). A hosted runner is a shared virtual machine. These are measurements under its conditions, not a target pass.

| Capture | Fixture | Run |
| --- | --- | --- |
| [`sealed-100`](sealed-100/summary.md) | The PRD fixture: 100 sealed segments at the 64 MiB default `roll_bytes`, 22.7 million keyed records | [43291174944](https://github.com/ironflowhq/segment-log/actions/runs/43291174944) |
| [`sealed-1000`](sealed-1000/summary.md) | A named variant ([ADR 0036](../../decisions/0036-measure-the-existing-defaults-before-tuning.md)): 1,000 sealed segments at `roll_bytes` 6,710,886, about the same number of keys | [43291173652](https://github.com/ironflowhq/segment-log/actions/runs/43291173652) |

The `closed-unkeyed` mode sends the same records without keys to the same keyed fixture, so the gap to the keyed closed loop on one side is what the key lookup costs on that side.

## Closed loop, 1,000 callers, medians of 5 runs

| Sealed segments | Side | Keyed records/s | Unkeyed records/s | Keyed minus unkeyed, µs per record |
| --- | --- | --- | --- | --- |
| 100 | M3 | 220,776 | 363,049 | 1.78 |
| 100 | M4 | 133,644 | 352,472 | 4.65 |
| 1,000 | M3 | 207,194 | 306,003 | 1.56 |
| 1,000 | M4 | 29,105 | 285,541 | 30.86 |

## What this shows

- **The lookup is the cost.** At 100 segments, unkeyed appends on M4 run at 0.971 of M3, and the ranges overlap. Keyed appends run at 0.605 of M3. The gap between M3 and M4 is in the key lookup.
- **The lookup cost grows with the number of sealed segments.** On M4 it goes from 4.65 µs per record at 100 segments to 30.86 µs at 1,000, with about the same number of keys. On M3, whose exact map does not depend on segments, it stays at about 1.6 to 1.8 µs.
- **At 1,000 segments, M4 misses the 50,000 records/s target.** The keyed closed loop reaches 29,105 records/s. The paced run at 50,000 records/s cannot keep up: it delivers 29,111 records/s, and its p50 latency is 7.0 s, because the queue grows for the whole run.
- **Without keys, M4 at 1,000 segments is a little slower than M3:** 0.933, and the ranges do not overlap. This cost is not in the lookup, and this capture does not show where it is.

[ADR 0051](../../decisions/0051-a-log-key-filter-in-memory-in-front-of-the-segment-filters.md) uses these numbers.
