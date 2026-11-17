# Qualification record: the ADR 0059 stack

**Status: draft.** The layout, the cases and the commands are final. The results are `Pending` until the stage 2 session runs them on the qualified host ([issue 112](https://github.com/ironflowx/segment-log/issues/112)), with `scripts/m7-session.sh`. The stack qualifies when every MUST row is `Pass`, or `Not applicable` where V8 allows it ([volume-requirements.md](../../architecture/volume-requirements.md#qualification-record)).

## Fingerprint

The machine-readable copy that the launcher and the guest check before every open (V10) is [`scripts/m7/fingerprint`](../../../scripts/m7/fingerprint). Host values are filled from the session's `host.txt`.

| Layer | Component and version | Configuration |
| --- | --- | --- |
| Guest kernel | Firecracker CI 6.1.155 (`uname -r` = `6.1.155+`) | `console=ttyS0 reboot=k panic=1`, no device-mapper |
| Guest filesystem | ext4, `mkfs.ext4` defaults | mounted `rw,relatime,errors=remount-ro` |
| Guest block device | virtio-blk `/dev/vdb` | `write_cache`: `write back` |
| VMM | Firecracker v1.18.0 | log drive `cache_type: Writeback`, one VMM per volume, `flock` on the image |
| Guest size | 2 vCPUs, 256 MiB (launcher default) | 1 GiB for the guest benchmark and the transport tests with 1,000 connections; 1 vCPU for the flush-latency guests ([sizing](handoff.md#guest-size)). Not in the fingerprint: size changes capacity, not durability |
| Host kernel | Pending (Debian 12) | Pending |
| Host filesystem | ext4 on the second NVMe drive, whole device, no RAID | Pending (`noatime`) |
| Backing file | raw sparse image per log | Pending |
| Media | Pending: 2× U.2 NVMe 960 GB "Datacenter" (auction #3114789) | Pending: model, firmware, `vwc`, power-loss protection, `awupf`, `nawupf` |
| Planned density | 8 guests per host, 200 groups/s each, 16 KiB groups | **Provisional** (issue 112) |

## Failure domain (V7)

| Event | Declared | How the harness causes it |
| --- | --- | --- |
| Process crash | yes | `SIGKILL` the server in the guest |
| Guest crash | yes | `sysrq c` in the guest |
| Guest destroy | yes | `SIGKILL` the VMM |
| Host kernel crash | yes | `sysrq c` on the host, `kernel.panic` reboots it |
| Host power loss | yes | Robot web service: long press, then press, of the power button. Counts as a power cut only with evidence that it removes power from the drives |
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

Logs are in the session output (`M7_OUT`), one per step, named after the step. Rehearsals in Lima or on the earlier host fill no row; they are noted under Limitations where they shaped the case.

| ID | Case | Evidence class | Commands or script | Control result | Result | Limitations |
| --- | --- | --- | --- | --- | --- | --- |
| V1 | `VOL-FLUSH-CONFIG` | Tested | `m7-rehearse.sh` (guest `write_cache`, mount options), `host.txt` (drive `vwc`) | `Unsafe` shows `write through` | Pending | Pending: the backing file's host filesystem and barriers |
| V1 | `VOL-FLUSH-TRACE` | Not verified | none | | Not verified | A trace alone never qualifies V1; `VOL-POWER-CUT` decides |
| V1 | `VOL-POWER-CUT`, guest-side events | Tested | `m7-power-cut.sh process-crash\|guest-crash\|guest-destroy 30` | `--control`, 10 cycles each | Pending | Earlier host (consumer NVMe): 0 lost in 3×30, controls caught 10/10 |
| V1 | `VOL-POWER-CUT`, host events | Tested | `m7-host-cut.sh host-kernel-crash\|host-power-loss 30` | `--control` (`Unsafe`) | Pending | Power loss counts only with evidence that the Robot reset removes drive power |
| V2 | `VOL-POWER-CUT-RECOVERY` | Tested | `m7-recovery-cut.sh clean\|sealed\|header 10` | `--control` (`rewrite-control` build) | Pending | Cut by host kernel crash on block-trace evidence; dm-delay widens the window |
| V2 | `VOL-ATOMIC-WRITE` | Supporting | `host.txt` (`awupf`, `nawupf`, logical block size) | | Pending | Supporting only |
| V3 | `VOL-METADATA-ERROR` | Tested | `m7-metadata-error.sh`, `--control`, `--panic` | Expected: no control can be built (`errors=continue` also goes read-only) | Pending | Under [ADR 0062](../../decisions/0062-v3-requires-refused-writes-not-failed-syncs.md); the failing target is a dm table on the host, under the guest's drive. `--panic` is evidence only: the stack mounts `errors=remount-ro`, so V3's `errors=panic` procedure does not apply, and ext4 panics only at the next write, after step 4's `fsync` calls return (Lima: `fail`, as ADR 0062 records) |
| V4 | `VOL-FS-CHECK` | Tested | `m7-rehearse.sh` (`/dev/vdb ext4`) | the same rule rejects `/dev/shm` (`tmpfs`) | Pending | Lima: pass, control rejected `tmpfs` |
| V5 | `VOL-EXCLUSIVE` | Tested | `m7-rehearse.sh` (steps 2, 3 and 4 on one host) | `M7_FENCE=0`: a second VMM attaches the held volume | Pending | Step 1's read-only attach is the same `flock`; two hosts never share an image. Lima: steps pass, 20 of 20 races, control attached |
| V6 | `HANDOFF` | Tested | `m7-rehearse.sh` (`TestHandoffSealed`, `TestHandoffUnsealed`) | | Pending | [Handoff record](handoff.md) |
| V7 | `VOL-FAILURE-DOMAIN` | Tested | the V1 power-cut rows, one per declared event | as in V1 | Pending | |
| V8 | `VOL-COPY` | Not applicable | none | | Not applicable | Provisional: the deployment never copies a log volume |
| V9 | `VOL-CAPACITY` | Not verified | none | | Not verified | Provisional: no workload profile or response time |
| V9 | `VOL-ENOSPC` | Tested | `m7-enospc.sh guest\|host` | | Pending | Lima: both layers passed |
| V10 | `VOL-DRIFT` | Tested | `m7-drift.sh` (cache mode, mount option, guest kernel) | | Pending | Lima: all four cases passed |
| V11 | `VOL-FLUSH-LATENCY` | Measured | `m7-flush-latency.sh 1 0 60`, `8 200 60`, `8 0 60` | | Pending | Density provisional |

## Throughput estimate (V11)

`records/s ≈ callers × records per submit / (linger + mean cycle at planned density)`, with 1,000 callers, 1 record per submit and 1 ms linger ([PRD](../../prd.md#footprint-and-performance-targets)). Pending the density run's mean cycle.

## Guest benchmark and #49

Pending. The guest benchmark is `scripts/m7-guest-bench.sh` (`m7-session.sh run guest-bench`): the M6 socket workload, 1,000 callers and 256-byte keyed records, against the server in a guest over hybrid vsock, paired with the same workload in process on the host's volume filesystem, closed loop and paced at 50,000 records/s, 5 pairs each. The issue 49 capture on this host, with `ROLL writer_ns` per roll ([ADR 0061](../../decisions/0061-a-sealer-thread-builds-the-key-frames-off-the-writer.md)), comes from `m7-session.sh collect`.
