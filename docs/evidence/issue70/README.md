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

## With the log key filter (ADR 0051)

Four more captures measure the log key filter of pull request 75 beside the same M3 code.

| Capture | Wait after open | Run |
| --- | --- | --- |
| [`log-filter-100`](log-filter-100/summary.md) | none | [43300711775](https://github.com/ironflowhq/segment-log/actions/runs/43300711775), at `2c3bfcc` |
| [`log-filter-1000`](log-filter-1000/summary.md) | none | [43300699281](https://github.com/ironflowhq/segment-log/actions/runs/43300699281), at `2c3bfcc` |
| [`log-filter-100-settled`](log-filter-100-settled/summary.md) | 15 s | [43305119312](https://github.com/ironflowhq/segment-log/actions/runs/43305119312), at `4744340` |
| [`log-filter-1000-settled`](log-filter-1000-settled/summary.md) | 15 s | [43305124391](https://github.com/ironflowhq/segment-log/actions/runs/43305124391), at `4744340` |

`4744340` changes only the probe and the scripts: `SEALED_PROBE_SETTLE_MS` waits between open and the first append on both sides, so that the builder has finished before the timed run.

Closed loop, 1,000 callers, medians of 5 runs:

| Sealed segments | Wait | M4 keyed records/s | M4 keyed / M3 keyed | M4 unkeyed / M3 unkeyed | Lookup cost, M3 | Lookup cost, M4 |
| --- | --- | --- | --- | --- | --- | --- |
| 100 | none | 162,564 | 0.848 | 0.842 | 1.79 µs | 2.08 µs |
| 1,000 | none | 154,828 | 0.779 | 0.817 | 1.66 µs | 2.33 µs |
| 100 | 15 s | 141,987 | 0.896 (ranges overlap) | 0.926 (ranges overlap) | 2.13 µs | 2.52 µs |
| 1,000 | 15 s | 230,447 | 0.933 (ranges overlap) | 0.866 | 1.31 µs | 1.18 µs |

The lookup cost is keyed minus unkeyed time per record on the same side. The runners differ from capture to capture, so compare the sides within one capture, not records/s across captures.

What this shows:

- **The lookup cost no longer grows with history.** Before the filter it was 4.65 µs per record at 100 sealed segments and 30.86 µs at 1,000. With the filter, it is 2.1 to 2.5 µs at 100 and 1.2 to 2.3 µs at 1,000, close to M3's exact map in the same captures.
- **The builder slows the first seconds after open.** Without the wait, unkeyed appends run at 0.82 to 0.84 of M3, and keyed p99 at 1,000 segments is 31.4 ms against 9.8 ms. With the wait, unkeyed appends run at 0.93 of M3 at 100 segments, and closed-loop p99 at 100 segments matches M3. The builder reads about 16 bytes per sealed key and competes for the runner's 4 processors. Until it covers a segment, lookups probe that segment's filter as before. These captures do not measure how long the build takes.
- **Keyed against unkeyed is not a measure of the lookup alone.** M3, with an exact map, gives keyed/unkeyed of 0.66 to 0.68. That gap also holds the 16 bytes of key per record, the active segment's key map, and, on M4, the key index that each roll writes.
- **Two costs remain that the lookup does not explain.** With the wait, unkeyed M4 at 1,000 segments runs at 0.866 of M3, and the ranges do not overlap. Keyed closed-loop p99 at 1,000 segments is 10.7 ms against 6.3 ms. The capture before the filter also showed unkeyed M4 behind M3 at 1,000 segments (0.933). Paced p99 at 100 segments is 66.1 ms against 52.5 ms; at 1,000 segments it is 5.0 ms against 18.9 ms. Paced p99 on a hosted runner follows rare slow flushes, so read it with its range.
