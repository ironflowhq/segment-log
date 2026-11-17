---
id: "0059"
title: "M7 qualifies Firecracker on a dedicated local-NVMe host; a Mac stack only rehearses"
status: accepted
date: 2026-11-15
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [deployment, m7, storage, transport]
touches: [docs/prd.md, docs/architecture/testing.md, clients/go/**, scripts/**]
enforced_by: none
---

# 0059. M7 qualifies Firecracker on a dedicated local-NVMe host; a Mac stack only rehearses

## Context

M6 passed on 2026-11-15. M7 needs a named guest stack ([ADR 0056](0056-m6-builds-the-server-m7-validates-the-guest-deployment.md)): the qualification record checks every layer of it against V1–V11 ([volume-requirements.md](../architecture/volume-requirements.md)), and V10 requires the running stack to match the qualified fingerprint. So a stack qualifies only where it will run.

These facts are fixed:

- The development machine is an Apple M4 Mac. macOS has no KVM. Firecracker and Cloud Hypervisor need `/dev/kvm`. On 2026-11-15, OrbStack 2.2.5's Linux machines had no `/dev/kvm`. A Lima 2.3 VM (`vmType: vz`, `nestedVirtualization: true`) had it, and Firecracker v1.18.0 booted a 6.1 guest inside it.
- A laptop or desktop Mac cannot pass V1 or V7 for host power loss: there is no honest power cut, and three layers of virtualization distort V11 and the benchmark.
- A shared cloud VM cannot pass them either. Its storage is virtual and shared, and it cannot be power-cut. Hetzner Cloud servers do not expose `/dev/kvm` to run a VMM.
- The PRD throughput and latency targets assume local NVMe. Issue #49 also asks for a bare-metal local-NVMe baseline.
- Firecracker's default `cache_type`, `Unsafe`, hides the VirtIO flush feature from the guest ([V1](../architecture/volume-requirements.md#v1-a-successful-flush-is-durable)).
- Firecracker exposes guest vsock on the host as a Unix socket. A host process connects and sends `CONNECT <port>\n`, then reads `OK <port>\n` ("hybrid vsock"). The Go client dials only Linux `AF_VSOCK` today.

## Options & trade-offs

### Option A - Firecracker on a dedicated local-NVMe host, rehearsed on the Mac

- **For:** a minimal VMM built for single-purpose microVMs, with the flush behavior documented per `cache_type`. A dedicated server has real KVM, physical NVMe that can be identified (`nvme id-ctrl`) and a hardware reset, and it also runs #49. The Mac runs the same VMM and configuration one level down, so scripts and failures carry over.
- **Against:** drives attach at boot only, so handoff restarts a VMM rather than hotplugging a disk. Hybrid vsock needs a new client dialer. A hardware reset is not an AC power cut, so V7 must record that limit or find a real cut. A dedicated server costs a monthly fee.

### Option B - Cloud Hypervisor on the same host

- **For:** disk hotplug, vhost-user devices, and the same hybrid vsock model.
- **Against:** more surface than the log needs. Hotplug only helps if a live reader VM must take the volume, and V5 forbids a reader while the writer runs anyway.

### Option C - QEMU with vhost-vsock

- **For:** real `AF_VSOCK` on the host, so the existing dialer works. Mature and flexible.
- **Against:** a large device model and many cache modes to get wrong (`cache=unsafe`, `cache.no-flush=on`). It is not a microVM deployment.

### Option D - qualify on a cloud VM

- **For:** cheap and quick to get.
- **Against:** it cannot pass V1, V2 or V7 under host power loss, and it gives no `/dev/kvm`. It would qualify nothing.

## Decision

We will qualify **Firecracker on a dedicated server with local NVMe** (option A), and rehearse the same configuration on the Mac in a nested Lima VM.

- **VMM.** Firecracker, a pinned release. The log volume is its own virtio-blk drive, not the root drive, with `cache_type: Writeback`. The guest filesystem is ext4 on that drive (V4).
- **Volume.** A raw image file on the host filesystem. It outlives every guest (V6). Only one VMM attaches it at a time (V5). The launcher takes an exclusive `flock` on the image before it starts the VMM, and the VMM process holds that descriptor for its whole life. The lock dies with the process, and a paused VMM still holds it, so ownership cannot move away from a former owner that can still write.
- **Caller placement.** The caller runs on the VMM host and reaches the server through Firecracker's hybrid vsock. The Go client gains a dialer for it, with a scenario-first test.
- **Rehearsal.** The Mac stack (Lima nested, same Firecracker configuration) runs the conformance cases, `HANDOFF`, `VOL-FS-CHECK` and `VOL-FLUSH-CONFIG` from one script. It never fills a qualification row and never reports a benchmark.
- **Qualified host.** The exact server model, CPU architecture, NVMe model and its power-loss protection are recorded in the qualification record's fingerprint when the server is provisioned. A different machine is a different stack.

Firecracker beat Cloud Hypervisor because the log needs no hotplug, and V5 already rules out a reader beside a running writer. A dedicated host beat a cloud VM because only it can pass the MUST rows. We accept the monthly cost, a new dialer, and a V7 limit for a reset that is not a power cut.

## Consequences

- Accepted by the owner on 2026-11-17 for the 0.1 release ([issue 133](https://github.com/ironflowx/segment-log/issues/133)), with one amendment. The M7 record must come from one host: the first session's host offered no power button reset, so host power loss and the ADR 0063 benchmark ran on a second host with other drives and CPU, which this ADR's rule "a different machine is a different stack" forbids. The whole record is rerun on one host that offers `power_long`, after format version 3 lands. The fingerprint gains host and media keys (host kernel, the volume filesystem's mount options, drive model and firmware, `vwc`), checked by the launcher before boot, so that V10 refuses a swapped machine.
- PRD open items 3 and 4 name this stack. Their evidence stays open until the qualification and handoff records pass.
- The M7 work runs in three stages: the Mac rehearsal, qualification on the dedicated host, then the guest benchmark. Only the last two count toward the gate.
- The power-cut harness observer runs on a second machine, as V7 requires.
- The bare-metal run of issue #49 on the same host is the baseline beside the guest benchmark, so the cost of virtualization is visible.
- The handoff record names the reader component. `LogReader` reads sealed history. An unsealed crash log needs a guest that opens it writable through `SegmentLog::open`.
- `clients/go` gains a host-side Firecracker vsock dialer.

## Enforcement

None. The qualification record's fingerprint and V10's drift check stop a different stack from passing as this one. A reviewer checks that no Mac result fills a qualification row.

## Revisit if

- The production caller runs in another guest, not on the VMM host.
- The deployment moves to a managed platform whose VMM is not Firecracker.
- No dedicated host can pass V1 under a real power cut, so the failure domain needs replicated storage (PRD open item 12).
