---
id: "0015"
title: "Open makes the recovered tail durable"
status: proposed
date: 2026-11-07
deciders: []
supersedes: null
superseded_by: null
tags: [recovery, durability]
touches: [docs/architecture/overview.md]
enforced_by: none
---

# 0015. Open makes the recovered tail durable

## Context

Source: new in this repository, from a review of the docs on 7 November 2026. Not reviewed by the source design.

[ADR 0003](0003-recovery-keeps-the-checksum-valid-prefix.md) has recovery call `fsync` only when it truncates. In three cases, recovery keeps bytes that it can read but that are not on the device:

- **`kill -9` between the write and the `fsync`.** The frame is dirty in the page cache. The scan keeps it, the watermark moves past it, readers see it, and a retry of its keys gets `deduplicated: true`. A power loss then removes it. This breaks I1 and I2.
- **A failed `fsync`.** On Linux, a writeback error can mark the pages clean while they stay readable. PostgreSQL hit this in 2018. A reopen in the same boot keeps the frame, and a later `fsync` returns success without writing those pages. Every group acknowledged after the reopen sits behind bytes that a power loss drops, so the valid prefix ends before them. This breaks I3. It is inferred from documented kernel behavior and was not tested here.
- **A crash during a roll, after the new segment's header is written and before the directory `fsync`.** Recovery adopts the new segment. Acks after that depend on a directory entry that a power loss can drop.

A plain `fsync` at open fixes only the first case, because it does not write clean pages.

Only a small tail can be unsynced. The writer syncs each group before it writes the next frame, and it writes nothing after an error. A roll or a seal writes its closing frames (footer, seal, locator) only after the last group's `fsync` returned, then syncs once. A new segment's header is synced before any frame follows it. So the bytes that can be unsynced are, in the newest segment, either the trailing closing frames, or the last group frame, or the header of a segment with neither. The directory entries of the newest segment and of the log directory can be unsynced too. This holds at every open only if every open ends with that tail durable.

## Options & trade-offs

### Option A - always `sync_data` the active segment at open

- **For:** one call.
- **Against:** fixes only `kill -9`. Does nothing for clean pages or for the directory entry.

### Option B - rewrite the tail that can be unsynced, then sync the file and the directory

- **For:** covers all three cases. Costs one write of one group frame or of the closing frames, plus three syncs, per open. The closing frames include a footer of up to one frame body.
- **Against:** one more write at open that the crash tests must cover.

### Option C - rewrite every kept frame of the active segment

- **For:** needs no reasoning about which bytes can be unsynced.
- **Against:** up to the roll size (64 MiB by default) written at every open, against the open-time target.

### Option D - read the device directly with `O_DIRECT`

- **For:** shows what is really on the device.
- **Against:** needs `libc` and aligned buffers, not `std` ([ADR 0012](0012-no-log-crate-dependency.md)). Does not fix anything by itself.

## Decision

We will rewrite the tail that can be unsynced, then sync the file and the directory (option B). We accept one more write at open that the crash tests must cover. After the scan and any truncation, and before it publishes the watermark, open:

1. writes back, at the same position and with the same bytes, the trailing closing frames of the newest segment if it ends with any. Otherwise it writes back the last group frame. If the segment has neither, it writes back the header.
2. calls `sync_all` on the segment. `sync_all` also commits a truncation that an earlier open made but did not sync before a crash ([ADR 0005](0005-flush-calls-and-platforms.md)).
3. calls `sync_dir` on the log directory and on its parent.

If any of these calls fails, open fails with `Io`. It does not retry ([ADR 0004](0004-io-errors-poison-the-writer.md)). The rewrite puts back identical bytes, so a sealed segment's content does not change and I7 holds. The steps are in [overview.md](../architecture/overview.md#open-and-recovery).

The rewrite also touches bytes that are already durable and acknowledged, for example after a clean shutdown. A power loss during the rewrite must not damage them. This design assumes that the device writes each sector atomically, so that a sector holds either its old or its new contents after a power loss. Old and new are the same bytes, so the data is unchanged. NVMe defines an atomic write unit of at least one logical block, but this design has not checked that against the specification. It is not verified for the guest's virtio disk, which is PRD open item 3.

A directory entry cannot be written back like file bytes. If a directory `fsync` fails and the log is reopened in the same boot, the next `sync_dir` may succeed without writing the entry. This design assumes that the filesystem stops accepting writes after a metadata write error. That assumption is not verified for ext4 or xfs, and it depends on their mount options.

## Consequences

- [ADR 0032](0032-qualify-deployment-storage-before-m6.md) keeps the rewrite's storage assumptions explicit. Qualification of the actual guest stack is required before M6 guest validation and production durability claims; M1 tests the conditional contract without claiming that qualification.
- R14 makes the kept prefix durable, not only the truncation.
- The storage trait needs a write at a position.
- The simulated storage keeps bytes written before a `Kill` volatile. After a failed `sync_data`, it keeps the volatile bytes readable, and a later successful `sync_data` does not make them durable. Only a new write to those bytes does ([testing.md](../architecture/testing.md)).

## Enforcement

None yet. Oracle check 5 in [testing.md](../architecture/testing.md#the-oracle) will check that a `PowerLoss` right after open gives the same log. The hand-made recovery cases in [testing.md](../architecture/testing.md#property-and-unit-tests) cover `PowerLossTorn` during the rewrite.

## Revisit if

- A measurement shows that the rewrite makes open miss its target.
- A kernel or filesystem that the log supports changes how it reports writeback errors.
- A supported device can tear a single sector. Then a rewrite in place can damage acknowledged data, and open needs another way to make the tail durable.
