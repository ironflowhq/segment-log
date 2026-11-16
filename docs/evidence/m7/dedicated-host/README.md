# The ADR 0059 stack on a dedicated NVMe host: rehearsal and guest-side power cuts

This record runs `scripts/m7-rehearse.sh` and `scripts/m7-power-cut.sh` on bare metal for the first time, on the [stack that ADR 0059 chose](../../../decisions/0059-m7-qualifies-firecracker-on-a-dedicated-nvme-host.md). It fills no row of the qualification record yet: host kernel crash and host power loss are not run, and V2, V3 and V8–V11 have no cases yet.

## Host

The same machine as the [issue 49 record](../../issue49/README.md): Intel Core i7-7700, 64 GB, Debian 12 with kernel 6.1.0-54, `/dev/kvm` on the host ([host.txt](host.txt)). The log volumes are raw images on ext4 on `nvme1n1`, one Samsung PM981 drive with no RAID, no power-loss protection, and its write cache on ([drives.txt](drives.txt)). Firecracker v1.18.0, guest kernel 6.1.155, both from `scripts/m7/setup.sh`. The observer ran on a Hetzner Cloud server in Ashburn, so no event on this host could touch it.

## Rehearsal

[rehearse-ax.log](rehearse-ax.log): every step passes.

- `VOL-FS-CHECK`: the guest sees `/dev/vdb` as ext4.
- `VOL-FLUSH-CONFIG`: `Writeback` shows the guest a write-back cache. The `Unsafe` control shows write through, so the check can fail.
- One VMM per volume, on one host: a second VMM is refused while the owner runs and while it is stopped, and a new one attaches after the owner is killed.
- Conformance over Firecracker vsock: all nine shared cases, `TRANSPORT-UNKNOWN` and `TRANSPORT-RESTART` pass.
- `HANDOFF`: `TestHandoffSealed` and `TestHandoffUnsealed` pass, each start on a new guest.

## Power cuts, guest-side events

Each real run has 30 cycles on one volume. Each check reads the whole log back, so the last check covers every cycle. Each control has 10 cycles and reports records before their acknowledgment ([pc-all.sh](pc-all.sh) is the driver).

| Event | Real run | Records in the log at the last check | Reported and checked | Control |
| --- | --- | ---: | ---: | --- |
| `process-crash`: SIGKILL the server in the guest | 0 of 30 cycles lost a record | 45,623 | 45,377 | 10 of 10 cycles caught |
| `guest-crash`: sysrq `c` in the guest | 0 of 30 | 42,758 | 42,637 | 10 of 10 caught |
| `guest-destroy`: SIGKILL the VMM | 0 of 30 | 46,975 | 46,913 | 10 of 10 caught |

The logs are [pc-process-crash.log](pc-process-crash.log), [pc-guest-crash.log](pc-guest-crash.log) and [pc-guest-destroy.log](pc-guest-destroy.log), and their `-control` pairs. Records that are in the log but were never reported are the unknown suffix that the commit protocol allows.

## What this does not show

- **Host kernel crash and host power loss.** These are the events where a drive without power-loss protection can lose a flushed write that its cache acknowledged. They need a second machine to cause them, through the Robot reset API or a power switch, and they are not built yet.
- V2 (a cut during recovery's rewrite), V3, V8, V9, V10 and V11 have no cases yet.
- The guest benchmark that M7's gate asks for is not run. The [issue 49 record](../../issue49/README.md) has the bare-metal numbers in process.
