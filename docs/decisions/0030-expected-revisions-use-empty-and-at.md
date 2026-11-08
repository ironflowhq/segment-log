---
id: "0030"
title: "Expected revisions use Empty and At"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, revisions, concurrency]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/testing.md, docs/glossary.md]
enforced_by: none
---

# 0030. Expected revisions use Empty and At

## Context

Every new record needs a stream-state precondition. ADR 0008 proposes `Empty` and `At(n)`, but PRD open item 2 leaves the vocabulary provisional until M2. M1 already implements expected-revision checks.

The owner approved these choices for M1, without unconditional `Any` appends, during the readiness discussion on 8 November 2026. This ADR confirms the vocabulary part of ADR 0008 without accepting its other proposed material as a whole.

## Options & trade-offs

### Option A - Empty and At only

- **For:** distinguishes a stream with no records from a stream at revision zero. Every new append states its concurrency precondition.
- **Against:** callers that do not know the current revision must read the head and handle races with other callers.

### Option B - add Any for unconditional appends

- **For:** callers that do not need a concurrency precondition can append without first reading the head.
- **Against:** introduces another write policy and allows callers to bypass expected-revision protection.

## Decision

M1 uses the expected-revision variants in [api.md](../architecture/api.md). `Empty` requires a stream with no records. Its first successful record gets revision zero. `At(n)` requires the stream's current last revision to equal `n`; its next successful record gets revision `n + 1`. There is no unconditional `Any` variant in M1.

A wrong expected revision rejects the entire submit with `WrongRevision` at the failing record's index. The error carries the current revision, or `None` if the stream has no records. The rejected submit consumes no offset or revision and leaves no pending changes for later submits.

The existing ordering rules remain. A record sees earlier records in its submit and earlier valid pending submits. For successive new records in one stream, the later record must expect the revision assigned to the earlier one. Fully matching retries check keys first and ignore expected revisions under ADR 0024.

We accept that a caller without a known head must read `stream_head` and handle a possible concurrent append, rather than bypassing the precondition.

## Consequences

- Revision checks and `WrongRevision.actual` include earlier valid pending records. Public `stream_head` reports only durable records. Reading it gives a durable starting point, not a guarantee that a concurrent append or a pending revision will leave the next precondition valid. A rejection does not acknowledge the pending records it observed.
- PRD open item 2 is resolved for M1. Missing-stream state and revision zero remain distinct.
- This decision adds no API variant or on-disk field. It adopts the existing API sketch's choices.
- The public API remains provisional overall. This decision settles M1's preconditions rather than freezing every signature.
- A future unconditional append policy needs a separate decision with a concrete caller need.

## Enforcement

None yet. The M1 expected-revision cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will check missing streams, revision zero, sequential records in one submit, earlier pending revisions, whole-submit rejection, no offset or revision consumed, and stale expectations on fully matching retries.

## Revisit if

- A concrete caller needs unconditional appends and the concurrency trade-off justifies another policy.
