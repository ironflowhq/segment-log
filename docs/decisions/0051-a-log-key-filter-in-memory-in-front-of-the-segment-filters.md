---
id: "0051"
title: "A log key filter kept only in memory, in front of the segment key filters"
status: accepted
date: 2026-11-12
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [idempotency, performance, memory]
touches: [segment-log-core/src/engine/keys.rs, segment-log-core/src/engine/writer.rs, segment-log-core/src/engine.rs, segment-log-core/src/storage/mod.rs, docs/architecture/overview.md, docs/architecture/testing.md, docs/evidence/m4/**]
enforced_by: none
---

# 0051. A log key filter kept only in memory, in front of the segment key filters

## Context

[ADR 0048](0048-sealed-segments-carry-a-key-filter-and-index.md) gives each sealed segment a key filter and a key index. The writer checks every new key against the filter of every sealed segment. So the writer's cost per new key grows with the number of sealed segments, and the PRD sets no limit on history ([R22](../prd.md#requirements)).

The [M4 Linux report](../evidence/m4/completion.md) measured closed-loop keyed appends on the PRD fixture (100 sealed segments of 64 MiB, 22.7 million sealed keys) on a hosted runner. They reached 109,100 records/s, against 201,800 records/s for the M3 code (issue #70). The 50,000 records/s target is met, so ADR 0048's revisit trigger ("the writer cost per new key misses its budget") has not fired at 100 segments.

These facts are fixed:

- The writer classifies a group's submits, then writes and flushes the group, on one thread. Classification does not overlap the flush. So each microsecond of writer time per record comes off closed-loop throughput directly.
- Timers on the writer thread, on a development machine and not on the hosted runner, put about 1.5 to 2 µs per new key in the filter probes, one cache miss each, and about 0.85 µs in false-positive index reads (about 9% of new keys read a key index page, about 7 µs each).
- [Two captures](../evidence/issue70/README.md) compare keyed and unkeyed appends on the same keyed history. At 100 sealed segments, unkeyed appends on M4 run at 0.971 of M3, and the ranges overlap. So the gap between M3 and M4 is in the key lookup: keyed appends cost 4.65 µs more per record than unkeyed on M4, and 1.78 µs on M3.
- With 1,000 sealed segments and about the same number of keys, the lookup costs 30.86 µs per record on M4 and 1.56 µs on M3. Keyed appends on M4 reach 29,105 records/s, which misses the 50,000 records/s target, and a paced run at 50,000 records/s cannot keep up.
- M5 freezes the byte layout. After M5, a format change needs a new version and a reader for the older one ([ADR 0013](0013-provisional-v0-byte-layout.md)).
- A cache is never the only copy of a fact (I9). Sealed segments never change (I7). Only the writer thread writes segment files.

## Options & trade-offs

### Option A - keep ADR 0048 alone and state a history limit

- **For:** no code. The target is met at the PRD fixture.
- **Against:** the cost per new key still grows with history. At 1,000 sealed segments, keyed appends already miss the throughput target, and logs are never trimmed.

### Option B - a persisted log filter (ADR 0048, option D)

- **For:** about 0.1 to 0.2 µs per new key at 20 million keys ([prototype](../evidence/m4/key-lookup/README.md)). Ready at open.
- **Against:** a format change before M5. Kept inside each segment, it rewrites the whole log filter at every roll, about 16 MiB and growing with history. Kept in its own file, it is a cache that a crash can lose or leave stale, which ADR 0048 rejected for its option B. Open also reads it, and open time is already 38.6 ms of a 50 ms target.

### Option C - a log filter kept only in memory, built in the background after open

- **For:** about the same lookup cost as option B once the filter is built, and nearly flat with history. No format change, so M5 does not wait for it. Open reads nothing more. No file can go missing or stale.
- **Against:** a second thread that reads every sealed segment's key index after open, about 16 bytes per sealed key (363 MB at the PRD fixture). Until that build covers a segment, the writer probes that segment as it does today. Key-cache memory grows by 1 to 2 bytes per sealed key. The build reads at the same time as the writer writes, which a cold device feels.

### Option D - a denser filter in each segment

- **For:** fewer false-positive index reads. No new structure.
- **Against:** a format change before M5. The probes still grow with history; only the 0.85 µs part shrinks. Memory grows with the density.

### Option E - keep sealed segments' files open for key index reads

- **For:** no format change. It removes the open call from each false-positive read.
- **Against:** it helps only the 0.85 µs part, and the probes still grow with history. The storage trait has to manage handles, and at 1,000 segments that runs into file-descriptor limits.

### Option F - classify the next group while the current group flushes

- **For:** writer time per record would hide behind the flush, for every cost, not only keys.
- **Against:** a different write path. Classification reads the tail map and the key map that the current group changes, so the overlap needs its own design and its own crash cases. That is far more than this problem needs.

## Decision

We will build option C, and keep it only if the measurement below shows that it works.

The writer keeps one log key filter in memory. It is a blocked Bloom filter over the key hashes of the sealed segments that it covers. The segment filters stay and keep their role. A lookup checks, in order:

1. the keys staged by the current group and the active segment's exact map, as now;
2. the segment filter of each sealed segment that the log filter does not cover yet, as now;
3. the log filter, once. A "no" ends the lookup. A "maybe" goes on to the segment filters of the covered segments, newest first, as ADR 0048 rule 1 says.

The rules that follow from it:

1. **Coverage.** The log filter covers a run of sealed segments from the oldest. A count of covered segments says where the run ends. A Bloom filter has no false negatives, so a "no" for a covered segment is always right. A segment outside the run is probed through its own filter.
2. **Build.** After open returns, a builder thread reads the key index frames of the sealed segments, oldest first, through the storage trait, and adds each hash. It moves the covered count forward after each segment. After each roll, the writer tells the builder, which adds the new segment from the page cache. The writer never waits for the builder.
3. **Sharing without `unsafe`.** The filter words are `AtomicU64`. The builder sets bits with `fetch_or`, and the writer reads them with relaxed loads. The builder stores the covered count with release ordering, after it has set every bit of that segment. The writer loads it with acquire ordering.
4. **Size and growth.** The filter holds at least 8 bits for each covered key. A build sizes it for twice the sealed keys at that time. When the sealed keys pass that size, the builder builds a larger filter from the start and hands it to the writer, which swaps it in at a group boundary. Until then, the old filter stays correct; it only gives more false hits.
5. **Failures.** An I/O error or a bad frame in the builder stops the builder. It does not poison the writer, because the builder is not the write path and the log filter is a cache. The writer goes on probing the segments the filter does not cover, so every answer stays correct. A bad frame that the writer reads itself still poisons it ([ADR 0048](0048-sealed-segments-carry-a-key-filter-and-index.md) rule 6).
6. **Lifetime.** The builder starts after open returns and stops at `seal` and when the log is dropped. `LogReader`, a sealed log and a poisoned writer start no builder.
7. **Open.** Open reads exactly what it reads now. The builder's reads have their own storage purpose, so that `ORACLE-13` still counts only open's reads.

Option C keeps the lookup that ADR 0048 chose and adds a cache in front of it, so a cache failure costs speed, never a wrong answer. It needs no byte on disk, so M5 can freeze the format without it. B is a format change and a cache file that ADR 0048 already rejected. A, D and E leave the cost growing with history. F fixes more than this problem, at the cost of a new write path. We accept a background thread that reads the whole key index once after each open, and 1 to 2 more bytes of memory per sealed key.

### Measurement that decides it

Before this ADR is accepted, the `m4-evidence` workflow measures on the same commit:

1. Keyed and unkeyed closed-loop appends on the keyed PRD fixture, on the M3 and the M4 code. If M4 is also slow without keys, the lookup is not the main cost, and this decision is wrong. **Done** at `15dd21d`: the lookup is the cost (see Context).
2. Keyed against unkeyed closed-loop appends with option C, at 100 sealed segments and at 1,000. The second fixture uses a smaller `roll_bytes` and keeps about the same number of keys, so that it fits on the runner's disk.

Option C passes if keyed closed-loop throughput is within 10% of unkeyed at both sizes, with open time, `ORACLE-13` and the key-cache memory formula still met. The owner replaced this criterion on 2026-11-12, after the result below.

**Result of step 2** ([evidence](../evidence/issue70/README.md#with-the-log-key-filter-adr-0051), pull request 75):

- The criterion above fails: keyed/unkeyed is 0.64 to 0.73. It also fails for M3, whose exact map gives 0.66 to 0.68 in the same captures. So keyed against unkeyed measures more than the lookup: the key bytes in each record, the active segment's key map and, on M4, the key index that each roll writes. The criterion was the wrong measure.
- The lookup cost (keyed minus unkeyed time per record) no longer grows with history. With the builder finished, it is 2.52 µs at 100 sealed segments and 1.18 µs at 1,000, against 2.13 µs and 1.31 µs for M3's exact map. Keyed closed-loop throughput is 0.896 and 0.933 of M3 at the two sizes, with overlapping ranges. Before the filter it was 0.605 and 0.140.
- **Replacement criterion, accepted by the owner on 2026-11-12:** the lookup cost does not grow from 100 to 1,000 sealed segments and stays within 1 µs of M3's, and keyed closed-loop throughput is within 10% of M3, once the build has finished. These captures meet it, and the owner accepted this ADR on that result.
- Warm open time is unchanged: 38.6 ms at 100 sealed segments, as at the M4 gate. `ORACLE-13` and `KEY-MEMORY` pass.

## Consequences

- Up to 1,000 sealed segments, the writer cost per new key no longer depends on history. That becomes a stated operating limit.
- ADR 0048 rule 5 changes. Key-cache memory adds the log filter: between 1 and 2 bytes for each sealed key.
- Open time does not change. After open, the builder reads about 16 bytes per sealed key. Until it finishes, lookups run at today's speed, and the builder competes with appends for processors: on a 4-processor runner, appends right after open ran at 0.82 to 0.85 of M3 against 0.90 to 0.93 after a 15 s wait. How long the build takes is not measured yet.
- A second thread runs beside the writer. It never writes a file. Tests can run the build synchronously, so that simulated traces stay deterministic.
- The issue no longer blocks M5. Issue #70 tracks the work.

## Enforcement

None yet. The work must add these, and testing.md names their cases:

- A differential scenario that classifies the same submits with and without the log filter, with forced hash collisions, and requires the same answers.
- A scenario that stops the builder with an I/O error and a bad key index frame, and checks that every answer stays correct and the writer is not poisoned.
- `KEY-MEMORY` with the new formula.
- `ORACLE-13` unchanged.
- The `m4-evidence` workflow reports keyed against unkeyed appends on the same commit.

## Revisit if

- The measurement shows that M4 without keys is about as slow as M4 with keys. Then look for the cost elsewhere.
- Keyed appends stay more than 10% behind unkeyed at 1,000 sealed segments.
- A deployment needs key memory with a hard cap, or open must be fast with a cold device on many segments. Then a persisted filter (option B) is the next option to measure.
- Classification and the flush come to overlap for another reason (option F). Then measure whether the log filter still pays for itself.
