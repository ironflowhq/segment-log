---
id: "0024"
title: "Fully matching retries ignore original submit grouping"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, idempotency, durability]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md, docs/glossary.md]
enforced_by: none
---

# 0024. Fully matching retries ignore original submit grouping

## Context

Keys identify records across the whole log. Durable retries already match keys without checking original submit membership. ADR 0021 covers pending retries only when they replay one original submit in the same order. The format stores records and group boundaries, without original submit boundaries.

The owner approved accepting entirely matching retries regardless of original order or grouping during the M1 readiness discussion on 8 November 2026. This ADR extends ADR 0021, amends ADR 0008, and clarifies R1 and I5 under ADR 0009.

## Options & trade-offs

### Option A - accept entirely matching retries regardless of original grouping

- **For:** matching uses record identity already stored on disk. Acceptance is consistent before a flush, after it and after recovery. Subsets and combined retries append no duplicates.
- **Against:** an accidentally incomplete retry can succeed. Returned positions need not increase. Success does not prove that records originally committed atomically together.

### Option B - require the exact original submit while any match is pending

- **For:** catches changed membership and order while pending. Needs no new on-disk metadata.
- **Against:** acceptance depends on timing. A request rejected while pending can succeed after the flush.

### Option C - require the exact original submit for all retries

- **For:** catches changed membership and order before and after recovery.
- **Against:** requires persistent submit membership and ordering, or a submit-level identity. Changes the existing durable-retry rule and format.

## Decision

From M1, a submit is a fully deduplicated retry if every record has a distinct key and each key matches a pending or durable record with the same stream and payload. Matching ignores original submit membership, ordering and grouping. Expected revisions are ignored.

A retry may select a subset, reorder records, combine original submits, or mix pending and durable matches. It appends nothing and consumes no offset or revision. Normal admission checks and queue limits apply.

Success waits until every referenced record is durable and covered by the published watermark. The result has `deduplicated: true` and original positions in the retry's requested order. Offsets need not increase. All-durable matches need no new flush. Pending matches wait for their existing group's flush, without another append or flush solely for the retry.

A matched retry depending on a failed in-flight group gets `Indeterminate`, including when some matches were already durable. A request still queued when poisoning occurs gets `Poisoned`. ADR 0021's failure distinction is unchanged.

Original writes retain their all-or-nothing submit guarantee. A fully deduplicated retry confirms existing durable records. It creates no new transaction and does not assert that those records originally committed atomically together. This clarifies new writes and retry confirmation in R1 and I5.

We accept that a subset retry can hide a caller's omission in exchange for one record-level matching rule before and after durability and recovery.

## Consequences

- PRD open item 9 is resolved for M1. Key scope, repeated keys, pending waits, conflicts, mixed reused-and-new records and regrouped retries now have defined rules.
- Pending matching uses per-record identity and retry waiters, without comparing original submit membership.
- Positions follow request order. Callers cannot infer log order from a retry result's order.
- Conflicting keys follow ADR 0022. Mixing existing matches with new or unkeyed records follows ADR 0023. If no record matches an existing key, ordinary new-submit revision checks apply.
- [ADR 0031](0031-keys-live-with-retained-segments.md) confirms R13 across roll and reopen. The M4 lookup mechanism and key-cache memory budget remain open.
- No new frame or recovery metadata is needed.

## Enforcement

None yet. The M1 regrouped-retry cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will check subsets, reorderings, several original submits, pending and durable mixtures, delayed success, request-order positions, unchanged offsets and revisions, and failure reporting.

## Revisit if

- Callers need the log to verify an original operation's complete membership and order. That requires a submit-level identity contract.
