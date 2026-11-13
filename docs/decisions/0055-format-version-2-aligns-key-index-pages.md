---
id: "0055"
title: "Format version 2 aligns key index pages to 4 KiB and drops the unbuilt sparse index"
status: accepted
date: 2026-11-13
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [format, performance]
touches: [segment-log-core/src/codec/index.rs, segment-log-core/src/codec/filter.rs, segment-log-core/src/codec/header.rs, segment-log-core/src/engine/keys.rs, segment-log-core/src/engine/closing.rs, segment-log-core/tests/fixtures/**, docs/architecture/format.md]
enforced_by: test
---

# 0055. Format version 2 aligns key index pages to 4 KiB and drops the unbuilt sparse index

## Context

[ADR 0052](0052-freeze-format-version-1.md) kept format version 1, where no key index page sits on a 4 KiB boundary, and rejected aligned pages (its option B) without a measurement. [ADR 0053](0053-format-changes-need-no-older-reader-before-production.md) lets a format change before production bump the version with no reader for older versions.

These facts are fixed:

- A key index page is 4,096 bytes with its own CRC ([ADR 0048](0048-sealed-segments-carry-a-key-filter-and-index.md)). It is the unit that one lookup reads: a lookup in a sealed segment opens the file, reads one page at its file position, and closes it (`storage/real.rs`).
- The key index frame follows the key filter frame with no gap. Its pages start 32 bytes into the frame, at a position that depends on every byte before them.
- The [alignment measurement](../evidence/format/key-page-alignment/README.md) read 2,000 cold random pages per run on a hosted runner with local NVMe and ext4. An unaligned page cost one device read, as an aligned one did, but moved 8 KiB instead of 4 KiB. Its p50 was about 3% higher (135.8 µs against 131.9 µs). The p99 differences were within the run-to-run spread, and readahead changed nothing.
- Cold lookups happen most after a restart, when callers retry the keys of an unknown outcome and each retry reads a page from a cold cache.
- format.md also describes a sparse index file (`.idx`). No code writes or reads it. It has no header, no version and no CRC. Reads find frames through in-memory frame tables.

## Options & trade-offs

### Option A - keep version 1

- **For:** no change.
- **Against:** every cold lookup reads twice the bytes it needs, and the unit of a lookup's I/O does not match the device's. An aligned page is also what direct I/O would need.

### Option B - pad the end of the key filter frame's body

- **For:** the pages start on a 4 KiB boundary, and frames still follow with no gap, so the scan and the closing-run rules stay as they are. The padding is zero, shorter than a page, and derived from the frame's position, so the writer and a reader compute the same length. A segment with no keyed record has no pages and no padding.
- **Against:** up to 4,095 bytes per sealed segment, about 0.006% of a 64 MiB segment. A decoder that does not know the frame's position can bound the padding but not check its length, so the reader that knows the position checks the alignment.

### Option C - a padding frame between the key filter and the key index

- **For:** the key filter's body stays as it was.
- **Against:** a new frame kind, which the closing-run shape, the scan and the recovery rules must all accept, for the same bytes on disk.

### Option D - align every frame

- **For:** any frame starts on a boundary.
- **Against:** padding for every group, a cost on the write path that no measurement asks for. Group reads are not page reads.

For the sparse index:

### Option E - keep its section, marked "not frozen"

- **For:** it records an idea.
- **Against:** a specification of a file that nothing writes, without the header, version and CRC that a file on disk needs. format.md is the only source for widths, and an unbuilt file in it reads as a promise.

### Option F - remove it from format.md

- **For:** format.md describes only what the code writes. A persistent index, if one is ever needed, gets its own decision with a versioned file.
- **Against:** none beyond losing the sketch.

## Decision

We will make format version 2 with options B and F.

1. The key filter frame's body ends with `pad` zero bytes, from 0 to 4,095. `pad` is the smallest count that puts the first key index page on a multiple of 4,096. A frame with no keyed record has `pad` 0.
2. A decoder accepts a key filter body whose padding is zero, shorter than a page, and empty when `count` is 0. A reader that knows the frame's position, which is open for each sealed segment and the closing-run check for the newest ([ADR 0054](0054-a-closing-run-must-match-its-groups.md)), requires the pages to start on a 4 KiB boundary, or fails with `Corrupt`.
3. `format_version` becomes 2. No build reads version 1, under ADR 0053.
4. format.md drops the sparse index section and its directory entry.

B halves the bytes of every cold lookup and makes a page the unit of I/O that its design meant, for under 4 KiB per sealed segment and no new frame kind. We accept that the measured latency gain is small on this hardware; the case rests on the I/O shape.

## Consequences

- `encode_key_frames` takes the key filter's file position.
- The `FORMAT-GOLDEN` fixture moves to `format-v2/`. It is written from a history: creation at epoch 1, a reopen at epoch 2, rolls, and a seal, with the log id of format.md's check values. The simulated storage can return given bytes for a random-bytes call (`Fault::Random`), which the trace saves and a replay repeats.
- The saved M1 seeded trace (`docs/evidence/m1/seeded/pressure-1000.trace`) has its three segment headers rewritten to version 2. Its frames do not change, because frame CRCs do not cover the version.
- ADR 0048 and ADR 0052 gain bullets that link here.

## Enforcement

- `CODEC-GROUP` checks that the pages start on a 4 KiB boundary for several key filter positions, that a filter with no keyed record has no padding, and that a decoder rejects padding that is not zero, a page long, or behind an empty filter.
- `RECOVERY-FIXTURES` checks that a sealed segment whose key filter is valid on its own bytes but leaves the pages off a boundary fails open with `Corrupt`.
- `FORMAT-GOLDEN` pins the version 2 bytes.

## Revisit if

- A target device reads in units larger than 4 KiB, such as 16 KiB pages, which would change the boundary.
- A persistent offset index is needed, which needs its own decision.
