# Qualification record: the ADR 0059 stack

**Status: qualified. Every MUST row is `Pass`, and V8 is `Not applicable`, on one host.** The requalification session ran every case on Hetzner auction host #3115170 on 2026-11-17 and 2026-11-18 ([issue 112](https://github.com/ironflowx/segment-log/issues/112), [issue 133](https://github.com/ironflowx/segment-log/issues/133)), with `scripts/m7-session.sh` at `386695f`, format version 3 and protocol version 2. Its logs and the host description are in [requalified-host/](requalified-host/README.md). The stack qualifies when every MUST row is `Pass`, or `Not applicable` where V8 allows it ([volume-requirements.md](../../architecture/volume-requirements.md#qualification-record)). Two earlier sessions fill no row: the [first qualified host](qualified-host/README.md) offered no power button reset, and the [power-loss host](power-loss-host/README.md) was a second machine, which [ADR 0059](../../decisions/0059-m7-qualifies-firecracker-on-a-dedicated-nvme-host.md)'s amendment forbids.

## Fingerprint

The machine-readable copy that the launcher and the guest check before every open (V10) is [`scripts/m7/fingerprint`](../../../scripts/m7/fingerprint), committed from this session's pinned copy. The launcher checks `vmm.*` and `host.*` before boot; the guest checks `guest.*` before the log opens. Host values come from [host.txt](requalified-host/host.txt) and [`scripts/m7/fingerprint-host`](../../../scripts/m7/fingerprint-host).

| Layer | Component and version | Configuration |
| --- | --- | --- |
| Guest kernel | Firecracker CI 6.1.155 (`uname -r` = `6.1.155+`) | `console=ttyS0 reboot=k panic=1`, no device-mapper |
| Guest filesystem | ext4, `mkfs.ext4` defaults | mounted `rw,relatime,errors=remount-ro` |
| Guest block device | virtio-blk `/dev/vdb` | `write_cache`: `write back` |
| VMM | Firecracker v1.18.0 | log drive `cache_type: Writeback`, one VMM per volume, `flock` on the image |
| Guest size | 2 vCPUs, 256 MiB (launcher default) | 1 GiB for the guest benchmark and the transport tests with 1,000 connections; 1 vCPU for the flush-latency guests ([sizing](handoff.md#guest-size)). Not in the fingerprint: size changes capacity, not durability |
| Host kernel | Debian 12, `6.1.0-54-amd64` (`host.kernel`) | distribution defaults; `kernel.panic` reboots after a crash |
| Host filesystem | ext4 on the NVMe drive that does not hold `/`, whole device, no RAID | `rw,noatime`, default barriers (`host.fs`) |
| Backing file | raw sparse image per log, on the host filesystem above | created by the launcher |
| Media | 2× Samsung PM983 `MZQLB960HAJR-00007`, 960 GB U.2, firmware `EDA5502Q` (`host.drive`); auction #3115170, AMD EPYC 7401P, 128 GB | `vwc 0`: no volatile write cache, so the block layer reports `write through` (`host.write_cache`) and sends no flush to the drive. `awupf 7`, `nawupf 7` (4 KiB at 512-byte blocks), 512-byte LBA format in use |
| Planned density | 8 guests per host, 200 groups/s each, 16 KiB groups | **Provisional** (issue 112) |

## Failure domain (V7)

| Event | Declared | How the harness causes it |
| --- | --- | --- |
| Process crash | yes | `SIGKILL` the server in the guest |
| Guest crash | yes | `sysrq c` in the guest |
| Guest destroy | yes | `SIGKILL` the VMM |
| Host kernel crash | yes | `sysrq c` on the host, `kernel.panic` reboots it |
| Host reset | yes | Robot web service, `hw`: the reset button. The drives keep power, so this is never a power cut |
| Host power loss | yes | Robot web service: long press, then press, of the power button. Each cut removed drive power: both drives' SMART `power_cycles` rose by exactly the number of cuts ([before](requalified-host/smart-before.txt), [after](requalified-host/host.txt)) |
| Host loss | no | The image lives on one host's local drive |
| Device loss | no | One drive, no RAID |
| Zone loss | no | One host |

Which events beyond the minimum a product needs is [PRD open item 12](../../prd.md#open-items).

## Provisional values

These stand in for deployment numbers that are not set yet. Each is labeled where it is used.

- V8: the deployment never copies a log volume.
- V9, the workload profile (owner, issue 133): 8 logs per host; a peak write rate per log of the density run's 200 groups/s × 16 KiB = 3,276,800 bytes/s; a planned peak history per log of 50 GiB encoded; 256-byte payloads, 16-byte stream ids and 16-byte keys on every record, as many records per group as fit in 16 KiB (53), and one stream per record (the footer's worst case); a response time of 1 hour; 20 % headroom. These are the defaults of [`scripts/m7-capacity.py`](../../../scripts/m7-capacity.py), which takes every width from [format.md](../../architecture/format.md).
- V9, what the profile gives on this host (format version 3): 146,335,728 records per log; a planned allocated size of 64.4 GB per log at the measured allocation ratio; an alert threshold of 11.8 GB free in each guest and 94.4 GB free on the host filesystem; a 82.7 GB image per log; 755.8 GB needed on the host filesystem, which has 895.9 GB ([capacity.log](requalified-host/capacity.log)).
- V11: the density above.

## Requirements

Logs are in [requalified-host/](requalified-host/), one per step, named after the step; `results.tsv` has each step's exit status.

| ID | Case | Evidence class | Commands or script | Control result | Result | Limitations |
| --- | --- | --- | --- | --- | --- | --- |
| V1 | `VOL-FLUSH-CONFIG` | Tested | `m7-rehearse.sh` (guest `write_cache`, mount options), `host.txt` (drive `vwc`) | `Unsafe` shows `write through` | Pass | Guest: `write back`, ext4 `rw,relatime,errors=remount-ro`. Host: ext4 `rw,noatime` with default barriers on a drive with no volatile cache (`vwc 0`), so the host sends no flush to the drive (0 flushes in every [latency](requalified-host/latency-density.log) run) |
| V1 | `VOL-FLUSH-TRACE` | Not verified | none | | Not verified | A trace alone never qualifies V1; `VOL-POWER-CUT` decides |
| V1 | `VOL-POWER-CUT`, guest-side events | Tested | `m7-power-cut.sh process-crash\|guest-crash\|guest-destroy 30` | `--control`, 10 cycles each: caught 9, 9 and 7 of 10 | Pass | 0 of 30 cycles lost a record for each event; the last checks covered 151,924, 155,164 and 141,349 reported records |
| V1 | `VOL-POWER-CUT`, host kernel crash | Tested | `m7-host-cut.sh host-kernel-crash 30` | `--control` (`Unsafe`): first cycle lost all 11,451 reported records | Pass | 0 of 30 cycles lost a record; 257,717 reported records checked |
| V1 | `VOL-POWER-CUT`, host reset | Tested | `m7-host-cut.sh host-reset 30` | `--control` (`Unsafe`): first cycle lost all 12,298 reported records | Pass | 0 of 30 cycles lost a record; 322,595 reported records checked |
| V1 | `VOL-POWER-CUT`, host power loss | Tested | `m7-host-cut.sh host-power-loss 30` | `--control` (`Unsafe`): first cycle lost all 19,167 reported records | Pass | 0 of 30 cycles lost a record; 576,495 reported records checked. Both drives' `power_cycles` went from 16 to 47 and `unsafe_shutdowns` from 2 to 33: one per cut, 30 real and 1 control |
| V2 | `VOL-POWER-CUT-RECOVERY` | Tested | `m7-recovery-cut.sh clean\|sealed\|header 10` | `--control` (`rewrite-control` build): first cycle, 5 reported records missing | Pass | 0 of 10 counted cycles failed for each window, open never `Corrupt`; the `clean` check covered 50,948 reported records. `clean` is from its [rerun](requalified-host/v2-clean-rerun.log): the first run stopped at cycle 10 when the ssh session to the host timed out before the check ran, after 9 clean cycles ([details](requalified-host/README.md#the-first-v2-clean-run)). Cut by host kernel crash on block-trace evidence; dm-delay widens the window |
| V2 | `VOL-ATOMIC-WRITE` | Supporting | `host.txt` (`awupf`, `nawupf`, logical block size) | | `awupf 7`, `nawupf 7`: 4 KiB | Supporting only |
| V3 | `VOL-METADATA-ERROR` | Tested | `m7-metadata-error.sh`, `--control`, `--panic` | No control could be built (`errors=continue` also goes read-only), as expected | Pass | Under [ADR 0062](../../decisions/0062-v3-requires-refused-writes-not-failed-syncs.md); the failing target is a dm table on the host, under the guest's drive. `--panic` exits 1 (`fail`), as in every earlier run and ADR 0062: evidence only, since the stack mounts `errors=remount-ro` |
| V4 | `VOL-FS-CHECK` | Tested | `m7-rehearse.sh` (`/dev/vdb ext4`) | the same rule rejects `/dev/shm` (`tmpfs`) | Pass | |
| V5 | `VOL-EXCLUSIVE` | Tested | `m7-rehearse.sh` (steps 2, 3 and 4 on one host) | `M7_FENCE=0`: a second VMM attaches the held volume | Pass | 20 of 20 races attached exactly one VMM. Step 1's read-only attach is the same `flock`; two hosts never share an image |
| V6 | `HANDOFF` | Tested | `m7-rehearse.sh` (`TestHandoffSealed`, `TestHandoffUnsealed`) | | Pass | [Handoff record](handoff.md) |
| V7 | `VOL-FAILURE-DOMAIN` | Tested | the V1 power-cut rows, one per declared event | as in V1 | Pass | Every declared event passes on this host |
| V8 | `VOL-COPY` | Not applicable | none | | Not applicable | Provisional: the deployment never copies a log volume |
| V9 | `VOL-CAPACITY` | Tested | `m7-capacity.sh` | | Pass | Guest: `df` shows 76.6 GB free after `mkfs` of the 82.7 GB image, against 76.2 GB needed. Host: 895.9 GB capacity against 755.8 GB needed for 8 images and the alert threshold. Both alerts fired at their (scaled) thresholds in the fill tests. Provisional profile |
| V9 | `VOL-ENOSPC` | Tested | `m7-enospc.sh guest\|host` | | Pass | Both layers: every acknowledged record survived, and open succeeded |
| V10 | `VOL-DRIFT` | Tested | `m7-drift.sh` (cache mode, host mount option, host drive, guest mount option, guest kernel) | the unchanged stack opens with every key checked | Pass | Refused: `vmm.cache_type` `Unsafe`, `host.fs` with `noexec`, `host.drive` naming another drive, guest `noatime`, guest kernel 5.10 |
| V11 | `VOL-FLUSH-LATENCY` | Measured | `m7-flush-latency.sh 1 0 60`, `8 200 60`, `8 0 60` | | Measured, below | Density provisional |

### Flush latency (V11)

A guest `fsync` of 16 KiB on the log drive, through virtio-blk and the host filesystem. Medians across guests.

| Run | Guests | Rate each | syncs/s each | mean µs | p50 µs | p99 µs | p999 µs | max µs |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| [alone](requalified-host/latency-alone.log) | 1 | unpaced | 1,479 | 675 | 658 | 898 | 999 | 2,103 |
| [density](requalified-host/latency-density.log) | 8 | 200/s | 200 | 1,626 | 1,599 | 2,104 | 2,497 | 2,813–3,925 |
| [saturation](requalified-host/latency-saturation.log) | 8 | unpaced | 1,184 | 844 | 771 | 1,486 | 1,726 | 2,309–13,019 |

## Throughput estimate (V11)

`records/s ≈ callers × records per submit / (linger + mean cycle at planned density)`, with 1,000 callers, 1 record per submit and 1 ms linger ([PRD](../../prd.md#footprint-and-performance-targets)). With the density run's mean cycle of 1.63 ms: 1,000 / 2.63 ms ≈ 380,000 records/s. The flush is not the bound; the guest benchmark below is.

## Guest benchmark and #49

The guest benchmark ([summary](requalified-host/guest-bench/summary.md)) runs the M6 socket workload, 1,000 callers and 256-byte keyed records, against the server in a guest over hybrid vsock, with the callers sharing 16 pipelined connections ([ADR 0063](../../decisions/0063-the-guest-benchmark-shares-pipelined-connections.md)), paired with the same workload in process on the host's volume filesystem, 5 pairs each. The [one-connection-per-caller diagnostic](requalified-host/guest-bench-per-caller/summary.md) runs the old topology.

| Median of 5 pairs | In process | Guest, 16 connections | Guest, 1,000 connections |
| --- | ---: | ---: | ---: |
| Closed loop, records/s | 130,665 | **52,860** (51,631 to 53,338) | 32,439 |
| Closed loop, p99 ms | 15.3 | 34.0 | 48.1 |
| Paced at 50,000/s, achieved records/s | 49,794 | 49,809 | 32,377 |
| Paced, p99 ms | 3.2 | 69.2 | 10,724 |

**The guest meets the PRD's 50,000 records/s target in every pair**, by about 6 % at the median. The bound is still the VMM's main thread, which runs Firecracker's vsock muxer: in [the thread sample](requalified-host/guest-bench/closed/guest-1.threads.txt) the `firecracker` thread runs at 99.9 % CPU and the two `fc_vcpu` threads at 63–69 %. **The guest misses the latency target at 50,000/s**: it runs at about 94 % of its closed-loop rate there, so the paced p99 of 69 ms is queueing near saturation. ADR 0063 records this as a known 0.1 limit, together with the Go client's lack of connection pooling ([issue 134](https://github.com/ironflowx/segment-log/issues/134)).

The issue 49 capture on this host ([summary](requalified-host/m1-capture/summary.md)), with `ROLL writer_ns` per roll ([ADR 0061](../../decisions/0061-a-sealer-thread-builds-the-key-frames-off-the-writer.md)):

- Closed loop: 131,329 records/s median, append p99 14.9 ms, device flush p99 0.45 ms.
- Paced at 50,000 records/s: p50 1.02 ms, but p99 29.1 ms median against a bound of 1.94 ms, so 0 of 5 runs are within the bound. The tail is long (p999 about 74 ms, max about 88 ms). Rolls are not the cause: every roll keeps the writer for 0.42–0.80 ms. The first qualified host (Xeon E-2176G) met the bound in 5 of 5 runs with the same build family, so this is a finding about the core's tail on this CPU ([issue 154](https://github.com/ironflowx/segment-log/issues/154)). Latency is a measured target, not a qualification row.
