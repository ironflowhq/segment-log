---
id: "0040"
title: "Volumes meet testable minimum requirements"
status: proposed
date: 2026-11-08
deciders: []
supersedes: null
superseded_by: null
tags: [durability, deployment, storage, testing]
touches: [docs/architecture/volume-requirements.md, docs/architecture/overview.md, docs/architecture/testing.md, docs/prd.md]
enforced_by: none
---

# 0040. Volumes meet testable minimum requirements

## Context

Every durability invariant is conditional on the storage under the log. The [storage contract](../architecture/overview.md#storage-contract) named three assumptions: the device honors the flush, sector writes are atomic, and the filesystem stops after a metadata write error. [ADR 0032](0032-qualify-deployment-storage-before-m6.md) requires evidence for those three on the actual stack before M6. It rejects a syscall trace as evidence on its own, but it does not say how to test each assumption or what counts as a pass.

The target placement adds layers and failure modes that the three assumptions do not cover:

- **The flush crosses several layers.** A guest flush passes through the guest filesystem, virtio-blk, the VMM, the host storage and the media. Any layer can drop it. Firecracker's default block `cache_type`, `Unsafe`, does not advertise the VirtIO flush feature to the guest, so the guest sends no flushes and a host power loss can drop data in the host page cache. QEMU's `cache=unsafe` ignores flushes.
- **The `LOCK` file works only inside one kernel.** ADR 0006 and ADR 0018 exclude network filesystems, but two microVMs that attach the same block device each see a local filesystem and can each take the lock. I8 would then fail silently.
- **No failure domain is stated.** The log keeps one copy, and replication is a non-goal. A volume on local media survives a guest exit and a power loss, but not the loss of its host. Nothing records which failures a deployment must survive.
- **Capacity is an outage risk.** [ADR 0037](0037-retain-complete-logs-through-current-milestones.md) retains complete history through M6, and [ADR 0004](0004-io-errors-poison-the-writer.md) poisons the writer on `ENOSPC`.
- **The host reintroduces the flush problem.** The PRD rejects one file per agent because one device cannot do tens of thousands of flushes per second. Many guests on one host share its media, so the host sees the sum of their group rates.
- **Qualification can drift.** A record proves one configuration. A later change to a mount option or a VMM setting can silently invalidate it.

The owner asked for minimum volume requirements and said that their testability is critical.

## Options & trade-offs

### Option A - testable property requirements in one document

Define each requirement as a property of the whole stack, with an ID, the invariant it protects, verification cases, a binary pass criterion and a known-bad control. Qualification checks the stack against the list.

- **For:** works for any hypervisor, media or managed block service. Each pass is evidence about the deployed stack. A known-bad control proves that the case can detect the fault. One list gives qualification, drift checks and requalification a shared vocabulary.
- **Against:** more work for each stack. Some cases need a power-cut rig. Managed services force reliance on provider documentation for layers the deployment cannot test.

### Option B - a reference list of approved hardware and platforms

For example: NVMe with power-loss protection, Firecracker with `Writeback`, ext4.

- **For:** short and easy to follow.
- **Against:** a product name does not prove behavior. Versions and defaults change. It cannot cover managed block services, and it does not test how the layers combine.

### Option C - self-checks in the core at open

For example: read `/proc/self/mountinfo` and refuse network filesystems.

- **For:** catches common misconfigurations in the process.
- **Against:** cannot detect the faults that matter most: flush forwarding, sector atomicity and the filesystem's error policy. A pass would look like assurance. It is Linux-specific parsing, and it changes the contract of `open`. The deployment can run the same check with a view of the whole stack.

### Option D - keep the three assumptions and ADR 0032 as they are

- **For:** no new work now.
- **Against:** leaves exclusive attachment, the failure domain, capacity, host density and drift without requirements, and leaves the test methods open.

## Decision

We will define the volume requirements as testable properties of the whole stack in [volume-requirements.md](../architecture/volume-requirements.md) (option A). That document is the only source for the requirements, their cases and their pass criteria. The storage contract in overview.md links to it.

- **Classes.** V1–V10 are MUST: a failure blocks qualification under ADR 0032. V11, flush latency, is MEASURED: a miss is a recorded finding and stops the stack from claiming the affected PRD target, as ADR 0029 treats target misses.
- **Testability rules.** Each case has a binary pass criterion and runs on the identified stack. Each detection case must fail against a known-bad control before its pass on the real stack counts. Evidence is `Tested`, or `Documented` only for layers that the deployment can neither configure nor test, and every `Documented` item is a recorded limitation. An unverified MUST requirement blocks qualification.
- **Minimum failure domain.** Every deployment survives a process crash, a guest crash, a guest destroy, a host kernel crash and a host power loss without losing an acknowledged record. It declares whether it also survives host loss, device loss and zone loss. Which of those a product needs stays open as PRD open item 12.
- **Exclusive attachment.** At most one kernel has the volume attached for writing, and no other kernel mounts it meanwhile (V5).
- **Copies.** Copies of an open log are crash-consistent, and only one writable lineage exists for each log id (V8).
- **Capacity.** Provisioned size covers the planned history, and an alert fires with enough free space for the deployment's response time (V9).
- **Drift.** Before every open, the deployment compares the running stack with the record's machine-readable fingerprint and refuses to open on a mismatch (V10). The core does not run this check (option C is rejected).

Option A beats B because only a test on the deployed stack shows that an ack is durable there. It beats C because the faults that lose acknowledged data are invisible from inside the guest process. We accept the cost of a power-cut rig for stacks we run ourselves, and of documented limitations for managed services, in exchange for evidence that applies to the deployed stack.

## Consequences

- ADR 0032's qualification record now checks V1–V11, not three assumptions. The three former assumptions are V1–V3.
- ADR 0033's deployment obligations gain exclusive attachment (V5), a failure-domain declaration (V7), single-lineage copies (V8), capacity sizing (V9) and the start-time drift check (V10).
- ADR 0037's deployment-owned lifecycle gains a capacity requirement and an alert (V9).
- PRD open item 12 holds the choice of failure domain beyond the minimum.
- A deployment on its own hosts needs a power-cut rig: a switched PDU or a BMC hard power-off, and an observer machine.
- A deployment on a managed block service records `Documented` evidence and limitations for host power loss, host loss and device loss, and still tests the guest-side events.
- The power-cut harness needs a test-only pause point in recovery to time the `VOL-POWER-CUT-RECOVERY` cut.
- Every change to a layer of a qualified stack triggers the reruns in the requalification table.
- The core's API, format and runtime dependencies do not change.

## Enforcement

None in code yet. The M6-DEPLOYMENT gate in [testing.md](../architecture/testing.md#milestone-acceptance-coverage) requires a qualification record with a row for every requirement. At runtime, the V10 drift check in the deployment stops a stack that has changed since qualification. A script in the repository that runs the configuration cases (`VOL-FLUSH-CONFIG`, `VOL-FS-CHECK`) and emits the fingerprint would make the automatable part repeatable.

## Revisit if

- A target platform supports only shared attachment, so V5 cannot hold. That needs a fencing design inside or around the log.
- A known-bad control cannot be made to fail for a case on the target stack, so the case cannot detect its fault.
- A product requires a failure domain that only `Documented` evidence can cover, and that evidence is judged insufficient.
- Recovery stops rewriting durable bytes, which narrows V2.
- Bounded retention is added, which changes the V9 sizing.
