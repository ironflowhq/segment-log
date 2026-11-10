---
id: "0018"
title: "Writer epoch from the segments, with no `EPOCH` file"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: "0006"
superseded_by: null
tags: [concurrency, recovery]
touches: [docs/architecture/format.md]
enforced_by: none
---

# 0018. Writer epoch from the segments, with no `EPOCH` file

## Context

Source: the lock, and the epoch covered by the CRC, are from the reviewed design (decision 1 and "File and recovery"). Deriving the epoch from the segments is new in this repository, from a review of the docs on 7 November 2026, and not reviewed.

Two live writers on one directory would interleave frames and break every invariant. ADR 0006 adds two guards. The first is an exclusive OS lock on `LOCK`. The second is a writer epoch, which open keeps in an `EPOCH` file and increments durably. ADR 0006 itself says that the lock is the main guard and that a network filesystem is not supported.

On a local filesystem, a process holds its lock until it closes the file or exits. A second live writer cannot exist, so the epoch check at recovery catches only bugs, such as code that releases the lock too early. Such a stale writer starts from the same next log offset as the new writer, so the base-offset rule in [format.md](../architecture/format.md#rules-for-readers) also ends the valid prefix at its first frame. In both designs, the new writer's frames after that point are lost at the next recovery. The epoch detects a stale writer. It does not save data from one. The `EPOCH` file costs a temporary file, a rename and two syncs at every open. It also makes every open a write. ADR 0006 rejected deriving the epoch because a process that wrote nothing leaves no trace, so the next process can get the same epoch. That matters only if the first process can still write after the second one opens, and that needs a lock that was lost.

## Options & trade-offs

### Option A - keep ADR 0006, with the `EPOCH` file

- **For:** every process gets a new epoch, even if it writes nothing.
- **Against:** one more file, a rename and two syncs at every open, plus the crash cases between them. It guards against a lost lock, which needs an unsupported filesystem or a bug.

### Option B - derive the epoch from the newest segment

- **For:** no file, no rename, and no syncs at open for the epoch. The epoch still lives in files (I9), in the segment header and in every frame.
- **Against:** two processes can get the same epoch if the first one wrote nothing. On a supported filesystem, the first one cannot write after the second one opens.

### Option C - lock only, no epoch

- **For:** simplest.
- **Against:** loses the recovery check against a frame from an older writer, a check the format already pays for.

## Decision

We will derive the writer epoch from the newest segment and drop the `EPOCH` file (option B). We accept that two processes can get the same epoch if the first one wrote nothing.

- Open takes a non-blocking exclusive lock on `LOCK` and fails with `Locked` if another process holds it.
- The writer epoch is one more than the highest epoch in the newest segment's header and its kept frames. A new log starts at epoch 1.
- Every frame stores the epoch inside the span that its CRC covers. In the active segment, a frame whose epoch is lower than the epoch of an earlier frame ends the valid prefix.

## Consequences

- [ADR 0047](0047-writer-epoch-from-every-segment.md) (proposed) amends the epoch rule for logs with several segments: the writer epoch counts every segment header and kept frame.
- The log must be on a local filesystem. A network filesystem is not supported.
- [format.md](../architecture/format.md) has no `EPOCH` file. Open ignores an `EPOCH` file that an older build left behind.
- The storage trait no longer needs rename.
- This is the residue of Kafka's leader epoch. It is not a follower-truncation protocol.

## Enforcement

None yet. Oracle check 11 in [testing.md](../architecture/testing.md#the-oracle) will check that a second open fails with `Locked`. The hand-made recovery case with a lower epoch in [testing.md](../architecture/testing.md#property-and-unit-tests) will check the prefix rule.

## Revisit if

- The log has to run on a filesystem where OS locks are not reliable. Then go back to a durable epoch counter (ADR 0006, option A).
