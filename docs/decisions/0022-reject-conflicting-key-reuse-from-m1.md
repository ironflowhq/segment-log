---
id: "0022"
title: "Reject conflicting key reuse from M1"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, idempotency, errors]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: none
---

# 0022. Reject conflicting key reuse from M1

## Context

Keys are scoped to the whole log. M1 recognizes full durable retries and exact pending retries, but the milestone previously deferred `KeyConflict` to M2. It needs a defined response when an existing key carries a different stream or payload.

The owner approved rejecting conflicting reuse against both pending and durable records during the M1 readiness discussion on 8 November 2026. This ADR amends the key rules in ADR 0008 and moves `KeyConflict` into M1.

## Options & trade-offs

### Option A - reject conflicts against both pending and durable records from M1

- **For:** a key keeps one meaning regardless of whether its original record has been flushed. Caller mistakes cannot append a second record under that key.
- **Against:** M1 must compare pending records as well as durable ones and report conflicts before checking revisions.

### Option B - reject conflicts only against durable records

- **For:** conflict checks use only the durable key map.
- **Against:** incompatible uses of one key can enter the same group. The result depends on when the group flushes.

### Option C - leave conflicts outside M1's supported inputs

- **For:** defers implementing `KeyConflict` until M2.
- **Against:** M1 needs another restriction and a defined way to enforce it. Existing key matching already needs the stream and payload comparison.

## Decision

From M1, if a submitted key identifies a pending or durable record with a different stream or payload, the writer rejects the entire new submit with `CommitError::KeyConflict { index }`. The index names the first conflicting record in submit order.

The writer checks keys before expected revisions. A different expected revision alone is not a key conflict. The same-bytes rule in [api.md](../architecture/api.md#behavior-notes) still compares the stream and payload, not a hash.

The rejected submit appends nothing and consumes no offset or revision. It changes neither the original submit nor its retry waiters. The conflict itself does not poison the writer or require a flush of the pending original. Other valid submits can proceed.

Admission remains unchanged. A repeated key within the new submit gets `AppendError::Invalid` before queuing under ADR 0020. Cross-submit conflicts are reported by `Pending::wait` after the writer checks its key maps.

We accept implementing the conflict response in M1 so one key identifies one record from the start.

## Consequences

- [ADR 0023](0023-reject-mixed-reused-and-new-records-from-m1.md) moves mixed reused-and-new rejection into M1 and sets `KeyConflict` before `PartialRetry`.
- `KeyConflict` moves from M2 into M1, including conflicts against earlier valid submits in the current group.
- A valid pending original reserves its keys before later submits are checked. A rejected submit creates no reservation and changes no stream head.
- A conflict response is a rejection, not an acknowledgment of the pending original's durability.
- Recovery rebuilds the same key identity from retained records. No new frame or metadata is needed.
- [ADR 0024](0024-fully-matching-retries-ignore-original-grouping.md) resolves open item 9 by accepting fully matching regrouped retries from M1. This decision does not define `PartialRetry` handling.

## Enforcement

None yet. The key-conflict cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will check pending and durable originals, the first conflicting index, rejection before revisions, unchanged offsets and revisions, and continued writer operation. Crash cases will verify that a rejected conflict adds no record.

## Revisit if

- The product needs to allow callers to replace a record's contents under the same key. That would change the retry contract and require a new decision.
