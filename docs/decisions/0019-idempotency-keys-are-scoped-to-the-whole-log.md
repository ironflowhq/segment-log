---
id: "0019"
title: "Idempotency keys are scoped to the whole log"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, idempotency]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/testing.md, docs/glossary.md]
enforced_by: none
---

# 0019. Idempotency keys are scoped to the whole log

## Context

M1 needs a key map that returns the original positions for a durable retry. PRD open item 9 leaves key scope undecided. ADR 0008 already compares the stream and payload after finding a key, but describes whole-log scope as an assumption.

The owner confirmed whole-log scope during the M1 readiness discussion on 8 November 2026. This ADR amends the scope assumption in ADR 0008.

## Options & trade-offs

### Option A - scope keys to the whole log

- **For:** a retry sent to a different stream finds the original key and follows the key-conflict rule. It cannot appear to be a new operation just because its stream changed.
- **Against:** callers must choose keys across all streams in the log. Independent streams cannot reuse local counters as keys without a namespace.

### Option B - scope keys to one stream

- **For:** each stream can choose its keys independently.
- **Against:** changing the stream on a retry makes it a new operation. The log cannot detect that routing mistake through its key check.

## Decision

An idempotency key is scoped to the whole log. Lookup uses the key bytes alone. The writer then compares the original stream and payload under the same-bytes rule in [api.md](../architecture/api.md#behavior-notes).

Reusing a retained key on a different stream follows the key-conflict rule, even if the payload matches. Callers must choose keys across all streams in the log. A UUID is one option, not a required encoding.

We accept that callers must coordinate their key namespaces to detect retries sent to the wrong stream.

## Consequences

- [ADR 0023](0023-reject-mixed-reused-and-new-records-from-m1.md) moves mixed reused-and-new rejection into M1. [ADR 0024](0024-fully-matching-retries-ignore-original-grouping.md) resolves open item 9 by accepting fully matching regrouped retries from M1.
- [ADR 0022](0022-reject-conflicting-key-reuse-from-m1.md) moves conflicting key reuse against pending and durable records into M1.
- [ADR 0020](0020-reject-repeated-keys-within-a-submit.md) amends the key rules by rejecting repeated keys within one submit before admission, starting in M1.
- The key map and the recovery model use whole-log key scope.
- The stream is part of the comparison after lookup, not part of the lookup key.
- [ADR 0031](0031-keys-live-with-retained-segments.md) confirms R13 across roll and reopen. Optimized lookup and key-cache memory remain M4 decisions.
- [ADR 0021](0021-exact-pending-retries-wait-for-the-original.md) resolves open item 8 by making exact pending retries wait for the original write's outcome from M1.
- This decision settles scope only. It does not move `KeyConflict` or `PartialRetry` into M1.

## Enforcement

None yet. The M1 cross-stream key-conflict case in [testing.md](../architecture/testing.md#property-and-unit-tests) will verify the rule before and after reopen.

## Revisit if

- Callers require independent stream-local key namespaces and cannot include a namespace in the key bytes.
