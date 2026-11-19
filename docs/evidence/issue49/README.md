# Default options on a physical Linux local-NVMe host (issue 49)

[Issue 49](https://github.com/ironflowx/segment-log/issues/49) asks for the M1 capture on bare metal with local NVMe, because every Linux host measured before was a virtual machine. This record runs `scripts/m1-evidence-guest.sh` unchanged on one dedicated host, on the current `main` and on the M1 record's commit, so that the two are compared on the same machine.

## Host

A Hetzner server-auction machine in FSN1, used on 2026-11-16 and cancelled the same day.

- Intel Core i7-7700 (4 cores, 8 threads), 64 GB DDR4. Debian 12, kernel 6.1.0-54-amd64 ([host.txt](https://github.com/ironflowx/segment-log-evidence/blob/dc1a230a4b0e7bd70fec1963473ddd7cb2733c1c/issue49/host.txt)).
- Two Samsung PM981 `MZVLB512HAJQ-00000` drives, firmware `EXA7301Q`. They are client drives with no power-loss protection, and the volatile write cache is on (`vwc 0x1`, `write_cache: write back`). Each flush goes to the flash. 29,235 power-on hours, 26 % used.
- The OS is on `nvme0n1`. The data directory is on `nvme1n1`: ext4, `rw,noatime`, one drive, no software RAID ([host-extra.txt](https://github.com/ironflowx/segment-log-evidence/blob/dc1a230a4b0e7bd70fec1963473ddd7cb2733c1c/issue49/main-f4cdbea/host-extra.txt) has `/proc/mdstat` and `nvme id-ctrl` for both).
- Nothing else ran on the host. The script checks the load before it measures.

[run.sh](run.sh) is the exact driver. Each run directory holds the script's full output: [main-95580e9](main-f4cdbea/summary.md) and [8fd09b2](ff74647/summary.md).

## Result

| | `8fd09b2` (M1 record) | `main` `95580e9` | PRD target |
| --- | ---: | ---: | --- |
| Closed loop, median records/s | 61,127 | 62,203 | ≥ 50,000: **met on both** |
| Closed loop append p999, median | 25.7 ms | 126.1 ms | none |
| Device flush p99, median | 6.74 ms | 6.81 ms | |
| Paced p99 at 50,000 records/s, median | 14.44 ms | 83.13 ms | ≤ linger + 2 × flush p99 |
| Paced bound of the same run, median | 14.49 ms | 14.65 ms | |
| Paced runs within their bound | 3 of 5 | **0 of 5** | |
| Reopen, median | 710.5 ms | 72.5 ms | |

**Throughput meets the target on bare metal** on both commits, with the same device flush times.

**On `8fd09b2`, the paced p99 sits at the bound.** Three of five runs are within it, and the misses are 0.26 ms and 0.73 ms. Measured against the bound computed from each run's own group flushes instead of the device benchmark, all five miss, by 1.0–2.1 ms. So the hosted runner's miss in the M1 record came mostly from its slow flushes, and on this drive the target is met only narrowly.

**On `main`, the paced p99 misses by about 70 ms, and every miss comes from segment roll.** In every paced run, the slowest groups take all 1,000 callers at once, and they land at 64.3, 128.4, 192.4 and 256.4 MiB written: each `roll_bytes` boundary. The storage calls traced at a roll take about 5 ms in all (`Roll.SyncData` 8 calls, 34.5 ms per run), so the stall is work on the writer thread outside storage I/O, not the device: sealing the segment, which builds its key filter and key index ([ADR 0048](../../decisions/0048-sealed-segments-carry-a-key-filter-and-index.md)). The closed-loop p999 rises for the same reason. This is a finding against the latency target on `main`, filed as [issue 107](https://github.com/ironflowx/segment-log/issues/107). The target is not changed.

**The two commits do not measure the same work.** `8fd09b2` has no roll: it predates M3, and its run writes one ~300 MB segment. So it shows the log without sealing, not an earlier version of the same code path. Its largest group has 578 submits. The faster reopen on `main` comes from the same difference: format version 2 and the key index let open skip sealed segments' records.

## Limits

- One host, one drive model, one day. A datacenter drive with power-loss protection flushes faster, so it gives a tighter bound and a different margin.
- The run is the M1 workload: 256-byte payloads, 1,000 callers, 1 ms linger. Larger workloads are issue 47.
- This is not a storage qualification of the deployment stack (issue 46, M7).
