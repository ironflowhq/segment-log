---
id: "0054"
title: "A closing run must equal the run its groups produce"
status: accepted
date: 2026-11-13
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [format, recovery, integrity]
touches: [segment-log-core/src/engine/survey.rs, segment-log-core/src/engine/closing.rs, segment-log-core/src/engine/keys.rs, segment-log-core/src/engine/recovery.rs, docs/architecture/format.md, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: test
---

# 0054. A closing run must equal the run its groups produce

## Context

A closing run repeats facts that the segment's group frames already hold. The footer names each stream's last revision, offset and file position. The key filter and the key index name each keyed record by its key hash, file position and record index ([ADR 0048](0048-sealed-segments-carry-a-key-filter-and-index.md)). The writer builds the run from the groups it wrote (`engine/closing.rs`).

The format checks each closing frame on its own bytes and checks the shape of the run ([format.md](../architecture/format.md#rules-for-readers)). Nothing checks that the run agrees with the groups. The M5 fuzz target `FUZZ-RECOVERY` found the effect: a newest segment whose footer passes its CRC but names a stream that its groups do not have. Open takes the stream heads from the scanned groups. After the next roll, a reopen takes them from the footer, and reports other heads. The fuzz target skips that check in its checksummed mode for this reason.

These facts are fixed:

- Forging a run needs the log id, which no caller has ([ADR 0049](0049-frame-crcs-cover-the-log-id.md)). A run that passes every CRC but disagrees with its groups comes from a writer bug, or from damage that a 32-bit CRC missed.
- A disagreement in the key frames breaks idempotency. A key index that misses a keyed record makes a later retry of that key append a duplicate, with no error.
- Open scans every frame of the newest segment, so it holds the groups that the newest run must match. For every other sealed segment, open reads only the locator, the footer and the key filter, so that open time does not grow with the history (`ORACLE-13`).
- A run that lost a page to a power loss fails a CRC, and open truncates it (open step 7). That case is not a disagreement.
- The format is pre-production. A new reader rule needs no reader for older versions ([ADR 0053](0053-format-changes-need-no-older-reader-before-production.md)).

## Options & trade-offs

### Option A - rebuild the newest run from its groups and compare the bytes

- **For:** one rule covers every field of every closing frame, including ones added later. Recovery calls the writer's own `roll_run` and `seal_run`, so the check cannot drift from what the writer writes. The cost falls only on a newest segment that ends in a closing run: a crash between a roll's `fsync` and the next segment, or a sealed log's final segment.
- **Against:** rebuilding the key frames costs up to one key index frame (at most 16 MiB) in memory and time, once, in that rare case. Recovery must hash keys with the same function the writer used, so the hash function passes into the survey.

### Option B - compare field by field

- **For:** an error can name the field that differs.
- **Against:** a second description of the run, which can drift from the writer's. More code for the same coverage.

### Option C - truncate a run that disagrees

- **For:** open never stops.
- **Against:** a seal's run that disagrees would lose its seal, and the sealed log would become writable. The disagreement has no crash explanation, so repairing it hides a bug.

### Option D - check every sealed segment at open

- **For:** covers the whole history.
- **Against:** open would read every group of every segment, which `ORACLE-13` and the open-time target rule out.

### Option E - keep the format as it is

- **For:** no code.
- **Against:** a writer bug in a footer or a key frame goes unseen until a reopen reports other heads or a retry appends a duplicate.

## Decision

We will use option A. A closing run is valid only if its bytes equal the run that the writer builds from the segment's groups, with the run's own epoch and start position. Open applies this rule to the newest segment in step 6. A run that is complete, passes every CRC and has the right shape, but differs from the rebuilt run, fails open with `Corrupt` at the run's start, and open changes no file. A run that is not complete or fails a CRC keeps today's handling in step 7.

The rule holds for every sealed segment. Open checks only the newest, because older segments are read through their summaries (option D rejected). An operator check that scans a whole log applies the same rule to every segment; it belongs with the repair tool ([issue #1](https://github.com/ironflowhq/segment-log/issues/1)).

A covers every field with the writer's own code, and costs time only in the rare case where the newest segment ends in a run. We accept that recovery holds one rebuilt key index in memory in that case.

## Consequences

- format.md's rules for readers add the rule. Overview step 6 applies it to the newest segment.
- The survey receives the key hash function and returns it to the writer, so that tests that force key hashes rebuild the same key frames.
- `LogReader` applies the rule to a sealed log's final segment, which has no key frames.
- `FUZZ-RECOVERY` checks a second open in both modes.
- Hand-made closing runs in the recovery fixtures must match their groups, or expect `Corrupt`.

## Enforcement

`RECOVERY-FIXTURES` adds the cases, and testing.md names them:

- A rolled newest segment whose footer names a stream that its groups do not have fails open with `Corrupt` at the run's start, and changes no file.
- A rolled newest segment whose key index misses one keyed record fails open the same way.
- A sealed log whose final footer names a wrong last revision fails both `SegmentLog::open` and `LogReader::open` with `Corrupt`.
- A rolled newest segment whose run matches its groups opens, and gets its successor, as today.

The roll crash cases (`KEY-FRAMES-CRASH`, the M3 roll crashes) open every crash state of a roll, so a false `Corrupt` on a real run fails them.

## Revisit if

- Rebuilding the newest run shows up in open time, which needs a newest segment that ends in a run with a full key index.
- A field in a closing run stops being derivable from the groups.
