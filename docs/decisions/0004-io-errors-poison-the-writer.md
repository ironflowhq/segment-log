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

- `Indeterminate` is a normal result that callers must handle: reopen, then retry with the same keys.
- The crash tests inject `fsync` errors, short writes and `ENOSPC` ([testing.md](../architecture/testing.md)).

## Enforcement

None yet. Oracle check 7 in [testing.md](../architecture/testing.md#the-oracle) will check the poison rule after injected `fsync` errors, short writes and `ENOSPC`.

## Revisit if

- Poisoning on a temporary `ENOSPC` causes real outages. A free-space check before the write could help, but never a retry of `fsync`.
