---
id: "0003"
title: "Recovery keeps the checksum-valid prefix"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [recovery, durability]
touches: [docs/architecture/overview.md]
enforced_by: none
---

# 0003. Recovery keeps the checksum-valid prefix

## Context

Source: from the reviewed design (decision 3 and "Commit protocol"). The review did not agree on this point. This ADR records the recommendation that the design chose.

After a crash, the active segment can end with frames that were written, maybe flushed, but never acknowledged. The first draft wanted recovery to ignore every CRC-valid frame after the last group whose `fsync` returned. Then "no ack" would mean "absent". The review found that write, then `fsync`, then move the watermark cannot give that rule. One barrier cannot record its own completion.

## Options & trade-offs

### Option A - keep the contiguous CRC-valid prefix (the indeterminate suffix)

- **For:** one `fsync` per group. It matches what a disk gives. The idempotency keys of surviving frames are kept, so a retry finds the survivor and does not run twice.
- **Against:** "no ack" means "might be durable". Callers must retry with the same key, and not assume absence.

### Option B - ignore a valid suffix after the last acknowledged group, with a second barrier that records the commit

- **For:** recovery matches what callers were told, for groups whose ack record survived.
- **Against:** two `fsync` calls per group, which doubles the cost this design exists to spread. It still cannot make "no ack" mean "absent", because the process can die after the barrier and before it sends the ack. Dropping a valid suffix deletes the only copy of its keys, so a retry looks new and its side effect can run again.

## Decision

We will keep the contiguous CRC-valid prefix at recovery (option A). We accept that no ack means "might be durable", so callers must retry with the same key. On open, the log removes a torn tail, keeps the contiguous CRC-valid prefix of the active segment, and sets the watermark after it. It truncates the file and calls `fsync` before it accepts an append. The exact scan rules are in [overview.md](../architecture/overview.md#open-and-recovery) and [format.md](../architecture/format.md#rules-for-readers).

The contract:

- An ack means the group survived, if the device honors the flush.
- No ack means the group might have survived.
- Recovery cannot tell if the previous process saw `fsync` return.
- A retry with the same key and bytes returns the original log offset and revision.

## Consequences

- [ADR 0015](0015-open-makes-the-recovered-tail-durable.md) extends this decision. It makes the kept tail durable at every open.
- Idempotency keys are required on any append that a caller might retry ([ADR 0008](0008-idempotency-key-before-expected-revision.md)).
- Offsets from a torn tail can be used again. Durable offsets cannot.
- The crash tests must model power loss that drops and tears unsynced bytes, not only `kill -9` ([ADR 0014](0014-crash-testing-strategy.md)).

## Enforcement

None yet. Oracle checks 1–5 in [testing.md](../architecture/testing.md#the-oracle) will check the prefix rule and the retry contract after every crash mode.

## Revisit if

- A consumer needs "no ack means absent". That needs a protocol with the caller, such as a prepare and a commit, and a new ADR.
