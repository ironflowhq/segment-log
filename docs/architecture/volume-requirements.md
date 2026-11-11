# Volume requirements

The log's durability guarantees are conditional. They hold only if the volume under the log directory meets the requirements below. This document is the only source for those requirements, their pass criteria and how to verify them. [ADR 0040](../decisions/0040-volumes-meet-testable-minimum-requirements.md) records why they exist. [ADR 0032](../decisions/0032-qualify-deployment-storage-before-m6.md) requires a qualification record for the actual stack before the M6 guest benchmark and before any production durability claim. Terms are as defined in the [glossary](../glossary.md).

The library cannot check most of these requirements from inside the process. The deployment layer owns the volume ([ADR 0033](../decisions/0033-deployment-owns-volumes-and-reader-handoff.md)), so the deployment runs the checks and keeps the evidence.

## The stack

A volume is every layer between the log directory and the bytes that survive a failure. A requirement holds only if every layer it touches meets it. The weakest layer decides.

```text
 log directory (SegmentLog::open)
   │
 guest filesystem          ext4 or xfs, mount options
   │
 guest block driver        virtio-blk: write cache mode, flush support
   │
 VMM / hypervisor          block cache mode, flush forwarding, attachment
   │
 host storage              backing file and host filesystem, or a host block device
   │
 durable media             physical device (volatile cache, power-loss protection),
                           or a network block service (replication, flush semantics)
```

## How a requirement is written

Each requirement has the same fields.

| Field | Meaning |
| --- | --- |
| ID | `V1`, `V2`, … Stable. Tests, records and issues refer to it. |
| Class | **MUST**: a failure blocks qualification. **MEASURED**: the result is recorded. A miss is a finding, not a qualification failure, and it stops the stack from claiming the affected PRD target. |
| Protects | The invariant, requirement or decision that fails without it. |
| Layers | The layers of the stack it touches. |
| Cases | The verification case IDs. Each case has a procedure and a binary pass criterion. |

## Testability rules

These rules apply to every case below.

1. **Binary pass criterion.** A case passes or fails. "Looks fine" is not a result.
2. **Run on the identified stack.** Evidence counts only for the stack named in the [qualification record](#qualification-record). Evidence from a developer machine or a different configuration does not count.
3. **A known-bad control.** Each case that detects a stack behavior also runs against a configuration known to break the requirement. The case must report that control as failing. If it does not, the procedure cannot detect the fault, and its pass on the real stack proves nothing. Raise the load, the cycle count or the instrumentation until the control fails, then use the same parameters for the real run. Records and measurements without a detection step (V9 sizing, V11) have no control.
4. **Evidence classes.** `Tested`: the case ran on the identified stack. `Documented`: a vendor or provider statement, cited with its URL, version and retrieval date. `Documented` is allowed only for a layer that the deployment can neither configure nor test, such as the physical media under a managed block service. Each `Documented` item is recorded as a limitation. A layer that the deployment controls needs `Tested` evidence.
5. **Not verified is a failure.** An unverified or contradicted MUST requirement blocks qualification. It is never recorded as a pass.
6. **Requalify on change.** A change to a layer reruns the cases that the [requalification table](#requalification-triggers) maps to it.
7. **Automate what can be automated.** Configuration checks are scripts, and V10 runs them on every guest start.

## Summary

| ID | Requirement | Class | Protects | Cases |
| --- | --- | --- | --- | --- |
| V1 | A successful flush makes the bytes survive every failure event in the declared failure domain. | MUST | I1, I3; [ADR 0005](../decisions/0005-flush-calls-and-platforms.md) | `VOL-FLUSH-CONFIG`, `VOL-FLUSH-TRACE`, `VOL-POWER-CUT` |
| V2 | A power cut never leaves a sector with contents other than its old or its new bytes. | MUST | I3, I4; [ADR 0015](../decisions/0015-open-makes-the-recovered-tail-durable.md) | `VOL-POWER-CUT-RECOVERY`; `VOL-ATOMIC-WRITE` as supporting evidence |
| V3 | After a metadata write error, the filesystem refuses every later write and sync until it is remounted. | MUST | I3; [ADR 0015](../decisions/0015-open-makes-the-recovered-tail-durable.md) | `VOL-METADATA-ERROR` |
| V4 | The log directory is on a local ext4 or xfs filesystem on a block device. | MUST | I8; [ADR 0005](../decisions/0005-flush-calls-and-platforms.md), [ADR 0018](../decisions/0018-writer-epoch-from-the-segments.md) | `VOL-FS-CHECK` |
| V5 | At most one kernel has the volume attached for writing, no other kernel mounts it while that one does, and a former owner cannot write after ownership moves. | MUST | I8; R16 | `VOL-EXCLUSIVE` |
| V6 | The volume outlives the guest and is readable after the guest is destroyed. | MUST | R20, R23; [ADR 0033](../decisions/0033-deployment-owns-volumes-and-reader-handoff.md) | `HANDOFF` ([testing.md](testing.md#surviving-volume-handoff-before-m6)) |
| V7 | The deployment declares its failure domain. It covers at least the minimum below, and each declared event passes the power-cut harness. | MUST | I3 | `VOL-FAILURE-DOMAIN` |
| V8 | A copy of a log is crash-consistent, each log id has one writable lineage, and a copy becomes writable only if it holds every acknowledged record. | MUST | I3, I4, I6, I8; R17 | `VOL-COPY` |
| V9 | Capacity at every allocating layer covers the planned history, and an alert fires while there is still time to act. | MUST | R6, R22; [ADR 0004](../decisions/0004-io-errors-poison-the-writer.md), [ADR 0037](../decisions/0037-retain-complete-logs-through-current-milestones.md) | `VOL-CAPACITY`, `VOL-ENOSPC` |
| V10 | On every guest start, the running stack matches the qualified fingerprint before the log opens. | MUST | All of the above, after qualification | `VOL-DRIFT` |
| V11 | Flush latency with the log's write pattern is measured alone and at planned density, and the throughput estimate is recorded. | MEASURED | [PRD targets](../prd.md#footprint-and-performance-targets) | `VOL-FLUSH-LATENCY` |

## Correctness

### V1. A successful flush is durable

**Requirement.** When `sync_data` or `sync_all` returns success inside the guest, the written bytes survive every failure event in the declared failure domain (V7). No layer acknowledges a flush that it has not passed down or made durable itself.

**Layers.** All.

**Why.** An ack means the group is durable only if the flush reached durable media. Every layer can break this. A filesystem mounted with barriers off skips the flush. A VMM can hide flush support from the guest. For example, Firecracker's [block caching documentation](https://github.com/firecracker-microvm/firecracker/blob/main/docs/api_requests/block-caching.md) says that its default `cache_type`, `Unsafe`, does not advertise the VirtIO flush feature to the guest. The guest then sends no flushes, and a host power loss can drop data still in the host page cache. `Writeback` advertises the feature and runs `fsync` on the backing file for each flush. QEMU's `cache=unsafe` ignores flushes. Confirm the behavior of the version you deploy. A host can keep the backing file in its page cache. A device can have a volatile write cache without power-loss protection and ignore FLUSH.

`VOL-FLUSH-CONFIG`. A configuration audit, as a script.

- Guest: `/sys/block/<dev>/queue/write_cache` reads `write back`, so the kernel sends flushes. The file reports the kernel's view of the device, not a verified property of the layers below. `write through` passes only with evidence that every layer completes each write durably before it reports completion, for example a VMM that opens the backing file with `O_DSYNC`. Lower caches can exist if each write is flushed through them before completion.
- Guest: the mount options of the log's filesystem do not disable barriers (`nobarrier`, `barrier=0`).
- VMM: the block device configuration forwards flushes (for example, Firecracker `cache_type: Writeback`; QEMU not `cache=unsafe` and not `cache.no-flush=on`).
- Host: the backing file is not on `tmpfs` or any other memory-backed filesystem. The host filesystem does not disable barriers.
- Media: the record states whether the device has a volatile write cache (on NVMe, the `VWC` field of `nvme id-ctrl`) and whether it has power-loss protection. A volatile cache without power-loss protection must honor FLUSH, which `VOL-POWER-CUT` then tests.

Pass: every item holds. Control: the audit fails on a configuration with flush forwarding off.

`VOL-FLUSH-TRACE`. Run the [flush probe](#flush-probe) in the guest for at least 60 seconds. Trace flush and FUA requests in the guest block layer for the log's device (for example, with `blktrace`). On the host, trace the durable operations on the backing storage: `fsync`, `fdatasync`, `O_DSYNC` or FUA writes by the VMM, or the backing device's flush requests.

Pass: for every probe sync, a guest flush or FUA request completes after the probe's write and before the sync returns. Each such guest request is covered by a host durable operation that starts after the request arrives and completes before the guest request completes. One host operation can cover several guest requests, because the kernel merges flushes, so the counts need not match one to one. Aggregate counters, such as the flush fields of `/proc/diskstats` (not kept for partitions), are diagnostics only. Control: with flush forwarding off, the trace shows probe syncs with no covering host durable operation. A trace alone never qualifies V1 ([ADR 0032](../decisions/0032-qualify-deployment-storage-before-m6.md)). `VOL-POWER-CUT` is still required.

`VOL-POWER-CUT`. Run the [power-cut harness](#power-cut-harness) for each power or crash event in the declared failure domain.

Pass: zero acknowledged records missing in every cycle. Control: the [control for each event](#controls-by-event) reports at least one missing acknowledged record within the same cycle budget.

### V2. Sector writes are atomic

**Requirement.** After a power cut during a write, each sector holds either its old or its new contents. Recovery relies on this when it rewrites bytes that are already durable ([ADR 0015](../decisions/0015-open-makes-the-recovered-tail-durable.md)).

**Layers.** VMM, host storage, durable media.

`VOL-ATOMIC-WRITE`. Record the media's power-fail atomic write parameters with their units decoded, and the logical block size at each layer. On NVMe, `AWUPF` (`nvme id-ctrl`) and `NAWUPF` (`nvme id-ns`) are 0's based counts of logical blocks, and atomic boundary fields can also apply. These parameters guarantee atomicity only for writes within their size and boundary limits, and the layers above can issue larger writes. This case is supporting evidence and does not pass V2 alone.

Pass: the record lists each value, its decoded unit and its source.

`VOL-POWER-CUT-RECOVERY`. Run the [power-cut harness](#power-cut-harness) with the cut inside open's rewrite of bytes that are already durable ([ADR 0015](../decisions/0015-open-makes-the-recovered-tail-durable.md)). Run each fixture:

1. A clean reopen: the last group frame is acknowledged and durable, and open rewrites it.
2. A last frame that shares its first and last sectors with earlier acknowledged bytes, because it neither starts nor ends on a sector boundary.
3. From M4, a sealed segment whose closing frames open rewrites. A segment with only a header.

Cut while a rewrite write is in flight to the device: after open issues it and before its `sync_all` completes. Count a cycle only when evidence that reaches the observer before the cut, such as streamed block-layer trace events, shows that a rewrite write was issued and had not completed.

Pass: zero acknowledged records missing, and the next open succeeds without `Corrupt`. V2 passes when this case passes on the identified stack. For a managed block service, `Documented` evidence of per-sector power-fail atomicity can replace it. Control: a test-only fault writes different bytes over one acknowledged sector in place of the identical rewrite. The harness must report the lost record.

### V3. The filesystem stops after a metadata write error

**Requirement.** After a metadata write fails, the filesystem refuses every later write and sync until it is remounted, even if the device recovers. A failed directory `fsync` must never be followed by a successful one that wrote nothing.

**Layers.** Guest filesystem.

`VOL-METADATA-ERROR`. In a qualification VM with the same guest kernel, filesystem, `mkfs` options and mount options, put the filesystem on a device-mapper target that can switch to failing writes (`dm-flakey` or `dm-error`). A small test program runs these steps:

1. Create a file in a directory, then write it and call `sync_all`, so that its data is durable.
2. Switch the target to fail writes. Create a second file in the same directory, then `fsync` the directory. Confirm with a block-layer trace that a metadata write failed.
3. Switch the target back to working.
4. `fsync` the directory again, once through the handle from step 2 and once through a newly opened handle. Then write and sync a file.

Pass: the directory `fsync` in step 2 fails, and every call in step 4 fails until remount. If step 2 cannot be made to fail, the case is `Not verified`. With `errors=panic`, run a separate procedure: it passes when the guest halts at the failure, before any later call returns. On ext4, the audit also confirms `errors=remount-ro` or `errors=panic` (`tune2fs -l` or the mount options). Control: where the kernel under test has a mode that continues after metadata errors (for ext4, `errors=continue` is the candidate), the case must fail under it. If that mode still stops writes on this kernel, record that no control could be built.

### V4. A local ext4 or xfs filesystem on a block device

**Requirement.** The log directory and its parent are on one local ext4 or xfs filesystem, backed by a block device. Network and shared filesystems are not supported. This includes NFS, SMB, FUSE, overlay, `tmpfs`, 9p and virtiofs. The `LOCK` file and the flush semantics need a local filesystem.

**Layers.** Guest filesystem.

`VOL-FS-CHECK`. Run `findmnt -no SOURCE,FSTYPE,OPTIONS -T <log dir>` and the same for its parent.

Pass: the type is `ext4` or `xfs`, the source is a block device, and the directory and its parent are on the same filesystem. Control: the check fails for a directory on `tmpfs`.

### V5. One writing kernel

**Requirement.** At most one kernel has the volume attached for writing. No other kernel mounts the filesystem, even read-only, while that one does. A reader attaches only after the writer's guest has stopped and the volume is detached. After ownership moves, the former owner cannot write to the volume, even if it was only paused.

**Layers.** VMM, host storage, deployment attachment logic.

**Why.** The `LOCK` file excludes a second writer only among processes under one kernel. Two microVMs that attach the same block device each see a local filesystem, and each can take its own lock. Their frames would then interleave, which breaks every invariant. A read-only mount next to a live writer can also read stale metadata.

`VOL-EXCLUSIVE`.

1. While guest A has the volume attached for writing, try to attach it to guest B, once for writing and once read-only.
2. Start a second VMM with the same backing file or device that guest A uses.
3. Start two launches for the same unattached volume at the same moment, from two launchers, and from two hosts where the platform allows it. Repeat at least 20 times.
4. Pause guest A (for example, `SIGSTOP` on its VMM). Move ownership of the volume to guest B with the deployment's procedure. Resume guest A and let it write.

Pass: the attempts in steps 1 and 2 are refused. In step 3, exactly one launch wins in every repetition. In step 4, no write from guest A reaches the volume after the move. The record names the atomic fencing mechanism. Control: with fencing disabled in the launcher, step 2 succeeds. With a launcher that checks and then attaches without an atomic step, and a delay added between the two, step 3 admits two winners.

## Survival

### V6. The volume outlives the guest

**Requirement.** Destroying the guest through the platform's real teardown path does not destroy the volume. Acknowledged records stay readable after the guest is gone.

`HANDOFF` in [testing.md](testing.md#surviving-volume-handoff-before-m6) is the case. Use the real teardown path, not a graceful shutdown. Pass: every acknowledged record is readable by the selected reader.

### V7. A declared failure domain

**Requirement.** The deployment declares the failure events after which the volume still holds every acknowledged record. The log keeps one copy and has no replication, so the volume sets the failure domain.

The declaration covers at least this minimum:

| Event | How the harness causes it |
| --- | --- |
| Process crash | `SIGKILL` the writer process. |
| Guest crash | `echo c > /proc/sysrq-trigger` in the guest. |
| Guest destroy | The platform's teardown path, for example `SIGKILL` of the VMM process. |
| Host kernel crash | `echo c > /proc/sysrq-trigger` on the host. |
| Host power loss | Cut power at the outlet (switched PDU). A hard BMC power-off (`ipmitool chassis power off`, not `soft`) counts only with evidence that it removes power from the media and their controllers on this hardware. Some servers keep standby power. |

The declaration states `yes` or `no` for each event beyond the minimum: host loss (the host never returns, and the volume is attached to another host), device loss (one device or replica fails), and zone loss. Which events a product needs is [PRD open item 12](../prd.md#open-items).

`VOL-FAILURE-DOMAIN`. For each declared event, run the [power-cut harness](#power-cut-harness) with that event. For host loss, attach the volume to a different host before recovery. For device loss, fail one device or replica during load.

Pass: zero acknowledged records missing for every declared event. A managed block service may supply `Documented` evidence for host power loss, host loss and device loss, because the deployment cannot cause those events. The guest-side events stay `Tested`.

### V8. Copies are crash-consistent and have one writable lineage

**Requirement.** This applies when the deployment snapshots, clones, backs up or archives a log volume. A copy taken while the log is open is a crash-consistent point-in-time image of the whole directory. Each log id has one writable lineage at a time: the log that accepts appends. A copy becomes that lineage only when it holds every record the log acknowledged. That holds for a copy taken after the original's writer stopped and its volume was fenced (V5), or after seal. A stale copy never replaces the original, because it would drop acknowledged records and reuse their offsets (I3, I6). A copy of a live log can be opened for inspection on an isolated volume. Opening it runs writable recovery, but it accepts no appends from callers and is discarded afterward.

`VOL-COPY`.

1. Take a snapshot during load. Restore it to an isolated scratch volume and open it there for inspection.
2. Seal a log, archive it, restore the archive, and read it with `LogReader` (M4).
3. During load, fence the original, copy it, and continue appending on the copy as the new lineage.
4. Run the restore procedure with a copy taken before the original's latest acknowledgment: once while the original is live, and once after it is retired.

Pass: in step 1, open succeeds without `Corrupt` and every record acknowledged before the snapshot started is present. In step 2, every record is readable. In step 3, every record the original acknowledged is present on the copy before it accepts an append. In step 4, the procedure refuses to make the stale copy writable in both runs. If the deployment never copies a log volume, record `not applicable` with that statement.

## Capacity

### V9. Capacity and a timely alert

**Requirement.** Valid history is never deleted through M6 ([ADR 0037](../decisions/0037-retain-complete-logs-through-current-milestones.md)), and `ENOSPC` poisons the writer ([ADR 0004](../decisions/0004-io-errors-poison-the-writer.md)). A full volume is therefore an outage. Capacity covers the planned peak history of each log at every layer that allocates space: the guest filesystem, and any sparse backing file, thin pool or quota beneath it. Exhausting a lower layer appears in the guest as an I/O error or a stall, and an I/O error also poisons the writer. An alert fires at each layer while there is still time to act.

`VOL-CAPACITY`.

- **Sizing record.** From the workload profile (records per log, mean stream id, key and payload sizes, mean records per group), compute the planned peak encoded size. Use the sizes in [format.md](format.md): segment headers, frame envelopes, body headers and record entries, plus key filter frames, key index frames, footers, seal frames and locators from M4. Measure the filesystem's allocation ratio: write a test log of known encoded size and read its allocated size (`du`). The planned allocated size is the encoded size times that ratio, plus the recorded headroom. Record the computation.
- **Alert threshold.** At each allocating layer, the alert fires when free bytes fall below the peak write rate in bytes per second multiplied by the deployment's response time: the time it needs to seal, move or grow the volume. Record both numbers.

Pass: at every allocating layer, the reserved or free capacity is at least the planned allocated size. Check the guest (`df` after `mkfs`) and each applicable lower layer: the host filesystem under a sparse backing file, thin-pool data and metadata, and quotas. In a fill test, the alert fires at its threshold at each layer.

`VOL-ENOSPC`. During load, exhaust each allocating layer separately: the guest filesystem, then each applicable lower layer. Restore the capacity and reopen.

Pass: the writer poisons or stalls without acknowledging a record it did not make durable, every acknowledged record is present after reopen, and open succeeds without `Corrupt`.

## Drift

### V10. The running stack matches the qualified fingerprint

**Requirement.** Qualification proves one configuration. Before every `SegmentLog::open`, the deployment compares the running stack with the machine-readable fingerprint in the qualification record. It does not open the log on a mismatch, and the error names the field. The fingerprint holds at least the guest kernel release, the filesystem type, source and mount options, the ext4 error behavior, the guest `write_cache` value and the VMM block configuration (checked by the launcher before boot).

`VOL-DRIFT`. Change one fingerprint field at a time: a mount option, the VMM cache mode, the kernel release.

Pass: each change stops the log from opening, and the error names the changed field.

## Performance

### V11. Flush latency allows the throughput target

**Requirement.** Measure the latency of the log's write pattern on the qualified stack, alone and at the planned density. Many guests on one host share its media, so the host sees roughly the number of guests multiplied by each guest's group rate in flushes per second.

`VOL-FLUSH-LATENCY`. Run the [flush probe](#flush-probe) inside the guest for at least 60 seconds after a warm-up:

1. alone on the host;
2. at the planned density: the planned number of guests on one host, each running the probe at the same time, limited to the planned group rate for each guest (with fio, `--rate_iops`);
3. at the planned density without a rate limit, as a saturation diagnostic.

Record the mean, p50, p99 and p999 of each write-and-sync cycle, the syncs per second for each guest, and the host's total flushes per second.

**Throughput estimate.** Record:

```text
records/s ≈ callers × records per submit / (linger + mean cycle at planned density)
```

Take the caller count, records per submit and linger from the benchmark configuration being claimed: the [PRD benchmark conditions](../prd.md#footprint-and-performance-targets) and the [api.md](api.md) defaults. The estimate assumes that each caller waits for its acknowledgment before it submits again, and that every group closes on linger with all waiting callers, not on the byte cap. It is an estimate, not a bound. If it falls below the PRD throughput target, record that the stack is not expected to meet the target. Only the M6 guest benchmark establishes a target result.

## Shared procedures

### Flush probe

The probe repeats the log's commit pattern: append one group-sized write to the end of a new file, then `sync_data`. Each write grows the file, so each sync must also commit the new size. A rewrite in place does not measure this. Group size is the expected encoded group body for the workload. The [benchmark section](testing.md#benchmarks-m1-for-the-core-m6-for-the-server) uses the same pattern for its device latency.

A reference with fio follows. Confirm on your fio version that every write grows the file:

```sh
fio --name=segment-log-flush --filename=<log volume>/flush-probe \
    --rw=write --bs=<group bytes> --size=<bytes for at least 60 s> \
    --file_append=1 --fallocate=none --ioengine=psync --fdatasync=1
```

### Power-cut harness

The harness checks that acknowledged records survive a failure event. It is built on the library from M1, not on a stand-in.

1. A writer in the guest appends fully keyed records with deterministic payloads at a steady load.
2. After each acknowledgment, the writer sends the key, log offset and stream revision to an observer on a different machine. The observer stores them durably. Records that are durable but never reported are allowed: the unknown suffix is part of the [commit protocol](overview.md#commit-protocol).
3. The harness causes the failure event at a uniformly random time within the load window.
4. After restart, open the log writable (recovery), then read everything.

A cycle passes when open succeeds without `Corrupt`, every record the observer stored is present with the same log offset and revision (I3), and the log offsets have no gap (I4).

Run at least 30 cycles for each event, and at least 10 counted cycles for each `VOL-POWER-CUT-RECOVERY` fixture. Use the same load and cycle count for the known-bad control and for the real stack. Passing cycles bound the failure rate; they do not prove its absence. Report the cycle count and the load.

#### Controls by event

A control must lose an acknowledged record under the event it validates. One fault does not do that for every event: a process crash keeps the guest page cache, and a guest crash keeps the host page cache, so disabling flush forwarding loses nothing under them.

| Event | Control | What it proves |
| --- | --- | --- |
| Process crash, guest crash, guest destroy | A test-only writer build that acknowledges each group before it writes it | The harness detects a lost acknowledged record. |
| Host kernel crash, host power loss | Flush forwarding off (for example, Firecracker `Unsafe`), or a volatile write cache whose flushes are ignored | The cut really drops the host's volatile caches, and the harness sees the loss. |
| Host loss, device loss, zone loss | The same event on a configuration without the replication or relocation being qualified | Survival depends on the mechanism being qualified. |

## Qualification record

One record in `docs/` for each qualified stack. It contains:

- **Fingerprint.** For each layer: component, version and configuration. Guest kernel; filesystem, `mkfs` options and mount options; guest block device and `write_cache`; VMM and its block configuration; host kernel, filesystem and mount options, or the host block device; media model, firmware, volatile cache and power-loss protection, or the managed block service and its tier. The planned density. A machine-readable copy for V10.
- **Failure domain.** The V7 declaration.
- **One row for each requirement.**

| ID | Case | Evidence class | Commands or script | Control result | Result | Limitations |
| --- | --- | --- | --- | --- | --- | --- |
| V1 | `VOL-POWER-CUT` | Tested | … | Lost 3 acks in 30 cycles (expected) | Pass: 0 lost in 30 cycles | … |

Result is `Pass`, `Fail`, `Not verified` or `Not applicable` (V8 only, with the reason). The stack qualifies when every MUST row is `Pass`, or `Not applicable` where allowed.

## Requalification triggers

| Change | Rerun |
| --- | --- |
| Guest kernel | V1, V2, V3, V10, V11 |
| Guest filesystem, `mkfs` or mount options | V1, V3, V4, V10, V11 |
| VMM version or block configuration | V1, V2, V5, V10, V11 |
| Host kernel, filesystem or mount options | V1, V2, V7, V11 |
| Media, firmware, or managed storage tier | V1, V2, V7, V11 |
| Attachment, launcher or restore logic | V5, V6, V8 |
| Workload profile | V9, V11 |
| Planned density | V11 |
