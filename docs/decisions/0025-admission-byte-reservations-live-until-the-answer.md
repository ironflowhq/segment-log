---
id: "0025"
title: "Admission byte reservations live until the answer"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, admission, memory]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md, docs/glossary.md]
enforced_by: none
---

# 0025. Admission byte reservations live until the answer

## Context

The accept queue limits the number of queued submits, not their bytes. Requests can also leave the queue while remaining in a writer group or waiting for a pending original's durability. A queue-only byte limit would stop charging those unanswered requests.

The owner approved adding a configurable admission byte budget alongside the queue-count limit during the M1 readiness discussion on 8 November 2026. This ADR amends ADR 0010. The accounting method and default value remain PRD open item 10.

## Options & trade-offs

### Option A - keep queue-count capacity and add a byte budget through the answer

- **For:** limits accepted request charges across queued, grouped and matched-retry states. Supports varying request sizes without relying on queue count alone.
- **Against:** needs reservation accounting across success, rejection and failure. Admission can return `Full` even when the queue has a free slot.

### Option B - reduce queue-count capacity only

- **For:** no additional byte accounting.
- **Against:** restricts small requests based on a worst-case large request. The same queue count can retain very different amounts of request data.

### Option C - charge bytes only while a request is in the queue

- **For:** follows the existing queue lifecycle.
- **Against:** frees the charge before the request is answered. Writer groups and pending retry waiters can retain work outside that limit.

## Decision

M1 keeps the existing queue-count limit and adds a configurable admission byte budget. A submit must reserve its byte charge before acceptance. If the queue-count limit or unavailable byte budget prevents admission, `append` returns `Full` immediately. It does not wait for capacity.

An accepted request holds its reservation while queued, in a writer group, or waiting as a matched retry. Retries are charged too. Dequeueing, matching a retry, or freeing part of a request's data does not release its reservation. Release happens when the request is answered. It does not wait for the caller to consume that answer through `Pending::wait`.

A failed admission retains no reservation. Rejection, success, poisoning and writer shutdown must release reservations for completed requests. Dropping a caller's `Pending` handle does not free the reservation of a request that is still being processed. Cleanup must still release that reservation when processing ends.

We accept additional accounting and admission refusals to keep the charge attached to the full lifetime of accepted work.

## Consequences

- [ADR 0028](0028-too-large-is-distinct-from-temporary-full.md) resolves open item 10. `TooLarge` rejects impossible size fits or charge overflow before capacity reservation; `Full` applies to size-eligible requests that lack current capacity.
- [ADR 0027](0027-default-admission-budget-is-an-accounting-ceiling.md) sets the initial configurable default as an accounting ceiling, without preallocating a budget-sized buffer.
- [ADR 0026](0026-admission-charges-retained-allocations.md) sets retained-allocation accounting, including unused capacities and explicit bookkeeping and result allowances.
- Queue count and admission bytes have different lifetimes. A queue slot becomes free at dequeue; its request's byte reservation remains until the answer.
- The API gains `pending_byte_budget`. Requests that cannot fit the entire budget get `TooLarge` under ADR 0028.
- A reservation must not leak when queue insertion fails or a caller drops its result handle.
- This decision defines the reservation lifecycle. It does not claim a bound on total process memory. ADR 0026 defines what the charge includes.
- Existing `Full` backpressure remains nonblocking. Callers retain the same responsibility to retry later or shed load.

## Enforcement

None yet. The M1 admission-budget cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will check both limits, reservations held after dequeue and retry matching, release on all terminal paths, dropped result handles, and concurrent admission without budget overrun.

## Revisit if

- Callers need to distinguish queue-count exhaustion from byte-budget exhaustion through separate errors.
