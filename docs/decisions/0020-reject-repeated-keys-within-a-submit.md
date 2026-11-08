---
id: "0020"
title: "Reject repeated keys within a submit"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, idempotency, admission]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: none
---

# 0020. Reject repeated keys within a submit

## Context

A submit holds one or more records and commits all of them or none. Keys are scoped to the whole log under [ADR 0019](0019-idempotency-keys-are-scoped-to-the-whole-log.md). M1 must return the original positions for a durable retry.

If two records in one submit use the same key, a key-to-position map cannot represent both original positions. The API previously left this case in PRD open item 9. The owner approved rejecting the submit before admission during the M1 readiness discussion on 8 November 2026.

## Options & trade-offs

### Option A - reject the entire submit before admission

- **For:** each key identifies one record. Invalid input consumes no queue slot and cannot change the log.
- **Against:** admission must inspect the keys for repeats. Callers must give distinct records distinct keys.

### Option B - collapse repeated keys within the submit

- **For:** callers can repeat an identical keyed record without an error.
- **Against:** fewer records are written than the caller supplied. Returning positions needs an additional rule, and collapsing records can hide caller mistakes.

### Option C - allow one key to identify multiple records

- **For:** a caller can reuse a key inside one atomic submit.
- **Against:** recovery and retry matching need rules for choosing among multiple original positions for the same key.

## Decision

From M1, `append` rejects the entire submit with `AppendError::Invalid` before queuing it if any two records carry the same key bytes. The rule applies even when their streams and payloads match. A difference in stream, payload or expected revision does not make the repeated key valid.

Nothing is queued or written. No log offset or stream revision is consumed. Multiple records without a key are allowed, subject to the other validation and expected-revision rules.

We accept checking keys at admission to keep one key associated with one record. A legitimate retry resends a submit whose records have distinct keys.

## Consequences

- [ADR 0023](0023-reject-mixed-reused-and-new-records-from-m1.md) moves mixed reused-and-new rejection into M1.
- [ADR 0022](0022-reject-conflicting-key-reuse-from-m1.md) moves conflicting key reuse against pending and durable records into M1.
- This decision amends the key rules in ADR 0008 and resolves repeated keys within a submit from PRD open item 9.
- Both new submits and retries must pass this admission check.
- The rule depends only on the submitted records. Admission does not need access to the writer's pending or durable key maps.
- [ADR 0024](0024-fully-matching-retries-ignore-original-grouping.md) resolves open item 9 by accepting fully matching regrouped retries from M1. [ADR 0021](0021-exact-pending-retries-wait-for-the-original.md) resolves open item 8 by making exact pending retries wait for the original write's outcome from M1.

## Enforcement

None yet. The repeated-key admission cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will check rejection, unchanged log state and queue capacity, and the control case with multiple unkeyed records.

## Revisit if

- Callers need one key to identify an entire multi-record submit rather than an individual record.
