# M4 completion record

M4 (sealed footer and locator, `seal`, read-only open, `LogReader`, retained-key lookup) passed on 2026-11-11. This record links the evidence for each part of the [M4 gate](../../prd.md#milestones). [ADR 0050](../../decisions/0050-m4-open-time-gate-uses-a-warm-page-cache.md) sets the open-time gate to a warm page cache.

| Gate part | Evidence |
| --- | --- |
| Open time for M4 conditions meets the target. | [M4 evidence run 43128582588](https://github.com/ironflowhq/segment-log/actions/runs/43128582588) at `64975e0` on a hosted ubuntu-24.04 runner. Keyed open p50 is 38.6 ms warm (target 50 ms) and 122.8 ms cold. With no keys, the fixture opens in 18.1 ms warm and 81.4 ms cold. [`linux/summary.md`](linux/summary.md) gives every run. |
| From each sealed segment except the newest, open reads only the header, the locator, the footer and the key filter, counted through the storage trait. | The `ORACLE-13` scenario in `segment-log-core/src/acceptance/summary_open.rs` checks the exact read list per sealed segment. |
| Retained-key retry and cache-rebuild tests pass, and key-cache memory matches the formula in ADR 0048. | The M4-SEAL-READ and M4-LOOKUP-OPEN scenarios in [testing.md](../../architecture/testing.md#milestone-acceptance-coverage), including `KEY-LIFETIME`, `KEY-COLLISION`, `KEY-FRAMES-CRASH`, `ORACLE-12` and `KEY-MEMORY`. CI is green on Ubuntu and macOS at the top of the stack, [run 43128718311](https://github.com/ironflowhq/segment-log/actions/runs/43128718311), including the reruns with `roll_bytes` at 65 and 256 bytes. |
| A Linux report gives open time cold and warm, key-cache memory, and the effect on append throughput and p99 latency. | [`linux/summary.md`](linux/summary.md), below. |

## Linux report

The fixture holds 22,715,000 keyed records: 100 sealed segments at the default 64 MiB roll size (227,000 records each) and a 4.23 MiB active segment. [`linux/environment.txt`](linux/environment.txt) records the runner (4 vCPUs, AMD EPYC 7763), the kernel, the mounts and the toolchain. [`linux/binaries.sha256`](linux/binaries.sha256) identifies both binaries. A hosted runner is a shared virtual machine. These are measurements under its conditions.

| Measure | M4 | M3 (`0ffc772`) |
| --- | --- | --- |
| Open, keyed, warm p50 | 38.6 ms | about 32 s, full scan |
| Open, keyed, cold p50 | 122.8 ms | not measured |
| Key-cache memory after open | 47.2 MB heap (filters 46.1 MB by the ADR 0048 formula, plus the fences and the 15,000-key active map) | about 2.5 GB resident |
| Closed-loop keyed appends, 1,000 callers | 109,100 records/s, p99 14.3 ms | 201,800 records/s, p99 9.4 ms |
| Paced keyed appends at 50,000 records/s | p50 1.24 ms, p99 37.4 ms | p50 1.21 ms, p99 46.3 ms |

The closed-loop append rate meets the 50,000 records/s target, but it is about half of M3. Each new key probes up to 100 key filters in memory, and about 9% of new keys read a key index page because of a filter false positive. A log-wide filter in front of the per-segment filters is the next option to measure ([ADR 0048](../../decisions/0048-sealed-segments-carry-a-key-filter-and-index.md), option D).

[README.md](README.md) explains how to reproduce the run.
