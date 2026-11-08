---
id: "0009"
title: "The caller's submit is the atomic unit"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [write-path, api]
touches: []
enforced_by: none
---

# 0009. The caller's submit is the atomic unit

## Context

Source: the atomic unit is from the reviewed design (decision 8). How a check sees earlier records in the same submit or in earlier pending submits is new in this repository and not reviewed. The design left it open.

A group can hold submits from many callers. If a caller thinks that a group is a transaction, it can depend on two unrelated appends committing together. A caller that writes several events for one command needs all of them or none.

## Options & trade-offs

### Option A - one submit is atomic; a group is not

- **For:** a clear unit for the caller. The writer can still pack many submits into one `fsync`.
- **Against:** atomic writes across calls need one bigger submit.

### Option B - transactions across submits (Kafka transaction markers, prepare and commit)

- **For:** more flexible.
- **Against:** control records, a last stable offset, and recovery for half-done transactions. None of that is needed.

## Decision

We will make one submit the atomic unit, and not a group (option A). We accept that atomic writes across calls need one bigger submit. A submit holds one or more records, possibly in different streams. The writer puts a whole submit in one frame. It never splits a submit across a linger boundary, the byte cap, or a roll. A submit that cannot fit in one group gets `TooLarge`.

Checks run in order of admission. A record's expected revision sees the records before it in the same submit and in earlier submits that are pending in the same group, as if each record were appended one after another. If a submit fails, its pending changes are removed, and later submits do not see them.

## Consequences

- [ADR 0024](0024-fully-matching-retries-ignore-original-grouping.md) clarifies that fully deduplicated retries confirm existing durable records without asserting that they originally committed atomically together. New writes retain the all-or-nothing guarantee.
- The largest submit is limited by the byte cap.
- The crash tests check that each submit is present in full or absent in full.

## Enforcement

None yet. Oracle check 3 in [testing.md](../architecture/testing.md#the-oracle) will check that each submit is present in full or absent in full.

## Revisit if

- A consumer needs atomic writes larger than the byte cap.
