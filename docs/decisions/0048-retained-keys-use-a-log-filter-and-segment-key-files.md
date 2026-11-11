---
id: "0048"
title: "Retained keys use a log filter and per-segment key files, not an in-memory map"
status: proposed
date: 2026-11-11
deciders: []
supersedes: null
superseded_by: null
tags: [idempotency, recovery, memory]
touches: [segment-log-core/src/engine/keys.rs, segment-log-core/src/engine/recovery.rs, docs/architecture/format.md, docs/architecture/api.md]
enforced_by: test
---

# 0048. Retained keys use a log filter and per-segment key files, not an in-memory map

## Context

[PRD open item 1](../prd.md#open-items) blocks M4. M4 opens a log by reading only the header, the locator and the footer of each sealed segment. Footers hold no keys. A key must stay valid while its segment is retained, including after roll and reopen ([ADR 0031](0031-keys-live-with-retained-segments.md), R13). M3 meets that by scanning every record of every segment at open. M4 must replace that scan without shortening the key lifetime.

These facts are fixed:

- The open-time target is 50 ms with a 4 MiB active segment and 100 sealed segments. The append target is 50,000 records/s, which leaves the writer about 20 µs for each record.
- The writer checks every key of every new submit against the retained keys. Almost every key is new, so the "not found" answer is the hot path. A retry is rare.
- A found key is already confirmed by reading its frame and comparing stream and payload. A false "found" therefore costs time, never correctness.
- Lookup data is a cache. The segment records hold every fact and must be able to rebuild it (I9). A lookup I/O error poisons the writer ([ADR 0004](0004-io-errors-poison-the-writer.md)).
- The idle resident set of an empty log must stay small (PRD goal 5, the 8 MiB server target).

At the default 64 MiB roll size, 100 sealed segments hold about 20 million keys when every record is keyed. A [prototype](../evidence/m4/key-lookup/README.md) measured each option at that scale (S1) and at 2 million keys (S2). The numbers below are S1 medians on macOS with a warm page cache. They are diagnostics, not target results.

Every option except A keeps the same structure on disk. Each sealed segment has a key file next to it. The file holds a filter and an index of the segment's key hashes and record offsets, sorted by hash, in checksummed pages. A filter is a compact bit array that can answer "this key is certainly not here" ([Bloom filter](https://en.wikipedia.org/wiki/Bloom_filter)). The options differ in what stays in memory and in which order the writer asks.

## Options & trade-offs

### Option A - an exact map in memory, loaded from key files

- **For:** a lookup is one hash-table probe (6 ns). No false "found".
- **Against:** 28.5 bytes per key, or 570 MB at S1, and 430 ms to load. Today's map of full keys is worse: 88 bytes per key and 22 s to build. Memory grows with history without a bound.

### Option B - one filter per sealed segment, all in memory

- **For:** 1.3 bytes per key (25 MB at S1). Open loads the filters in 16 ms.
- **Against:** a new key probes every segment's filter. That costs 1.3 µs at 100 segments, and false hits add index reads, so a new key costs 4 µs in all. The cost grows with the segment count, on the writer thread.

### Option C - one log filter with a fixed memory budget

- **For:** a new key costs one probe. Memory is the budget, whatever the history. A 64 MiB filter gives 25 ns per new key at S1.
- **Against:** a false hit has no segment to go to. It searches the index of every segment, which took 320 µs at 100 segments. At 16 MiB the filter answers "maybe" for 4% of new keys at S1, so a new key costs 11 µs on average. At 64 MiB, open takes 40 ms and each roll rewrites 64 MiB.

### Option D - the log filter in front of the segment filters

- **For:** the log filter answers "certainly not here" for most new keys in one probe. Only its "maybe" answers reach the segment filters, and those rarely send the writer to an index read. With a 16 MiB log filter, a new key costs 0.2 µs at S1 and 7 ns at S2. Open loads both kinds of filter in about 28 ms (11 ms plus 16 ms, measured separately). A retry costs 2 to 15 µs before the frame read.
- **Against:** two filters to build, persist and rebuild instead of one. Memory is the log filter plus 1.3 bytes per sealed key, so it still grows with history, about 43 MB at S1.

### Option E - expire keys when their segment is sealed

- **For:** the active segment's map is enough.
- **Against:** a retry of a retained record appends a duplicate. [ADR 0031](0031-keys-live-with-retained-segments.md) rejected this.

## Decision

We will use option D. Each sealed segment gets a key file with a filter at 10 bits per key and a sorted, checksummed index. The log keeps one log filter of at most `key_filter_bytes`, with a proposed default of 16 MiB. A lookup asks the log filter first, then the segment filters from the newest segment back, then reads the index of a segment whose filter answered "maybe", then reads and compares the frame as today. The active segment keeps its exact in-memory map, as it does now.

Option D is the only option that keeps the writer's cost per new key well under its 20 µs budget at S1 and stays inside the open-time target with room for the footers and the active-segment scan. A and C fail the memory or open-time limits at S1. B fails the writer cost. We accept two structures instead of one, and memory that still grows with history at about 1.3 bytes per sealed key.

The rules that follow from it:

1. **Memory budget.** Key-cache memory is the log filter, at most `key_filter_bytes`, plus 10 bits per key of each sealed segment, plus the active segment's exact map, which the roll size bounds. The log filter starts small and doubles up to `key_filter_bytes` as sealed keys grow, so an empty or small log stays small. A doubling rebuilds the log filter from the key files at a roll. The prototype measured 190 ms for this rebuild at S1, so it happens at most a few times in a log's life.
2. **Writes.** At each roll, after the footer is durable, the writer writes the old segment's key file and adds its keys to the log filter file. It does not `fsync` either file, because both are caches. Each file records the segments it covers and a checksum. The prototype measured about 8 ms of roll work for a segment of 200,000 keys, plus about 9 ms to write a 16 MiB log filter.
3. **Open.** Open reads each key file's filter and the log filter file, not the segment records. A key file that is missing, short or fails its checksum is rebuilt by scanning its segment. A log filter that is missing, fails its checksum or covers too few segments is rebuilt from the key files. A rebuild is a repair, not the normal open. A missing key file costs about 40 to 55 ms per 64 MiB segment, and that open can miss the target. Open counts these reads separately from segment-file reads, so the ORACLE-13 limit on segment files still holds.
4. **Failures during a lookup.** An I/O error while the writer reads a key file or a frame poisons the writer, as a lookup failure does today. A key-file page that fails its checksum is not a fact of the log. The writer rebuilds that key file from its segment and repeats the lookup. An I/O error during that rebuild poisons the writer.
5. **Hash.** Filters and indexes persist a 64-bit hash of the key. The hash function is specified in format.md and keyed by the log id, so it does not change with the Rust version and callers cannot choose keys that collide in every log.

## Consequences

- PRD open item 1 has a proposed answer. M4 can start once this ADR is accepted.
- format.md gains two file layouts: the key file and the log filter file. Both stay provisional until M5. The M3 recovery already ignores files that are not segment files.
- api.md gains `key_filter_bytes` with its default and limits.
- Each roll does about 20 ms more work on the writer thread at S1 scale. Appends wait during that time.
- A retry of a key in an old segment can read the device when the page cache is cold. The prototype measured 76 µs for one uncached 4 KiB read. Retries are rare, so this is accepted.
- Memory still grows with history at about 1.3 bytes per sealed key. At S1 that is about 43 MB in all. A deployment with a hard memory cap must size its history or its keyed share to fit.
- The numbers come from macOS with a warm cache. M4's completion gate must measure open time, lookup reads, rebuild cost and key-cache memory on Linux, as testing.md requires.

## Enforcement

The M4 tests carry this decision.

- `KEY-LIFETIME` at M4 checks that retries of keys in sealed segments return the original positions after reopen, with key files present, missing, short and corrupted.
- `ORACLE-12` deletes the key files and the log filter file and checks the same answers after reopen.
- `ORACLE-13` counts open's reads and checks that segment files give only headers, locators and footers.
- A memory test checks that key-cache memory stays within the budget formula above.
- Crash cases cut a roll before, during and after the key-file and log-filter writes, and check that every retry still finds its key.

## Revisit if

- The Linux measurement at the M4 gate misses the open-time target or the writer cost under this design.
- A deployment needs a hard cap on total key memory. Keeping segment filters on disk behind the log filter is the next option to measure.
- Logs grow far past 100 sealed segments, so the per-key memory of the segment filters dominates.
- A future retention feature deletes segments. Then the log filter must be rebuilt without the deleted keys.
