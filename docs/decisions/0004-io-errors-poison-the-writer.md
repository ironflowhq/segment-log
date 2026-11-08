---
id: "0004"
title: "I/O errors poison the writer"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [write-path, errors]
touches: [docs/architecture/api.md]
enforced_by: none
---

# 0004. I/O errors poison the writer

## Context

Source: new in this repository. Not reviewed. The source design left open "what a failed fsync or a short write does to other admitted submits".

On Linux, when `fsync` fails, the kernel can mark the dirty pages clean and drop them. A second `fsync` can then succeed even though the data never reached the disk. PostgreSQL found this in 2018 and changed to crash and recover on an `fsync` error. A short write or `ENOSPC` can leave part of a frame in the file.

## Options & trade-offs

### Option A - poison the writer until reopen

- **For:** the state on disk is unknown, and this option does not guess. Recovery on reopen decides from the CRC which frames survived. It is simple to test.
- **Against:** one bad `fsync` stops all appends until the process reopens the log.

### Option B - retry the `fsync`

- **For:** the writer keeps running.
- **Against:** a success after a failure does not prove durability. This can ack data that is lost.

### Option C - fail only the affected group and continue

- **For:** other callers keep going.
- **Against:** the file can hold part of the failed frame. A later frame written after it is either lost behind the bad bytes or makes the bad bytes look valid. Both break the prefix rule.

## Decision

We will poison the writer until the log is reopened (option A). We accept that one bad `fsync` stops all appends until the reopen. After any I/O error on the write path, including a roll and a seal (for example from create, write, `sync_data`, `sync_dir` or truncate):

- every submit in the group that was in flight gets `Indeterminate`;
- every later submit, and every submit still in the queue, gets `Poisoned`;
- the writer never calls `fsync` again on that file handle.

The caller must drop the log and open it again. Recovery then keeps the valid prefix ([ADR 0003](0003-recovery-keeps-the-checksum-valid-prefix.md)).

## Consequences

- [ADR 0038](0038-m1-admission-and-recovery-boundaries.md) and the [API](../architecture/api.md#admission-and-answer-ordering) make current-group lookup failures and finalized answers explicit. A durable-key lookup read error, unexpected EOF, bad CRC or malformed frame poisons the writer. Unfinished requests in the current group, including the lookup request, get `Indeterminate` even before a write. Queued requests get `Poisoned`. A result already finalized does not change.
- [ADR 0021](0021-exact-pending-retries-wait-for-the-original.md) clarifies that a retry matched to an in-flight submit gets `Indeterminate` if the original does. Requests still in the queue get `Poisoned` as before.
- `Indeterminate` is a normal result that callers must handle. Reopen, then resolve fully keyed requests with the same keys and bytes. [Mixed and unkeyed requests](../architecture/api.md#retry-after-an-unknown-outcome) cannot rely on full-submit deduplication.
- The crash tests inject `fsync` errors, short writes and `ENOSPC` ([testing.md](../architecture/testing.md)).

## Enforcement

None yet. Oracle check 7 and the durable-key lookup cases in [testing.md](../architecture/testing.md#the-oracle) will check the poison rule after injected flush, write and lookup failures.

## Revisit if

- Poisoning on a temporary `ENOSPC` causes real outages. A free-space check before the write could help, but never a retry of `fsync`.
