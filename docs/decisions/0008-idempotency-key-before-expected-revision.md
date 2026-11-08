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

Recovery keeps an unacknowledged valid suffix ([ADR 0003](0003-recovery-keeps-the-checksum-valid-prefix.md)), so a caller that got no ack must retry. A retry usually carries a stale expected revision, because the first attempt may have succeeded. If the revision check runs first, the retry fails, and the caller cannot tell "my write landed" from "someone else wrote".

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

- The decision compares the stream id on a key match, which assumes that keys are scoped to the whole log. That scope is still PRD open item 9, and this ADR changes when the item is settled.
- The key map is a cache. Open rebuilds it from the segments.
- **Open gap (blocks M4):** after footers exist, open does not scan sealed segments, and footers do not hold keys. Then nothing finds a key in a sealed segment. See PRD open item 1.
- A retry of a record that is still in a group that is not yet durable does not match a key. M2 must decide whether it waits for that group.

## Enforcement

None yet. Oracle check 4 in [testing.md](../architecture/testing.md#the-oracle) will check that a retry with the same keys resolves to the original result. The model test in [testing.md](../architecture/testing.md#property-and-unit-tests) will compare key and revision results with an in-memory model.

## Revisit if

- Callers need `Any`, or need "the stream must exist" without a revision.
- The key map uses too much memory for the footprint target.
