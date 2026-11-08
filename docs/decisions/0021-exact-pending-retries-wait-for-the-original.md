---
id: "0021"
title: "Exact pending retries wait for the original"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, idempotency, durability]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: none
---

# 0021. Exact pending retries wait for the original

## Context

The writer checks keys before expected revisions so a retry can resolve despite a stale expected revision. ADR 0008 checks only durable keys and leaves a retry of a pending write to the revision check. That can reject a legitimate retry with `WrongRevision`.

The owner approved supporting exact pending retries in M1 during the readiness discussion on 8 November 2026. This ADR amends ADR 0008 and clarifies which retry waiters count as in flight under ADR 0004.

## Options & trade-offs

### Option A - wait for the original write

- **For:** an exact retry returns the original positions without another append or caller retry loop. A successful response still proves durability.
- **Against:** the writer must track pending keys and retry waiters until the group succeeds or fails.

### Option B - return a retryable key-pending error

- **For:** the writer need not retain a waiter for the original group.
- **Against:** the API needs another error and callers must retry again later.

### Option C - keep checking only durable keys

- **For:** no pending retry tracking.
- **Against:** a legitimate retry can fail with `WrongRevision` while the original write is pending.

## Decision

From M1, an exact pending retry waits for its original write's outcome. An exact pending retry has the same ordered keys, streams and payloads as one earlier submit accepted into the writer's current group. Every record must have a key. Expected revisions are not compared, as with a durable retry.

Normal admission checks and queue limits still apply. `append` returns a `Pending` handle without waiting for durability. The writer checks for the retry before checking expected revisions. Once matched, the retry becomes a waiter for the original submit and adds no records to the group. It consumes no new offset or revision.

After the original group's successful flush and watermark publication, the retry returns the original positions with `deduplicated: true`. It must not return success earlier. A retry matched to an in-flight submit gets `Indeterminate` if the original write does. A writer panic without an answer also gives `Indeterminate` through `Pending::wait`.

A request still in the queue when the writer is poisoned gets `Poisoned`, even if its keys would have matched. It has not yet become a retry waiter. This preserves ADR 0004's distinction between in-flight and queued requests.

We accept tracking pending keys and waiters so callers can retry the same operation without a second append or a false revision conflict.

## Consequences

- [ADR 0024](0024-fully-matching-retries-ignore-original-grouping.md) accepts fully matching retries regardless of original submit membership, ordering or grouping from M1 and resolves PRD open item 9.
- [ADR 0023](0023-reject-mixed-reused-and-new-records-from-m1.md) moves mixed reused-and-new rejection into M1.
- [ADR 0022](0022-reject-conflicting-key-reuse-from-m1.md) moves conflicting key reuse against pending and durable records into M1.
- PRD open item 8 is resolved and moves from M2 into M1.
- Matched retry waiters count as in flight for failure reporting even though their records are not written again.
- The writer compares pending streams and payloads against the buffered original submit. Durable comparisons still read the original record from its segment.
- Neither the watermark nor the durable key map includes an unsynced record merely because a retry matched it.
- A request processed after the original group becomes durable follows the existing durable-retry rule.
- Recovery needs no new frame or metadata. It rebuilds keys from the original records.

## Enforcement

None yet. The pending-retry cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will verify one append, delayed success, original positions, failure classification and crash recovery.

## Revisit if

- Pending retry waiters cannot be bounded within the admitted-request memory budget.
