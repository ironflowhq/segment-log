---
id: "0023"
title: "Reject mixed reused and new records from M1"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, idempotency, errors]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: none
---

# 0023. Reject mixed reused and new records from M1

## Context

M1 recognizes full durable retries and exact pending retries. It also rejects a reused key with a different stream or payload. The API's durable-key rules already propose `PartialRetry` for a submit that mixes matching records with new ones, but previously schedule that response for M2.

A new submit can reuse a matching pending or durable record alongside a fresh record. It needs a defined outcome before M1 can accept arbitrary submits. The owner approved rejecting the entire mixed submit during the M1 readiness discussion on 8 November 2026.

## Options & trade-offs

### Option A - reject the whole mixed submit

- **For:** the submit has one outcome. The log does not combine reused positions with newly appended positions in a successful append.
- **Against:** callers must retry their original submit or send a separate operation with fresh keys.

### Option B - append only the new records

- **For:** a caller can combine previously submitted records with fresh records in one request.
- **Against:** a successful append can contain both old and new positions. Revision checks and retry outcomes need additional rules for that combination.

## Decision

From M1, the writer rejects the entire submit with `CommitError::PartialRetry` if at least one record matches a pending or durable record with the same key, stream and payload, and another record has a new key or no key.

The rejected submit writes nothing and consumes no offset or revision. It creates no key reservation and changes no stream head. Existing submits and their retry waiters are unchanged. The rejection does not poison the writer or wait for the matching pending original to become durable.

`KeyConflict` takes precedence if any reused key has a different stream or payload. The writer finishes checking for conflicts before returning `PartialRetry`. Both responses precede expected-revision checks. Intrinsic admission checks, including repeated-key rejection, still happen before the writer processes the submit.

If no record matches an existing key, `PartialRetry` does not apply. The submit follows the ordinary expected-revision checks. A mixture of fresh keyed records and unkeyed records is allowed, subject to the other checks.

We accept rejecting mixed submits so callers and recovery have no successful append that combines new writes with deduplicated records.

## Consequences

- [ADR 0024](0024-fully-matching-retries-ignore-original-grouping.md) accepts fully matching retries regardless of original submit membership, ordering or grouping from M1 and resolves PRD open item 9.
- This decision amends ADR 0008 and moves `PartialRetry` from M2 into M1 for both pending and durable matches.
- Rejection does not acknowledge the durability of any matching pending record.
- Callers can retry their original submit or submit a separate new operation with fresh keys.
- A submit made entirely of matching existing records is not a mixed reused-and-new submit. Full durable retries and exact pending retries retain their rules.
- No new frame or recovery metadata is needed.

## Enforcement

None yet. The M1 partial-retry cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will check pending and durable matches, unkeyed records, conflict precedence, unchanged state, continued writer operation and crashes after rejection.

## Revisit if

- Callers need a successful submit to combine reused and new records and can define the required revision and result semantics.
