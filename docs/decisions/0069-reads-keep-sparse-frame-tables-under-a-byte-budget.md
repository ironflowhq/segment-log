---
id: "0069"
title: "Reads keep sparse frame tables under a byte budget, not a persisted frame index"
status: proposed
date: 2026-11-20
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [read-path, memory, performance, release-0.1]
touches: [segment-log-core/src/engine/view.rs, segment-log-core/src/engine/writer.rs, segment-log-core/src/engine/reader.rs, docs/architecture/api.md, docs/architecture/testing.md, docs/evidence/issue177/**]
enforced_by: test
---

# 0069. Reads keep sparse frame tables under a byte budget, not a persisted frame index

## Context

[Issue #177](https://github.com/ironflowx/segment-log/issues/177). A log, or a `LogReader`, keeps the frame tables of at most 16 sealed segments and drops the least recently used one ([api.md](../architecture/api.md#opening-a-directory)). A `read` that needs a dropped table walks that segment's frame headers again, from the header to the key filter, in 64 KiB chunks. With large groups that reads almost the whole segment. The [issue 156 record](../evidence/issue156/README.md#frame-tables) measured one `LogReader` reading 64 records per call on the M4 Mac: at 43 sealed segments a random read took 1,466 µs at the median against 187 µs for a scan. At 7 segments the two were close (206 µs and 184 µs).

These facts are fixed:

- The bound of 16 came from commit `bbc4a12`, a fix that bounded memory. No ADR chose it. Its comment sizes it for forward readers, one table each, and not for random reads.
- A table is a dense list of `GroupSpan` (position, length, base offset, count), one per group frame. Its size grows with the number of group frames, so with the group size. A 64 MiB segment of 64 KiB groups holds about 1,000 frames, about 32 KiB of table. Single-record groups can hold a few hundred thousand frames.
- The first read of a sealed segment walks and checks every frame envelope and body header ([open step 5](../architecture/overview.md#open-and-recovery)). `read` checks each frame's CRC when it decodes it.
- `SegmentLog` and each `LogReader` have their own view, so their own tables. A roll hands the writer's table of the segment it seals to the view.
- Format version 3 is the only version. Every later build must read it ([ADR 0064](0064-the-0-1-compatibility-promise.md)). A new version is a one-way door: 0.1.x builds can't open logs it writes.
- [ADR 0051](0051-a-log-key-filter-in-memory-in-front-of-the-segment-filters.md) states 1,000 sealed segments as the operating limit for the key path.

## Options & trade-offs

### Option A - cache more tables, bounded by count or by bytes

- **For:** the smallest change. With large groups, every table of 1,000 segments fits in about 32 MiB.
- **Against:** a table's size follows the group size, so a fixed byte budget holds many tables of large groups and only a few of single-record groups. A miss still walks the whole segment.

### Option B - sparse tables in memory: one entry per 64 KiB of segment, under a byte budget

- **For:** a table's size follows the segment's bytes, not its frame count, so the budget covers a known amount of history whatever the group size. No format change. A miss is no worse than today.
- **Against:** a read through a kept table reads a window of up to 64 KiB before it finds its frame: at most 2 lookup reads instead of 1. The window walk is new code on the read path.

### Option C - persist a frame index in each sealed segment

- **For:** a miss reads one small index frame. Nothing to rebuild after open.
- **Against:** format version 4. The writer and the sealer write a new frame, roll and recovery gain crash cases, every later build reads v3 without the index and v4 with it, and 0.1.x builds can't open v4 logs. It answers a read cost that Option B already brings in line in memory.

## Decision

We will build Option B.

**Entries.** When the first read of a sealed segment walks it, or when a roll hands over the table of the segment it seals, the view keeps one `GroupSpan` for each 64 KiB boundary of the segment (`WALK_CHUNK`): the first group frame that starts at or after that boundary. A boundary with no frame starting before the next boundary keeps no entry. A frame of 64 KiB or more is kept when it starts after a boundary, so a segment of large groups keeps a dense table, as today. `GroupSpan` is reordered to 24 bytes. A table of a segment of `b` bytes holds at most `b / 64 KiB + 1` entries, about 24 KiB for a 64 MiB segment.

**Lookup.** To find offset `o`, a `read` takes the last kept entry whose base offset is at or below `o`, and reads one window: from its position to the next kept entry's position (or to the end of the groups for the last entry), cut at 64 KiB plus one frame header. Every frame between two kept entries starts within 64 KiB of the first, because a frame that starts past the next boundary is itself kept. So the window holds every header it needs, and only the last frame's body can run past it. It walks the frame headers in the window, and fails that `read` with `ReadError::Corrupt` at the frame where any of these breaks: the body length is in range for a group, the kind is group, the base offset follows the previous frame's offsets with no gap, the epoch does not go down and stays inside the segment's epochs, and the window ends exactly at the next kept entry's position and base offset. If the frame that holds `o` lies wholly inside the window, the `read` decodes it from those bytes. Otherwise it reads that frame alone. A `read` that needs more frames continues the same way through the following windows. A `Corrupt` result keeps no table, so a later read walks the segment again, as today.

**Budget.** The view keeps tables until their entries cost more than 24 MiB, then drops the least recently used tables until they fit. The budget is a `pub(crate)` const, the same for each log and each `LogReader`, with no new `Options` field. 24 MiB covers about 64 GiB of sealed history: 1,000 segments at the default roll.

**Done when.** On the issue 156 workload through `LogReader`, the random-read p50 is at most 2× the scan p50, and the random-read p99 is at most 3× the scan p99, for up to 1,000 sealed segments. The record decides on the M4 Mac and reruns on the qualified host before this ADR is accepted.

We chose this because it brings random reads in line with scans up to the stated segment limit, without a format version. Its memory has a fixed bound that does not depend on how callers group their records. We accept a second lookup read on some hits, and a full walk again past 64 GiB of history.

## Consequences

- api.md replaces "at most 16 frame tables" with the byte budget and the sparse entry, in the change that implements it.
- A random read touches each sealed segment's full walk once per budget lifetime, instead of on nearly every call past 16 segments.
- The memory bound now depends on history bytes. Each `LogReader` on one directory holds its own budget, so N readers can hold N × 24 MiB.
- A hit through a kept table reads up to 64 KiB more than it does today. Sequential scans read those bytes anyway.
- Ships in a 0.1.x patch. [CHANGELOG.md](../../CHANGELOG.md) lists it under Changed. No API, format version or protocol version changes.
- I1–I10 are unchanged. The tables stay caches that the log rebuilds from the segments ([overview.md](../architecture/overview.md)).

## Enforcement

Scenarios on simulated storage in `segment-log-core/src/acceptance/`, written to fail before the change:

- `READ-BOUNDED`, in `reads_keep_a_bounded_number_of_frame_tables`: given more sealed segments than the budget holds, a read through a kept table costs at most 2 `Purpose::Lookup` reads, and a read of a dropped table walks that segment again. The view takes its budget as a field, so the scenario sets a small one; production uses the const.
- A frame inside a window that was valid on the first walk and damaged afterwards fails that `read` with `ReadError::Corrupt`, and a later `read` walks the segment again.
- A seeded oracle check: random reads over many sealed segments return the oracle's records. A failing seed prints and saves its storage trace.

The `docs/evidence/issue177/` record measures the done-when check with the issue 156 driver, extended by a 1,000-segment case at a 4 MiB roll, and it reruns the case of 1,000 single-record callers.

## Revisit if

- The done-when check fails on the qualified host. Then reopen Option C under a new format version.
- Logs routinely exceed 64 GiB of sealed history and read it at random.
- Many `LogReader`s on one directory make the per-reader budget a real cost. Then share tables between readers.
