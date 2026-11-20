---
id: "0069"
title: "Reads keep sparse frame tables under a byte budget, not a persisted frame index"
status: accepted
date: 2026-11-20
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [read-path, memory, performance, release-0.1]
touches: [segment-log-core/src/engine/view.rs, segment-log-core/src/engine/writer.rs, segment-log-core/src/engine/reader.rs, segment-log-core/src/acceptance/**, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md, docs/evidence/issue177/**, CHANGELOG.md]
enforced_by: test
---

# 0069. Reads keep sparse frame tables under a byte budget, not a persisted frame index

## Context

[Issue #177](https://github.com/ironflowx/segment-log/issues/177). A log, or a `LogReader`, keeps the frame tables of at most 16 sealed segments and drops the least recently used one ([api.md](../architecture/api.md#opening-a-directory)). A `read` that needs a dropped table walks that segment's frame headers again, from the header to the key filter, in 64 KiB chunks. With large groups that reads almost the whole segment. The [issue 156 record](../evidence/issue156/README.md#frame-tables) measured one `LogReader` reading 64 records per call on the M4 Mac: at 43 sealed segments a random read took 1,466 µs at the median against 187 µs for a scan. At 7 segments the two were close (206 µs and 184 µs).

These facts are fixed:

- The bound of 16 came from commit `bbc4a12`, a fix that bounded memory. No ADR chose it. Its comment sizes it for forward readers, one table each, and not for random reads.
- A table is a dense list of `GroupSpan` (position, length, base offset, count), one per group frame, 24 bytes each. Its size grows with the number of group frames. The issue 156 log holds about 100 frames per 64 MiB segment, each 0.8–0.9 MiB at the median, because the writer packs several 256-record submits into one group, so its tables are about 2.4 KiB each. Groups of one 256-byte record hold about 200,000 frames, about 4.7 MiB of table.
- A hit through a dense table reads the frame it needs from the stored position and length: one lookup read per frame, and `read` checks the frame's CRC when it decodes it.
- The first read of a sealed segment walks and checks every frame envelope and body header ([open step 5](../architecture/overview.md#open-and-recovery)). A sealed segment never changes after that.
- `SegmentLog` and each `LogReader` have their own view, so their own tables. A roll hands the writer's table of the segment it seals to the view. The newest segment's spans live apart from the sealed tables.
- Format version 3 is the only version. Every later build must read it ([ADR 0064](0064-the-0-1-compatibility-promise.md)). A new version is a one-way door: 0.1.x builds can't open logs it writes.
- [ADR 0051](0051-a-log-key-filter-in-memory-in-front-of-the-segment-filters.md) states 1,000 sealed segments as the operating limit for the key path.

## Options & trade-offs

### Option A - today's dense tables under a byte budget

- **For:** the smallest change. Hits stay at one read per frame. With the issue 156 groups, the tables of 1,000 segments fit in about 2.4 MiB.
- **Against:** memory follows the frame count, so how callers group their records sets how much history the budget covers. With single-record groups a 24 MiB budget holds 5 segments, and random reads fall back to full walks.

### Option B - sparse tables in memory: one entry per 64 KiB of segment, under a byte budget

- **For:** a table's size follows the segment's bytes, not its frame count, so the budget covers a known amount of history whatever the group size. With groups of 64 KiB or more, every frame is kept, and the table and its hits are the same as Option A. No format change.
- **Against:** with small groups, a read of a frame that is not kept reads a window of up to 64 KiB first. A reader that reads a few small frames per call pays that window on each call. The window walk is new code on the read path.

### Option C - persist a frame index in each sealed segment

- **For:** a miss reads one small index frame. Nothing to rebuild after open.
- **Against:** format version 4. The writer and the sealer write a new frame, roll and recovery gain crash cases, every later build reads v3 without the index and v4 with it, and 0.1.x builds can't open v4 logs. Option B answers the measured cost in memory.

### Option D - share tables between the views on one directory

- **For:** N `LogReader`s on one directory would hold one budget, not N.
- **Against:** the views are in different processes as often as in one, and a shared cache in one process needs a registry keyed by directory and log id. It does not change the cost of a miss. Nobody has reported many readers on one directory.

## Decision

We will build Option B.

**Positions.** Boundaries and positions are byte positions in the segment file, counted from its start, so the segment header sits before the first boundary's first frame. "A header" below means a frame envelope plus its group body header, 32 bytes.

**Entries.** When the first read of a sealed segment walks it, or when a roll hands over the table of the segment it seals, the view keeps, for each 64 KiB boundary (`WALK_CHUNK`), the first group frame that starts at or after it. A boundary whose first frame is also the first frame of a later boundary yields one entry. Every frame that starts past a boundary with no frame start between them is kept, so consecutive frames of 64 KiB or more are each kept, and a segment of such frames keeps a dense table. An entry is the existing 24-byte `GroupSpan`; it stores no epoch. A table of a segment of `b` bytes holds at most `b / 64 KiB + 1` entries, about 24 KiB for a 64 MiB segment.

**Lookup.** To find offset `o`, a `read` takes the last kept entry `k` whose base offset is at or below `o`.

- If `k` holds `o`, the `read` reads that frame from `k`'s position and length, one read, as today.
- Otherwise `o` lies in a frame between `k` and the next kept entry `k+1` (or the end of the groups). The `read` reads one window from the end of `k`'s frame to `k+1`'s position, cut at `k`'s position plus 64 KiB plus one header. Every frame between `k` and `k+1` starts before the boundary after `k`'s, which lies within 64 KiB of `k`'s position, so the window holds every header it needs. Only the last frame's body can run past it. When `k`'s frame ends at `k+1`'s position, the window is empty and is not read.
- The `read` walks the headers in the window and fails with `ReadError::Corrupt` at the frame where any of these breaks: the body length is in range for a group; the kind is group; the first frame's base offset is `k`'s next offset, and each later one follows its predecessor with no gap; the epoch does not go down within the window and lies inside the segment's epochs; the last frame ends exactly at `k+1`'s position, with `k+1`'s base offset next, or, after the last kept entry, at the end of the groups, with the segment's next offset next. Because no epoch is stored, the epoch floor at a window's start is the segment's lowest epoch, not the epoch of `k`. The target frame's CRC still guards every record a `read` returns.
- A frame that lies wholly inside the window is decoded from the window's bytes. A frame that runs past it is read alone.
- A `read` that needs more frames goes on through the following kept entries and windows the same way.

So a `read` costs one lookup read for each kept frame it uses, plus one for each window it walks, plus one for each frame that runs past its window.

**Damage after the first walk.** A window check that fails returns `Corrupt` for that `read`, and the view keeps the table. The segment does not change, so walking it again would fail at the same frame. While the table is kept, reads that need other windows of the segment still succeed. Reads that need any frame of the damaged window fail, where today only reads of the damaged frame fail. A CRC failure when a frame is decoded also keeps the table, as today. For header damage, once the budget drops the table, the next read walks the segment, fails at the damaged frame, and keeps no table, so every read of the segment fails, as it does today after a dropped table. A walk does not check CRCs, so damage inside a frame body still fails only reads of that frame.

**Budget.** The view charges each sealed segment's table its entries plus a fixed allowance for the table itself. When the charges pass 24 MiB, it drops the least recently used tables until they fit, but it always keeps the most recently used or handed-over table, even if that table alone exceeds the budget. A lookup must not scan every kept table, because the view now keeps thousands, not 16. The budget is a `pub(crate)` const, for each log and each `LogReader`, with no new `Options` field. The newest segment's spans are outside it, as today. 24 MiB covers about 64 GiB of sealed history: 1,000 segments at the [default roll](../architecture/api.md).

**Done when.** The `docs/evidence/issue177/` record runs the issue 156 driver at the pre-change commit `0a11998` and at the change, on the same host and the same logs, with one discarded pass that walks every segment before any timed run. It reports first walks separately, the frames per segment of each log, and each log's bytes against the host's RAM, with the fraction of the log resident in the page cache where the host can measure it. A random read that misses the cache can fail check 1 for reasons unrelated to tables. Such a failure still counts, and the record names the cache state beside it.

The cases, each sealed and read through `LogReader`:

- the issue 156 logs: 7, 18 and 43 segments;
- a log of about 200 segments at the default roll;
- the log of 1,000 single-record callers, whose groups the writer still packs from several callers;
- a log written by one caller with one record per group, the case with the most frames per window: 20 segments at a 4 MiB roll, about 250,000 frames, about 30 minutes to build at the 145 flushes per second measured on the M4.

The record passes when all of these hold:

1. For every case: random-read p50 at most 2× the pre-change scan p50, and random-read p99 at most 3× the pre-change scan p99.
2. For every case: scan p50 and p99 at most 10% slower than the pre-change scan. Faster is fine.
3. The 43-segment, 200-segment and 1,000-caller cases fail check 1 at the pre-change commit. Any other case that already passes check 1 there is reported and excluded from the verdict on the change's benefit, but must still pass checks 1 and 2 after it. On the M4, the 18-segment case fails before the change only on p99. The one-record-per-group case may pass before the change, because a 4 MiB walk is cheap. It is there to show that windows do not make reads slower.

The record decides on the M4 Mac and reruns on the qualified host. A failure there reopens this decision (Revisit if). The owner accepted on 2026-11-20 that the measured cases stop near 314 segments. The 1,000-segment figure stays arithmetic, and eviction at the production budget is tested in simulation.

We chose this because it brings random reads in line with scans without a format version, and the sealed tables' memory has a fixed bound that does not depend on how callers group their records. Option A would do the same for large groups but not for small ones. We accept a window read for frames that are not kept, wider read failures after damage in a sealed segment, and full walks again past 64 GiB of history.

## Consequences

- [ADR 0070](0070-read-checks-are-judged-on-logs-the-page-cache-holds.md) amends done-when check 1: a case whose log the page cache does not hold, and that fails check 1, is reported and judged by a log of the same shape that the cache holds.
- Accepted by the owner on 2026-11-20 on the design, before the evidence runs ([PR #184](https://github.com/ironflowx/segment-log/pull/184)). The qualified-host rerun confirms it rather than gating it.
- api.md replaces "at most 16 frame tables" with the byte budget and the sparse entry, and overview.md open step 5 adds the window check and its failure, in the change that implements them.
- With groups of 64 KiB or more, reads cost what they cost today, and random reads past 16 segments stop walking.
- With small groups, a read that uses few frames reads up to 64 KiB more than today. A read that uses many frames reads fewer, larger blocks. Done-when check 2 measures the scan side.
- The memory bound depends on history bytes. Each `LogReader` on one directory holds its own budget, so N readers can hold N × 24 MiB. The newest segment's dense spans add to it, as they do today.
- Damage in a sealed segment after its first walk fails reads of a whole window, not one frame, while the table is kept, and of the whole segment once the budget drops it.
- The table store changes from a 16-entry list to one that finds a table without a full scan.
- Ships in a 0.1.x patch. [CHANGELOG.md](../../CHANGELOG.md) lists it under Changed. No API, format version or protocol version changes.
- The roll hands over a filtered table and changes no I/O, so the write and recovery paths gain no crash cases.
- I1–I10 are unchanged. The tables stay caches that the log rebuilds from the segments ([overview.md](../architecture/overview.md)).
- The done-when cases reach about 314 segments. The 1,000-segment and 64 GiB figures are arithmetic from the entry size, not measured.

## Enforcement

Scenarios on simulated storage, written to fail before the change:

- `READ-BOUNDED`, in `reads_keep_a_bounded_number_of_frame_tables` in `segment-log-core/src/acceptance/summary_open.rs`, on segments of many frames. It asserts exact `Purpose::Lookup` read counts: 1 for a kept frame, 1 for a frame inside its window, 2 for a frame that runs past its window. The view takes its budget as a field, so the scenario sets a small one. Production uses the const. Given more segments than the budget holds, a dropped table's segment is walked again, and a budget smaller than one table still keeps the table just used.
- In the same file: a frame inside a window that was valid on the first walk and damaged afterwards fails reads of that window with `ReadError::Corrupt`, without a walk of the segment. Reads of other windows of the segment still succeed.
- A seeded oracle check in `segment-log-core/src/acceptance/seeded.rs`: random reads over many sealed segments of mixed group sizes return the oracle's records. A failing seed prints and saves its storage trace.

The `docs/evidence/issue177/` record holds the done-when checks above.

## Revisit if

- The done-when checks fail on the qualified host. Then reopen Option C under a new format version.
- Logs routinely exceed 64 GiB of sealed history and read it at random.
- Readers of small groups report the window read as a cost.
- Many `LogReader`s on one directory make the per-reader budget a real cost. Then reopen Option D.
