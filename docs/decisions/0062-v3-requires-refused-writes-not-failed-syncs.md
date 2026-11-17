---
id: "0062"
title: "V3 requires that writes fail after a metadata write error, not that every later sync fails"
status: accepted
date: 2026-11-16
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [storage, qualification, m7, filesystem]
touches: [docs/architecture/volume-requirements.md, scripts/m7-metadata-error.sh]
enforced_by: test
---

# 0062. V3 requires that writes fail after a metadata write error, not that every later sync fails

## Context

[ADR 0015](0015-open-makes-the-recovered-tail-durable.md) makes open write back the recovered tail and sync it. It cannot write back a directory entry, so it states one assumption: "the filesystem stops accepting writes after a metadata write error." [ADR 0040](0040-volumes-meet-testable-minimum-requirements.md) turned that assumption into [V3](../architecture/volume-requirements.md#v3-the-filesystem-stops-after-a-metadata-write-error). V3 says more than ADR 0015: "the filesystem refuses every later write **and sync** until it is remounted", and `VOL-METADATA-ERROR` passes only if every call in step 4, including two directory `fsync` calls, fails.

`scripts/m7-metadata-error.sh` ran that case on the M7 guest, with ext4 on guest kernel 6.1.155 and 5.10.245 ([evidence](../evidence/m7/lima-v3-v9/README.md)):

- The directory `fsync` while writes fail returns EIO. The journal aborts, and ext4 remounts the filesystem read-only.
- Afterwards, the directory `fsync` through the held handle and through a new handle both return success. The file write fails with EROFS.
- The outcome is the same with `errors=remount-ro`, `errors=continue` and `errors=panic`. Under `errors=panic` the panic comes only at the file write. A read-only ext4 answers an `fsync` that has nothing to write with success.

So V3 as written fails on ext4 under every mount mode, while the property that ADR 0015 relies on holds: no later write succeeds.

These facts are fixed:

- The writer never retries a failed flush and poisons on the first one ([ADR 0004](0004-io-errors-poison-the-writer.md)). A poisoned writer appends nothing more.
- A new process trusts nothing it has not flushed itself. Open writes back the recovered tail, then calls `sync_all` and `sync_dir` (ADR 0015). Every recovery that changes a file writes before it syncs: truncation, the closing run that [ADR 0061](0061-a-sealer-thread-builds-the-key-frames-off-the-writer.md) rebuilds, and the next segment's creation.
- A writable open that cannot write fails with `Io`.

## Options & trade-offs

### Option A - Keep V3 and qualify another filesystem or kernel

- **For:** V3 stays as strict as it is written.
- **Against:** xfs and other ext4 kernels would need the same test, with no reason to expect a different `fsync` answer on a read-only filesystem. M7 stalls on a requirement that nothing the log does depends on.

### Option B - Require that every later write fails, and keep syncs as evidence

- **For:** V3 then states exactly what ADR 0015 assumes, and what the log relies on. A later `fsync` can only succeed with nothing to write, because every write after the error fails, and the log takes no `fsync` as proof of bytes it did not write in the same process before a successful sync.
- **Against:** V3 no longer catches a filesystem that would accept writes after a failed directory `fsync` and then report a later `fsync` as clean. The write check catches that filesystem instead, because it accepts writes.

### Option C - Require `errors=panic` and a halt at the first error

- **For:** nothing can run after the error.
- **Against:** ext4 on this kernel panics only at the next journal start, after both `fsync` calls returned, so it fails too. A panic also takes down every log in the guest for one device error.

## Decision

We will use option B. V3 becomes:

> After a metadata write fails, the filesystem refuses every later write until it is remounted, even if the device recovers.

`VOL-METADATA-ERROR` changes with it:

- Step 4 writes and syncs a new file, and creates a new entry in the directory from step 2.
- Pass: the directory `fsync` in step 2 fails, and every write in step 4 fails until remount. The results of step 4's directory `fsync` calls, through the held handle and through a new one, are recorded as evidence and decide nothing.
- The control, the `errors=panic` procedure and the ext4 audit of `errors=remount-ro` or `errors=panic` stay as they are. On a kernel where `errors=continue` still makes the filesystem read-only, the record says that no control could be built, as V3 already allows.

Option B beat A because the log's correctness depends on refused writes, not on refused syncs, and A would block M7 on a property no layer of the log uses. It beat C because C fails on the same kernel and costs every log in the guest.

## Consequences

- volume-requirements.md changes V3's requirement, its summary row and its pass rule in the same change.
- `scripts/m7-metadata-error.sh` decides from step 2's `fsync` and step 4's writes, and prints the step 4 `fsync` results.
- The qualification record's V3 row can pass on ext4 with `errors=remount-ro`, which the fingerprint already pins.
- ADR 0015's assumption is now the requirement, word for word, so a later change to either names both.

## Enforcement

`scripts/m7-metadata-error.sh` on the qualified stack, recorded in the M7 qualification record ([issue 112](https://github.com/ironflowx/segment-log/issues/112)).

## Revisit if

- The log gains a path that trusts a later `fsync` for bytes written before an earlier failed one, in the same process or across a restart.
- A qualified filesystem accepts any write after a failed metadata write.
