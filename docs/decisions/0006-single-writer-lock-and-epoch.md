---
id: "0006"
title: "Single writer: an OS lock and a writer epoch"
status: superseded
date: 2026-11-07
deciders: []
supersedes: null
superseded_by: "0018"
tags: [concurrency, recovery]
touches: []
enforced_by: none
---

# 0006. Single writer: an OS lock and a writer epoch

## Context

Source: the lock, and the epoch covered by the CRC, are from the reviewed design (decision 1 and "File and recovery"). The `EPOCH` file is new in this repository and not reviewed. The design said only that the epoch is "a durable fact", not where it lives.

Two live writers on one directory would interleave frames and break every invariant. A writer epoch lets recovery reject a frame from an older writer that appears after a frame from a newer one. A stored epoch alone does not stop a live writer that has already passed its check, and it does not revoke a writable mapping. So the lock is the main guard and the epoch is a check at recovery.

## Options & trade-offs

### Option A - an exclusive OS lock on `LOCK`, and an epoch kept in an `EPOCH` file that open increments durably

- **For:** the lock stops a second live writer on a local filesystem. The epoch is a durable fact (invariant I9). Each process gets a new epoch even if it writes nothing.
- **Against:** one more file and one more `fsync` at open. An OS lock is advisory and can be weak on network filesystems.

### Option B - lock only, no epoch

- **For:** simpler.
- **Against:** no check at recovery against a stale writer, for example one that lost its lock on a network filesystem.

### Option C - derive the epoch from the highest epoch seen in the segments

- **For:** no `EPOCH` file.
- **Against:** a process that wrote nothing leaves no trace, so the next process can get the same epoch.

## Decision

We will guard the single writer with an exclusive OS lock and keep the writer epoch in an `EPOCH` file (option A). We accept one more file and one more `fsync` at open. Open takes a non-blocking exclusive lock on `LOCK` and fails with `Locked` if another process holds it. Then it replaces `EPOCH` with the old value plus one (write a temporary file, `fsync`, rename, `fsync` the directory). Every frame stores the epoch inside the span that its CRC covers. In the active segment, a frame whose epoch is lower than the epoch of an earlier frame ends the valid prefix. The layout of `EPOCH` was in format.md until [ADR 0018](0018-writer-epoch-from-the-segments.md) removed the file.

## Consequences

- The log must be on a local filesystem. A network filesystem is not supported.
- This is the residue of Kafka's leader epoch. It is not a follower-truncation protocol.

## Enforcement

None. [ADR 0018](0018-writer-epoch-from-the-segments.md) replaced this decision before any code enforced it.

## Revisit if

- The log has to run on a filesystem where OS locks are not reliable.
