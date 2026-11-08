---
id: "0016"
title: "A read-only reader for sealed logs that takes no lock"
status: proposed
date: 2026-11-07
deciders: []
supersedes: null
superseded_by: null
tags: [api, read-path]
touches: [docs/architecture/api.md]
enforced_by: none
---

# 0016. A read-only reader for sealed logs that takes no lock

## Context

Source: new in this repository, from a review of the docs on 7 November 2026. Not reviewed by the source design.

The PRD names a reader that opens sealed segments from a surviving volume after the guest is gone. Where that volume is mounted, and who reads it, is still PRD open item 4. `SegmentLog::open` serves only a writer. It creates the directory, takes the exclusive lock, and can truncate, rewrite and create segments ([ADR 0015](0015-open-makes-the-recovered-tail-durable.md)) before it learns that the log is sealed. So:

- on a read-only mount, open fails with `Io`;
- a second reader gets `Locked`. `File::try_lock` refuses a second handle even inside one process (observed on macOS APFS with Rust 1.99.1);
- on a log whose guest died unsealed, the reader truncates the tail and becomes a writer.

A reader that writes nothing cannot make anything durable. On an unsealed log, after a `kill -9` earlier in the same boot, it would read frames from the page cache whose `fsync` never returned. A power loss could then drop records that the reader already showed, which breaks I1.

## Options & trade-offs

### Option A - a separate `LogReader` type for sealed logs only, with no lock and no writes

- **For:** works on a read-only volume and with many readers at once. The type has no `append` and no `seal`. A seal frame is written only after every group's `fsync` returned, so every record it shows is durable and I1 holds.
- **Against:** an unsealed log needs one writer pass first: `SegmentLog::open` on a writable volume, then `seal`.

### Option B - a `LogReader` for any log

- **For:** reads a crashed guest's log without a writer pass.
- **Against:** breaks I1 on an unsealed log in the same boot as the writer. The library cannot tell whether the writer's page cache is gone.

### Option C - `open` with a shared lock

- **For:** refuses to read while a writer is live.
- **Against:** a reader blocks the writer from reopening. On a read-only mount, a missing `LOCK` file cannot be created.

## Decision

We will add a separate `LogReader` type for sealed logs only, with no lock and no writes (option A). We accept that an unsealed log needs one writer pass first. `LogReader::open(dir)` checks the headers and reads every segment, but it creates, deletes, truncates, syncs and locks nothing. If the log is not sealed, it fails with `NotSealed`. Its watermark is the log's end. The signatures are in [api.md](../architecture/api.md).

## Consequences

- To read a crashed guest's log, open it with `SegmentLog::open` on a writable volume and call `seal`. Recovery makes the tail durable first.
- The reader uses only the read calls of the storage trait.
- The tests open a reader on storage that refuses every write, open two readers at once, and check that an unsealed log fails with `NotSealed`.
- `LogReader` is M4 work, with seal.

## Enforcement

None yet. The `LogReader` tests in [testing.md](../architecture/testing.md#property-and-unit-tests) will open a reader on storage that refuses every write, open two readers at once, and expect `NotSealed` on an unsealed log.

## Revisit if

- The volume that outlives the guest must be read-only everywhere, so no writer pass is possible (PRD open item 4).
