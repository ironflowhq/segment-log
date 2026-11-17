# Qualification record: the ADR 0059 stack

**Status: not qualified, one MUST row open.** The stage 2 session ran every case on the qualified host on 2026-11-17 ([issue 112](https://github.com/ironflowx/segment-log/issues/112)), with `scripts/m7-session.sh` at `df23d9b` plus the `host-reset` event added to `scripts/m7-host-cut.sh` in the same change. Every row passes except host power loss: this server's Robot web service offers no power button reset, so the harness cannot cut the host's power. The stack qualifies when every MUST row is `Pass`, or `Not applicable` where V8 allows it ([volume-requirements.md](../../architecture/volume-requirements.md#qualification-record)). The session's logs and the host description are in [qualified-host/](qualified-host/README.md).

## Fingerprint

The machine-readable copy that the launcher and the guest check before every open (V10) is [`scripts/m7/fingerprint`](../../../scripts/m7/fingerprint). Host values come from the session's [host.txt](qualified-host/host.txt).

| Layer | Component and version | Configuration |
| --- | --- | --- |
| Guest kernel | Firecracker CI 6.1.155 (`uname -r` = `6.1.155+`) | `console=ttyS0 reboot=k panic=1`, no device-mapper |
| Guest filesystem | ext4, `mkfs.ext4` defaults | mounted `rw,relatime,errors=remount-ro` |
| Guest block device | virtio-blk `/dev/vdb` | `write_cache`: `write back` |
| VMM | Firecracker v1.18.0 | log drive `cache_type: Writeback`, one VMM per volume, `flock` on the image |
| Guest size | 2 vCPUs, 256 MiB (launcher default) | 1 GiB for the guest benchmark and the transport tests with 1,000 connections; 1 vCPU for the flush-latency guests ([sizing](handoff.md#guest-size)). Not in the fingerprint: size changes capacity, not durability |
| Host kernel | Debian 12, `6.1.0-54-amd64` (Debian `6.1.190-1`) | distribution defaults; `kernel.panic` reboots after a crash |
| Host filesystem | ext4 on the second NVMe drive (`nvme1n1`), whole device, no RAID | `rw,noatime`, default barriers |
| Backing file | raw sparse image per log, on the host filesystem above | created by the launcher |
| Media | 2× Samsung PM983 `MZQLB960HAJR-00007`, 960 GB U.2, firmware `EDA5502Q` (auction #3114789, Xeon E-2176G, 64 GB) | `vwc 0`: no volatile write cache, so the block layer reports `write through` and sends no flush to the drive. `awupf 7`, `nawupf 7` (4 KiB at 512-byte blocks), 512-byte LBA format in use |
| Planned density | 8 guests per host, 200 groups/s each, 16 KiB groups | **Provisional** (issue 112) |

## Failure domain (V7)

| Event | Declared | How the harness causes it |
| --- | --- | --- |
| Process crash | yes | `SIGKILL` the server in the guest |
| Guest crash | yes | `sysrq c` in the guest |
| Guest destroy | yes | `SIGKILL` the VMM |
| Host kernel crash | yes | `sysrq c` on the host, `kernel.panic` reboots it |
| Host reset | yes | Robot web service, `hw`: the reset button. The drives keep power ([evidence](qualified-host/README.md#host-reset-is-not-a-power-cut)), so this is never a power cut |
| Host power loss | yes | Robot web service: long press, then press, of the power button. **Not offered on this server** (`sw`, `hw`, `man` only), so not caused |
| Host loss | no | The image lives on one host's local drive |
| Device loss | no | One drive, no RAID |
| Zone loss | no | One host |

Which events beyond the minimum a product needs is [PRD open item 12](../../prd.md#open-items).

## Provisional values

These stand in for deployment numbers that are not set yet. Each is labeled where it is used.

- V8: the deployment never copies a log volume.
- V9: no workload profile or response time yet, so the sizing record and the alert thresholds are `Not verified`.
- V11: the density above.

## Requirements

Logs are in [qualified-host/](qualified-host/), one per step, named after the step; `results.tsv` has each step's exit status. Rehearsals in Lima or on the earlier host fill no row; they are noted under Limitations where they shaped the case.

| ID | Case | Evidence class | Commands or script | Control result | Result | Limitations |
| --- | --- | --- | --- | --- | --- | --- |
| V1 | `VOL-FLUSH-CONFIG` | Tested | `m7-rehearse.sh` (guest `write_cache`, mount options), `host.txt` (drive `vwc`) | `Unsafe` shows `write through` | Pass | Guest: `write back`, ext4 `rw,relatime,errors=remount-ro`. Host: ext4 `rw,noatime` with default barriers on a drive with no volatile cache (`vwc 0`), so the host sends no flush to the drive (0 flushes in every [latency](qualified-host/latency-density.log) run) |
| V1 | `VOL-FLUSH-TRACE` | Not verified | none | | Not verified | A trace alone never qualifies V1; `VOL-POWER-CUT` decides |
| V1 | `VOL-POWER-CUT`, guest-side events | Tested | `m7-power-cut.sh process-crash\|guest-crash\|guest-destroy 30` | `--control`, 10 cycles each: caught 10, 10 and 9 of 10 | Pass | 0 of 30 cycles lost a record for each event; the last checks covered 139,883, 143,756 and 127,719 reported records |
| V1 | `VOL-POWER-CUT`, host kernel crash | Tested | `m7-host-cut.sh host-kernel-crash 30` | `--control` (`Unsafe`): first cycle lost all 11,558 reported records | Pass | 0 of 30 cycles lost a record; 249,682 reported records checked |
| V1 | `VOL-POWER-CUT`, host reset | Tested | `m7-host-cut.sh host-reset 30` | `--control` (`Unsafe`): first cycle lost all 10,084 reported records | Pass | 0 of 30 cycles lost a record; 183,477 reported records checked. Not a power cut: the drives' `power_cycles` stayed below the session's 31 resets |
| V1 | `VOL-POWER-CUT`, host power loss | Tested | `m7-host-cut.sh host-power-loss 30` | not run | **Not verified** | The Robot API offers no power button reset for this server ([log](qualified-host/hc-host-power-loss.log)). Supporting only: the drive has no volatile write cache (`vwc 0`), and no step depends on a host flush reaching the media |
| V2 | `VOL-POWER-CUT-RECOVERY` | Tested | `m7-recovery-cut.sh clean\|sealed\|header 10` | `--control` (`rewrite-control` build): first cycle, 4 reported records missing | Pass | 0 of 10 counted cycles failed for each window, open never `Corrupt`. Cut by host kernel crash on block-trace evidence; dm-delay widens the window |
| V2 | `VOL-ATOMIC-WRITE` | Supporting | `host.txt` (`awupf`, `nawupf`, logical block size) | | `awupf 7`, `nawupf 7`: 4 KiB | Supporting only |
| V3 | `VOL-METADATA-ERROR` | Tested | `m7-metadata-error.sh`, `--control`, `--panic` | No control could be built (`errors=continue` also goes read-only), as expected | Pass | Under [ADR 0062](../../decisions/0062-v3-requires-refused-writes-not-failed-syncs.md); the failing target is a dm table on the host, under the guest's drive. `--panic` exits 1 (`fail`), as in Lima and ADR 0062: evidence only, since the stack mounts `errors=remount-ro` and ext4 panics only at the next write |
| V4 | `VOL-FS-CHECK` | Tested | `m7-rehearse.sh` (`/dev/vdb ext4`) | the same rule rejects `/dev/shm` (`tmpfs`) | Pass | |
| V5 | `VOL-EXCLUSIVE` | Tested | `m7-rehearse.sh` (steps 2, 3 and 4 on one host) | `M7_FENCE=0`: a second VMM attached the held volume | Pass | 20 of 20 races attached exactly one VMM. Step 1's read-only attach is the same `flock`; two hosts never share an image |
| V6 | `HANDOFF` | Tested | `m7-rehearse.sh` (`TestHandoffSealed`, `TestHandoffUnsealed`) | | Pass | [Handoff record](handoff.md) |
| V7 | `VOL-FAILURE-DOMAIN` | Tested | the V1 power-cut rows, one per declared event | as in V1 | **Not verified** | Every declared event passes except host power loss, which was not caused |
| V8 | `VOL-COPY` | Not applicable | none | | Not applicable | Provisional: the deployment never copies a log volume |
| V9 | `VOL-CAPACITY` | Not verified | none | | Not verified | Provisional: no workload profile or response time |
| V9 | `VOL-ENOSPC` | Tested | `m7-enospc.sh guest\|host` | | Pass | Guest full: 20,744 of 20,744 reported records survived; host full: 27,317 of 27,317 |
| V10 | `VOL-DRIFT` | Tested | `m7-drift.sh` (cache mode, mount option, guest kernel) | | Pass | Unchanged stack opened; `Unsafe`, `noatime` and guest kernel 5.10 each refused |
| V11 | `VOL-FLUSH-LATENCY` | Measured | `m7-flush-latency.sh 1 0 60`, `8 200 60`, `8 0 60` | | Measured, below | Density provisional |

### Flush latency (V11)

A guest `fsync` of 16 KiB on the log drive, through virtio-blk and the host filesystem. Medians across guests.

| Run | Guests | Rate each | syncs/s each | mean µs | p50 µs | p99 µs | p999 µs | max µs |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| [alone](qualified-host/latency-alone.log) | 1 | unpaced | 2,514 | 397 | 394 | 532 | 677 | 1,556 |
| [density](qualified-host/latency-density.log) | 8 | 200/s | 200 | 659 | 646 | 946 | 1,127 | 1,601–3,383 |
| [saturation](qualified-host/latency-saturation.log) | 8 | unpaced | 1,550 | 644 | 635 | 874 | 1,009 | 4,362–5,203 |

## Throughput estimate (V11)

`records/s ≈ callers × records per submit / (linger + mean cycle at planned density)`, with 1,000 callers, 1 record per submit and 1 ms linger ([PRD](../../prd.md#footprint-and-performance-targets)). With the density run's mean cycle of 0.66 ms: 1,000 / 1.66 ms ≈ 600,000 records/s. The flush is not the bound; the guest benchmark below is.

## Guest benchmark and #49

The guest benchmark ([summary](qualified-host/guest-bench/summary.md)) runs the M6 socket workload, 1,000 callers and 256-byte keyed records, against the server in a guest over hybrid vsock, paired with the same workload in process on the host's volume filesystem, 5 pairs each.

| Mode | In process | Firecracker guest |
| --- | --- | --- |
| Closed loop, records/s (median) | 261,793 | 46,187 |
| Closed loop, p99 ms | 7.5 | 33.5 |
| Paced at 50,000/s, achieved records/s | 49,972 | 46,014 |
| Paced, p99 ms | 1.4 | 1,731 |

**The guest topology misses the PRD's 50,000 records/s target by about 8 %.** It cannot reach the paced rate, so the paced latencies show a queue that grows for the whole run, not the log's latency. The bound is the VMM's main thread: in [the thread sample](qualified-host/guest-bench/closed/guest-1.threads.txt) the `firecracker` thread runs at 99.9 % CPU while the two `fc_vcpu` threads run at 87–93 %. That thread runs Firecracker's vsock muxer, so the bound is the hybrid vsock transport, not the log, the drive or the guest's CPU. The in-process run on the same volume does 5.7× the guest's rate.

The issue 49 capture on this host ([summary](qualified-host/m1-capture/summary.md)), with `ROLL writer_ns` per roll ([ADR 0061](../../decisions/0061-a-sealer-thread-builds-the-key-frames-off-the-writer.md)):

- Closed loop: 258,188 records/s median, p99 7.4 ms, device flush p99 0.52 ms.
- Paced at 50,000 records/s: p99 1.47 ms median, within the bound of the same run (2.03 ms) in 5 of 5 runs, and within the bound from the run's own group flushes in 3 of 5.
- Every roll keeps the writer for 0.27–0.49 ms (`ROLL writer_ns`), against the ~70 ms roll stall that issue 107 found before ADR 0061.
