---
id: "0067"
title: "Appends do not wait for a roll's closing run: the sealer writes and flushes it"
status: accepted
date: 2026-11-19
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [core, writer, roll, latency]
touches: [segment-log-core/src/engine/**, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: test
---

# 0067. Appends do not wait for a roll's closing run: the sealer writes and flushes it

## Context

[ADR 0061](0061-a-sealer-thread-builds-the-key-frames-off-the-writer.md) moved the CPU work of a roll to a sealer thread. The writer still writes the old segment's closing run at the first group boundary after the sealer is done: it appends the key filter and key index frames, flushes, then appends the footer and locator and flushes again. ADR 0061 expected about 5 ms of I/O there and named the trigger to revisit: "The writer's remaining roll I/O shows in the paced tail on a slower device."

The [issue 154 record](../evidence/issue154/README.md) shows exactly that. On an EPYC 7502P with Samsung PM983 drives, 64 MiB segments, and every record keyed:

- Writing the closing run blocks the writer for 8–9 ms per roll (`epyc7502p/8b1a2d1/traced-*/stall.txt`). No group is written for 30–37 ms after that, then one group of about 450 submits.
- About 20% of the slowest 1% of paced requests start in that window.
- With the key map fix (#168), paced p99 is 4.1 ms against a bound of about 1.8 ms, 0 of 5 runs within it. The large groups left are at the rolls, except three in the first segment after open.

These facts are fixed:

- The closing run is about 1% of the segment, about 3.5 MiB at 64 MiB with every record keyed. Most of the writer's 8–9 ms is writing and flushing those bytes, not CPU work.
- The writer never appends to a segment after the next segment exists (overview.md, roll). Once the roll creates the next segment, the old file gets only its closing run.
- Recovery already handles a second-newest segment without its closing run, while the newest segment holds only its header and group frames (ADR 0061 rule 8). An ack never depends on a closing run.
- The two flushes must stay in order: open trusts a sealed segment's locator and footer, so the key frames must be durable before them (ADR 0061 rule 3).
- An I/O error on the write path poisons the writer, and no `fsync` is retried ([ADR 0004](0004-io-errors-poison-the-writer.md)). Only the writer thread can answer queued submits.
- The simulator's traces depend on call order. Background threads run inline in tests for that reason.

## Options & trade-offs

### Option A - Keep the closing run on the writer (ADR 0061 as it is)

- **For:** one thread writes files. No new error path or crash interleaving.
- **Against:** the stall is the I/O time of about 1% of a segment, so it grows with the roll size and the key count, and with a slower device. It misses the paced bound on the EPYC host today.

### Option B - Spread the closing run's writes over the next groups

The writer appends a slice of the run at each group boundary, and flushes the old file once the key frames are all there.

- **For:** one thread writes files. Each group waits for only a slice.
- **Against:** every group in the window pays an extra write, and two of them pay an extra flush of the old file, which is still on the writer. The window, and with it the time a lookup must use the pending map, grows with the run. More states for recovery and the crash cases.

### Option C - The writer appends, a flusher thread flushes

- **For:** the flushes leave the writer, and only the writer writes bytes.
- **Against:** the footer must follow the first flush, so the writer must wait for that flush before it appends the footer, or hand the footer to the flusher anyway. Either way this turns into option D with an extra hand-off.

### Option D - The sealer writes and flushes the closing run

After it builds the run, the sealer appends it to the old segment and does both flushes, in the order ADR 0061 rule 3 requires.

- **For:** the writer does no closing-run I/O. Each file still has one writing thread at a time: the writer owns the active segment, the sealer the rolled one.
- **Against:** two threads write the same device at once, so the sealer's flushes can slow group flushes. The sealer's I/O errors need a path back to the writer. Crash cases must cover writes to the two files interleaved.

## Decision

We will use option D.

1. **The sealer writes.** After it builds the closing run (ADR 0061 rule 2), the sealer appends the key filter and key index frames to the rolled segment through the storage trait, flushes it (`sync_data`, as the writer does today), then appends the footer and locator frames and flushes it again. It starts only after the roll has made the next segment durable. It uses the storage purposes the writer uses for these writes today.
2. **One writing thread per file.** From the roll on, only the sealer writes the rolled segment, and only the writer writes the active one. The writer never touches the rolled file again, as today.
3. **Hand-off.** The sealer reports `Durable` or `Failed` to the writer. The writer checks at each group boundary, without waiting. On `Durable` it does what ADR 0061 does once a closing run is durable: it drops the pending key map, so lookups use the segment's filter and index, and tells the log-filter builder about the segment.
4. **Failure.** On `Failed`, or if the sealer panics, the writer poisons itself at the next group boundary ([ADR 0004](0004-io-errors-poison-the-writer.md)). Nothing is retried. Groups acknowledged after the roll stay acknowledged: they are durable in the active segment, and recovery rebuilds a missing closing run in the second-newest segment (ADR 0061 rule 8).
5. **Unchanged rules.** Depth one (ADR 0061 rule 4): a roll that finds a closing run still pending waits until it is durable. `seal` and the drop of the log wait the same way (rule 7). Recovery is unchanged (rule 8).
6. **Tests.** In the simulator, the sealer still runs inline, and its writes happen at the first group boundary after the roll, where the writer makes them today. Simulated traces keep their order. The crash cases add the interleavings that production allows.

Option D beat A because only taking the I/O off the writer bounds the writer's time at a roll. It beat B and C because they keep part of the I/O, or a wait for it, on the writer, and add more states than D does. We accept a second thread writing to the device, and the error hand-off back to the writer that ADR 0061 avoided.

## Consequences

- Accepted by the owner on 2026-11-19 for the 0.1 release ([issue 133](https://github.com/ironflowx/segment-log/issues/133)).
- ADR 0061 is amended. Rule 3 ("the writer writes the closing run") is replaced by decision 1–4 here. Its sentence that the sealer does no I/O no longer holds. Its option C is the option chosen here.
- ADR 0051 is amended. Its Context fact "Only the writer thread writes segment files" becomes "only the writer writes the active segment; the sealer writes a rolled segment's closing run". The builder still writes no file.
- overview.md changes with the code: roll step 4 moves to the sealer, and the roll failure paragraph gains the sealer's `Failed`.
- The writer's time at a roll no longer depends on the device's write speed or the closing run's size.
- The sealer's flushes and the group flushes share the device. A group flush that lands during a closing run's flush can take longer. The issue 154 capture shows whether that costs more than the writer's stall did.
- Memory is unchanged: the pending key map lives until its closing run is durable, as today.

## Enforcement

The implementing change adds these, and testing.md names them:

- A failing scenario first (`ROLL-BOUNDED`, `segment-log-core/src/acceptance/sealed_keys/sealer_io.rs`): with the sealer's first flush held, groups into the new segment still commit. The writer before this change waits for that flush.
- Simulator crash cases (`KEY-FRAMES-CRASH`, same file): a crash in each mode at the sealer's first flush and between its two flushes, after groups were acknowledged in the new segment. Each checks that every acknowledged record and every retry survive, and that open completes the run its groups produce. The simulator holds one call at a time, so these cases cannot also hold a group flush in flight.
- A case where the sealer's flush fails (`ORACLE-7`, same file): the writer poisons at a later group boundary, the flush is not retried, and earlier acks survive reopen.
- The issue 154 capture on an EPYC 7401P or 7502P host, with paced p99 within its bound.

## Revisit if

- Group flush p99 rises during closing runs by more than the writer's stall it removes.
- Retention or compaction adds another thread that writes segment files.
- A roll size or rate makes the depth-one wait happen in practice.
