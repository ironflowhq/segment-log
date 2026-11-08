---
id: "0008"
title: "Idempotency key before expected revision"
status: proposed
date: 2026-11-07
deciders: []
supersedes: null
superseded_by: null
tags: [api, idempotency]
touches: [docs/architecture/api.md]
enforced_by: none
---

# 0008. Idempotency key before expected revision

## Context

Source: the order of checks and the key lifetime are from the reviewed design (decision 4 and "Records"). The rules for a whole submit and the vocabulary `Empty` / `At(n)` are new in this repository and not reviewed.

Recovery keeps an unacknowledged valid suffix ([ADR 0003](0003-recovery-keeps-the-checksum-valid-prefix.md)), so a caller with an unknown outcome needs to determine whether its records survived. For fully keyed submits, a retry can resolve survivors. It usually carries a stale expected revision, because the first attempt may have succeeded. If the revision check runs first, the retry fails, and the caller cannot tell "my write landed" from "someone else wrote".

The source design adopted no name for "this stream has no records" (no `NoStream`, no "empty", no "must exist", no `Any`). It also left open how a check sees an earlier record in the same submit. A first append needs some precondition, so this ADR has to choose one, and the PRD lists the choice as open item 2.

## Options & trade-offs

### Option A - key first, then expected revision

- **For:** a retry resolves to the original result even with a stale revision.
- **Against:** the log must keep a key map and read the original record to compare bytes.

### Option B - expected revision first

- **For:** no key map.
- **Against:** retries fail, so the caller cannot know if its side effect ran.

### Vocabulary A - `Empty` and `At(n)`, no `Any`

- **For:** every append states a precondition. Two names cover every case.
- **Against:** a caller that does not care about order must still read the head first.

### Vocabulary B - add `Any`

- **For:** easier for callers that do not need concurrency control.
- **Against:** the source design did not adopt it. It invites blind appends.

## Decision

We will check the idempotency key before the expected revision (option A), with `Empty` and `At(n)` as the only preconditions (vocabulary A). We accept a key map, and that a caller that does not care about order must still read the head first.

- For a submit, the writer checks keys first. If every record has a key and every key matches a durable record with the same stream and payload, the submit is a retry: the answer is the original offsets and revisions, and nothing is appended.
- A key that matches with different bytes is `KeyConflict`. A mix of matching and non-matching records is `PartialRetry`.
- Otherwise, the writer checks each expected revision. A mismatch rejects the whole submit, returns the actual revision, and uses no offset.
- A key lives as long as the segment that holds its record. When that segment is deleted, the key expires. A caller that needs a longer memory appends a record.
- Kafka's producer id and sequence are not used. They are not an application dedupe token ([prior art](../architecture/prior-art.md#kafka)).

The exact signatures are in [api.md](../architecture/api.md).

## Consequences

- [ADR 0031](0031-keys-live-with-retained-segments.md) confirms segment-lifetime keys across roll and reopen. Only the optimized lookup mechanism and key-cache memory budget remain open for M4 under PRD item 1.
- [ADR 0030](0030-expected-revisions-use-empty-and-at.md) confirms `Empty` and `At(n)` without `Any` for M1 and resolves PRD open item 2. Other proposed material in this ADR is not accepted by that confirmation alone.
- [ADR 0024](0024-fully-matching-retries-ignore-original-grouping.md) accepts fully matching retries regardless of original submit membership, ordering or grouping from M1 and resolves PRD open item 9.
- [ADR 0023](0023-reject-mixed-reused-and-new-records-from-m1.md) amends this decision by rejecting mixed reused-and-new submits against pending and durable records from M1. `KeyConflict` takes precedence over `PartialRetry`, and both precede expected revisions.
- [ADR 0022](0022-reject-conflicting-key-reuse-from-m1.md) amends this decision by rejecting conflicting key reuse against pending and durable records from M1, before expected-revision checks.
- [ADR 0021](0021-exact-pending-retries-wait-for-the-original.md) amends this decision by recognizing exact pending retries before expected-revision checks, starting in M1. A matched retry waits for the original group's durability result.
- [ADR 0020](0020-reject-repeated-keys-within-a-submit.md) amends this decision by rejecting repeated keys within a submit before admission, starting in M1.
- [ADR 0019](0019-idempotency-keys-are-scoped-to-the-whole-log.md) amends this decision by confirming whole-log key scope.
- The key map is a cache. Open rebuilds it from the segments.
- **Lookup gap (blocks M4):** footers do not hold keys. M4 must choose a retained-key lookup mechanism and key-cache memory budget under ADR 0031 before it stops scanning old records. It cannot resolve that gap by expiring keys on roll.

## Enforcement

None yet. Oracle check 4 in [testing.md](../architecture/testing.md#the-oracle) will check that a fully keyed survivor resolves to its original result. Absent records still follow expected-revision checks, and known rejections have no success guarantee. The M2 model test in [testing.md](../architecture/testing.md#property-and-unit-tests) will compare key and revision results with an in-memory model.

## Revisit if

- Callers need `Any`, or need "the stream must exist" without a revision.
- The key map uses too much memory for the footprint target.
