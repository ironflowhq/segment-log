---
id: "0061"
title: "Appends do not wait for the key frames of a roll: a sealer thread builds them, the writer writes them later"
status: accepted
date: 2026-11-16
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [core, writer, roll, recovery, latency]
touches: [segment-log-core/src/engine/**, segment-log-core/src/codec/**, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: test
---

# 0061. Appends do not wait for the key frames of a roll: a sealer thread builds them, the writer writes them later

## Context

[ADR 0048](0048-sealed-segments-carry-a-key-filter-and-index.md) seals each rolled segment with a key filter frame and a key index frame, and the writer builds them at roll. It estimated about 8 ms of writer work per roll at S1 and accepted that "appends wait for that roll".

The [issue 49 record](../evidence/issue49/README.md) measured that work on bare metal (i7-7700, Samsung PM981, 64 MiB segments, every record keyed): every roll stalls the writer for about 70–130 ms. The storage calls of a roll take about 5 ms of it. At 50,000 records/s the paced p99 is 83 ms against a bound of 14.6 ms, 0 of 5 runs within it ([issue 107](https://github.com/ironflowx/segment-log/issues/107)). In every run the slow groups land exactly at the roll boundaries.

A profile in a Linux VM (about 32 ms per roll) splits the work: hashing every key of the active map into index entries about 27 %, sorting, building the filter and encoding the pages about 18 %, clearing the map and dropping about 200,000 heap keys about 20 %, and unattributed allocation, free and copy about 33 %. No step dominates. Every step is O(keys in the segment), so speeding up steps lowers the stall but leaves it growing with the roll size, the key count and the rate.

These facts are fixed:

- Key frames are derived data. Every key in them comes from a record in the same segment, so they can be rebuilt from the records (I9).
- Recovery today requires every segment except the newest to be sealed. One without closing frames fails open with `Corrupt` ([overview.md](../architecture/overview.md#open-and-recovery) step 5), and `LogReader` fails the same way. Nothing rebuilds key frames (ADR 0048 rule 6).
- After a roll, a retry of a key in the rolled segment is found only through that segment's key index on disk ([ADR 0048](0048-sealed-segments-carry-a-key-filter-and-index.md) rule 1). A failed lookup poisons the writer ([ADR 0004](0004-io-errors-poison-the-writer.md)).
- Only the writer thread writes segment files ([ADR 0051](0051-a-log-key-filter-in-memory-in-front-of-the-segment-filters.md)). An I/O error anywhere on the write path poisons the writer, and only the writer thread can answer queued submits.
- The log-filter builder reads each rolled segment's index pages after the writer tells it about the roll. It stops silently on a bad read (ADR 0051 rule 5).
- The simulator accepts I/O from more than one thread, but its traces depend on call order. The builder runs inline in tests for that reason.

## Options & trade-offs

### Option A - Keep the seal on the writer and make each step faster

Store each key's hash at insert, drop the decode of the filter frame the writer just encoded, and free the old map in bulk.

- **For:** no change to recovery, lookups or threads. No ADR beyond this one's Consequences.
- **Against:** the stall stays O(keys). A larger roll size, more keys per segment or a higher rate brings it back. The remaining encode and CRC of about 3.5 MiB of index pages still happens at roll.

### Option B - Build the key frames incrementally during appends

Keep entries sorted and the filter filled as keys arrive, so roll only writes them.

- **For:** no second thread.
- **Against:** the index is sorted by hash, so its pages can be encoded and CRC'd only once the segment ends. Keeping a sorted structure costs every append. The roll still does O(keys) encoding.

### Option C - A sealer thread builds and writes the closing frames

- **For:** the writer does nothing at roll beyond opening the next segment.
- **Against:** a second thread writes and `fsync`s segment files. ADR 0004's error path then needs a channel back into the writer, ADR 0051's rule that only the writer writes changes, and the crash cases double, because two threads' writes interleave.

### Option D - A sealer thread builds the frames, the writer writes them at a group boundary

- **For:** the writer's CPU work at roll no longer depends on the key count. Only the writer writes files, so ADR 0004 and ADR 0051 hold as they are. The writer's remaining roll I/O is the closing frames, about 1 % of the segment, in two flushed appends, about the 5 ms the record measured plus one flush.
- **Against:** a segment can exist for a while with its successor already written and its own closing frames not yet written. Recovery, `LogReader`, lookups, the builder hand-off and `seal` must each handle that window.

## Decision

We will use option D.

1. **Roll.** At roll the writer moves the old segment's key map and the facts the footer needs to a dedicated sealer thread, then creates the next segment (roll steps 2–4 as today) and continues with the next group. It writes nothing to the old segment at roll.
2. **The sealer** hashes the keys, sorts the entries, builds the key filter, encodes the filter frame, the index frame, the footer and the locator with their CRCs, frees the old map, and returns the encoded bytes. It does no I/O. A panic in the sealer poisons the writer. In tests the sealer runs inline, so simulated traces stay the same from run to run.
3. **The writer writes the closing run.** At the first group commit after the bytes arrive, before that group's frame, the writer appends the key filter and key index frames to the old segment and `fsync`s it, then appends the footer and locator frames and `fsync`s it again. That segment is then sealed. The first flush matters because open reads only a sealed segment's locator and footer: without it, a power loss could keep those pages and drop a key index page, and the segment would look sealed. A failure here answers the group `Poisoned`, because none of its bytes were written. An I/O error there poisons the writer, as for any roll ([ADR 0004](0004-io-errors-poison-the-writer.md)). The bytes are the same run that ADR 0054 requires a closing run to be.
4. **Depth one.** At most one segment waits for its closing run. A roll that finds the previous one still pending first waits for the sealer and writes that run.
5. **Lookups.** Until the closing run is durable, the writer keeps the old key map readable, shared with the sealer, and a lookup checks it for the pending segment instead of that segment's filter and index. Rule 1 of ADR 0048 is otherwise unchanged.
6. **Builder.** The writer tells the log-filter builder about a segment only after its closing run is durable. ADR 0051's rule that the writer never waits for the builder holds.
7. **Seal and drop.** `seal`, and the writer's shutdown on drop, first wait for a pending closing run and write it, then do what they do today. A sealed log therefore never holds a segment without its closing run.
8. **Recovery.** The second-newest segment may lack its closing run, but only while the newest segment holds nothing but its header and group frames. Recovery scans that segment as it scans the newest one, truncates a torn closing run at its start as step 7 does, rebuilds the closing run from its records with the newest segment's header epoch, which is the rolling writer's, and writes it in the same two flushed steps before any other change to the files. A newer epoch would make the scan of the newest segment stop at its first group. Any other segment without a valid closing run stays `Corrupt`. `LogReader` writes nothing, so it returns `NotSealed` for such a log, as it does for a crashed roll today.

The format bytes do not change. A closing run is still the last thing in its segment, and the writer still never appends to a segment after its first closing frame.

Option D beat A and B because only moving the work off the writer bounds it, and the goal is a writer whose CPU cost at roll does not grow with the segment. It beat C because letting the writer keep every write costs only the ~5 ms of I/O the roll already pays, and keeps one error path and one writing thread. We accept a window in which the newest two segments are both unsealed, and a recovery path that rebuilds key frames, against ADR 0048's "no repair path".

## Consequences

- ADR 0048 is amended. Rule 6's "Nothing rebuilds key frames" gains the exception of decision 8. Its consequence "Appends wait for that roll" no longer holds: appends wait only for writing the closing run.
- ADR 0051 is amended. Rule 2's "After each roll, the writer tells the builder" becomes "after each rolled segment's closing run is durable".
- overview.md changes with the code: roll step 1 moves to the sealer and the next group boundary, recovery steps 5, 7 and 11 gain the second-newest case, and seal gains the wait. testing.md's M3 roll verifier, which requires no write to the old segment after the roll, changes to allow exactly one closing run.
- Open of a log that crashed with a closing run pending does O(keys) work to rebuild it, once.
- Memory during a roll holds the old key map until its closing run is durable, as it does today during the roll.

## Enforcement

The implementing change adds these, and testing.md names them:

- A scenario in `segment-log-core/src/acceptance/evidence.rs` (`measured_appends_survive_reopen`) that fails before the change: the writer's time in roll at 4× the keys per segment stays within its time at 1× plus a slack.
- Simulator crash cases: a crash with a closing run pending, after the run is written and before its `fsync`, and during the depth-one wait. Each checks that every acknowledged record and every retry survive, and that the rebuilt run equals the one the writer would have written.
- `KEY-LIFETIME` cases that retry keys of a segment whose closing run is pending.
- A `LogReader` case: `NotSealed` for a log with a pending closing run.
- The issue 49 capture on the next dedicated host, with the paced p99 within its bound.

## Revisit if

- The writer's remaining roll I/O shows in the paced tail on a slower device. Writing the closing run from the sealer (option C) is the next option.
- A roll size or rate makes the depth-one wait happen in practice.
- Retention deletes segments, which changes which segment can be second-newest.
