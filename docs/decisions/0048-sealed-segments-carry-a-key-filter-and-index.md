---
id: "0048"
title: "Each sealed segment carries a key filter and a key index"
status: accepted
date: 2026-11-10
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [idempotency, recovery, memory, format]
touches: [segment-log-core/src/engine/keys.rs, segment-log-core/src/engine/recovery.rs, docs/architecture/format.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md, docs/prd.md]
enforced_by: none
---

# 0048. Each sealed segment carries a key filter and a key index

## Context

[PRD open item 1](../prd.md#open-items) blocks M4. M4 opens a log from each sealed segment's header, locator and footer, and footers hold no keys. A key must stay valid while its segment is retained, including after roll and reopen ([ADR 0031](0031-keys-live-with-retained-segments.md), R13). M3 meets that by scanning every record at open. M4 must replace that scan without shortening the key lifetime.

These facts are fixed:

- Open must take at most 50 ms with a 4 MiB active segment and 100 sealed segments. Appends must reach 50,000 records/s, which leaves the writer about 20 µs for each record.
- The writer checks every key of every new submit against the retained keys. Almost every key is new, so "not found" is the hot path. A retry is rare.
- The key bytes decide a match (api.md, "Lookup uses the key bytes alone"). A lookup that reads a frame compares the key first, then the stream and the payload.
- A lookup I/O error or a bad CRC during a writer lookup poisons the writer ([ADR 0004](0004-io-errors-poison-the-writer.md)). A bad frame in a sealed segment is corruption (R15).
- Sealed segments never change (I7). Lookup data must be derived from segment records (I9).

At the default 64 MiB roll size, 100 sealed segments hold about 20 million keys when every record is keyed. A [prototype](../evidence/m4/key-lookup/README.md) measured each option at that scale (S1) and at 2 million keys (S2). The numbers are medians on macOS with a warm page cache and exclude hashing the key, about 15 ns with SipHash-2-4. They are diagnostics, not target results.

In every option except A, each sealed segment has a filter and an index. A filter is a compact bit array that can answer "this key is certainly not here" ([Bloom filter](https://en.wikipedia.org/wiki/Bloom_filter)). The index lists the segment's key hashes, sorted, each with what is needed to read its record. The options differ in where these live and what stays in memory.

## Options & trade-offs

### Option A - an exact map in memory

- **For:** one hash-table probe, 6 ns, for a new key.
- **Against:** a map of 64-bit hashes takes 28.5 bytes per key, 570 MB at S1, and 430 ms to load. Today's map of full keys takes 88 bytes per key and 22 s to build.

### Option B - a filter per segment, in a separate key file next to it

- **For:** with 16 bits per key, a new key costs 1.7 to 1.9 µs at S1. Memory is 40 MB, and loading the filters takes 25 ms. With 10 bits per key it is 4.3 µs, 25 MB and 17 ms.
- **Against:** a key file is a cache outside the segment. If the writer does not `fsync` it, a guest kill loses the key files of the last rolls, and the next open rebuilds each one from its segment at 44 to 59 ms. If the writer does `fsync` it, each roll waits for another flush. A key file must also prove which segment and which log it describes, or a leftover file from a deleted log can hide keys and let duplicates through.

### Option C - one log filter with a fixed size

- **For:** a new key costs one probe. Memory is a true bound. A 64 MiB filter costs 25 ns per new key at S1.
- **Against:** a false hit names no segment, so it searches every index. That took 320 µs at 100 segments. A 16 MiB filter gives a false hit for 4% of new keys at S1, or 11 µs on average. A 64 MiB filter takes 40 ms to load and is rewritten at every roll. A 32 MiB filter was not measured. The false-hit rate grows with history, so the cost per new key grows without a bound.

### Option D - a log filter in front of per-segment filters

- **For:** the fastest new-key lookup. 0.1 to 0.2 µs at S1 with a 16 MiB log filter.
- **Against:** a second structure to build, persist, grow and repair. It rewrites 16 MiB at every roll, about 29% more device writes (inferred). Growing the log filter means a full rebuild of 100 to 190 ms on the writer thread. Its gain over option E is about 1.6 to 1.8 µs per new key.

### Option E - a filter and an index inside each sealed segment

- **For:** the same lookup as option B, but the key data is part of the segment. The roll's existing footer `fsync` makes it durable. No separate file can go missing, go stale or belong to another log. The segment's header, frame CRCs and epochs already protect it.
- **Against:** a sealed segment gains about 18 bytes per keyed record, about 5% at S1. Open reads each sealed segment's filter frame too, so R21 and ORACLE-13 must allow that read. A bad key frame makes its segment `Corrupt`, as any bad frame in a sealed segment does. The key frames must fit the frame size limit.

### Option F - expire keys when their segment is sealed

- **For:** the active segment's map is enough.
- **Against:** a retry of a retained record appends a duplicate. [ADR 0031](0031-keys-live-with-retained-segments.md) rejected this.

## Decision

We will use option E. When the writer rolls a segment, it writes two closing frames before the footer: a key filter frame with 16 bits per key, and a key index frame. The roll's footer `fsync` makes them durable with the footer. Open loads each sealed segment's key filter into memory. The active segment keeps its exact in-memory map, as it does now.

Option E keeps the writer's cost per new key at about a tenth of its 20 µs budget at S1, and it adds no file, no rewrite and no repair path. A fails on memory and open time. C trades a true memory bound for a cost per new key that grows with history. D is faster by about 1.6 to 1.8 µs per new key, which the writer does not need, at the cost of a second structure that every roll rewrites. B has E's lookup but puts the key data in a cache file that a crash can lose or leave stale. We accept larger sealed segments, an extra read at open, and memory that grows with history.

The rules that follow from it:

1. **Lookup order.** The writer checks the keys staged by the current group and the active segment's map first. Then it asks each sealed segment's filter, from the newest segment back. For each "maybe", it searches that segment's index for every entry with the key's hash, then reads and decodes each named frame.
2. **Hash collisions.** The index holds a 64-bit hash, so two keys can share one. The writer compares the key bytes in the frame first. A different key there is a false hit. The writer then tries the next entry with the same hash, then older segments. A false hit is never a match, never a `KeyConflict` and never a lookup failure. The hash is SipHash-2-4 keyed by the log id, and format.md specifies it, so it does not change with the Rust version.
3. **Index entries.** Each entry holds the key hash, the file position of the frame that holds the record, and the record's index in that frame. The frame gives the log offset and the revision. format.md defines the widths.
4. **Frame size.** The writer rolls before a group whose keys would make a key frame pass the frame body limit, as it does for the footer ([ADR 0017](0017-footer-locator-offset-and-size.md)). The footer records where the key frames start. The locator is unchanged.
5. **Memory.** Key-cache memory is 2 bytes for each key in a sealed segment (the filters), plus a few bytes per index page for the search, plus the active segment's exact map, plus the key frames built during a roll. There is no fixed upper limit. It grows with retained history, and the M4 memory test checks this formula. At S1 the filters take 40 MB and the active map about 18 MB.
6. **Failures.** A bad key filter frame at open is `Corrupt` for that segment, as R15 requires for any sealed frame. An I/O error or a bad CRC while the writer reads a key index frame or a record frame poisons the writer, as today. Nothing rebuilds key frames, because they live in segments that never change.
7. **Read-only paths.** `seal` writes no key frames, because a sealed log never appends again. `LogReader` and the read-only open of a sealed log never read key frames.

## Consequences

- PRD open item 1 is resolved. M4 can start.
- R21 and ORACLE-13 change. From each sealed segment except the newest, open reads the header, the locator, the footer and the key filter frame.
- format.md gains the key filter frame, the key index frame, the footer field for their position, and the hash. They stay provisional until M5. The sentence "The footer does not hold idempotency keys" stays true. The keys stay out of the footer.
- api.md gains no option. The filter density is a format constant.
- Each roll does about 8 ms more work on the writer thread at S1, and its `fsync` writes about 3.6 MB more. Appends wait for that roll.
- Open reads about 2 bytes per sealed key. At S1 that is 40 MB and took 25 ms. With the footers and the active-segment scan, open at S1 is likely close to 50 ms warm and above it cold (inferred). This conflicts with PRD goal 5, which says open time is proportional to the active segment and not to history. The owner accepted this on 2026-11-10, together with memory that has no fixed upper limit.
- A retry of a key in an old segment reads and checks a whole group frame, up to the group byte cap. With a cold cache that read goes to the device. Retries are rare, so this is accepted.
- The M4 gate must measure on Linux: open time end to end, cold and warm, key-cache memory, and the effect on append throughput and p99 latency.

## Enforcement

None yet. The M4 tests must add these, and testing.md names their cases.

- `KEY-LIFETIME` at M4 checks that retries of keys in sealed segments return the original positions after reopen.
- A simulator case forces hash collisions and checks that a new key with a colliding hash is appended, and that a true retry still matches.
- `ORACLE-12` checks that a reopen gives the same answers.
- `ORACLE-13` counts open's reads and checks the four parts that each sealed segment gives.
- A memory test checks key-cache memory against rule 5.
- Crash cases cut a roll before, during and after the key frames and check that every retry still finds its key.

## Revisit if

- The Linux measurement at the M4 gate misses the open-time target. Loading the filters after open returns, or a smaller filter such as a binary fuse filter, are the next options to measure.
- The writer cost per new key misses its budget, for example far past 100 sealed segments. Option D is the next option to measure.
- A deployment needs a hard cap on key memory. Option C, or keeping old segments' filters on disk, are the options to measure.
- A future retention feature deletes segments. Their keys then leave with them.
