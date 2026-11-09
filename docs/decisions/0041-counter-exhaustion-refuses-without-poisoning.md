---
id: "0041"
title: "Counter exhaustion refuses without poisoning"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, admission, recovery]
touches: [docs/architecture/api.md, docs/architecture/testing.md, segment-log-core/src/**]
enforced_by: test
---

# 0041. Counter exhaustion refuses without poisoning

## Context

M1 implementation review found no caller-facing outcome for exhaustion of a log offset, stream revision or writer epoch. Wrapping counters breaks identity. Panicking or returning an I/O error would conflate exhaustion with writer failure. The owner accepted explicit errors, refusal without writes or poisoning, and epoch refusal before recovery mutations in the M1 implementation discussion.

## Options & trade-offs

### Option A - explicit errors

- **For:** distinguishes a known refusal from an unknown write outcome and leaves unrelated streams usable.
- **Against:** adds error variants that callers must handle.

### Option B - permanent read-only state with existing errors

- **For:** avoids new variants.
- **Against:** stops unrelated appends and gives existing errors a second meaning.

## Decision

Use option A. The [API](../architecture/api.md#counter-exhaustion) defines the errors, ordering and boundary values. No format field changes. Refused submits retain no pending changes, and epoch exhaustion cannot trigger recovery writes.

## Consequences

- Exhaustion is a terminal rejection, not an indeterminate outcome.
- A stream at its revision limit does not prevent writes to another stream.
- Recovery must finish validating the kept prefix and checking its epoch before truncating or rewriting the segment.
- Counter fixtures need private test setup because filling the offset space is impractical.

## Enforcement

The [counter scenarios](../../segment-log-core/src/acceptance/counters.rs) exercise `COUNTER-EXHAUSTION` from [testing.md](../architecture/testing.md).

## Revisit if

- A caller needs counters wider than the current format permits.
- A future read-only API needs to inspect a log whose writer epoch is exhausted.
