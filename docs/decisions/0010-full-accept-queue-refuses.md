---
id: "0010"
title: "A full accept queue refuses the append"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, write-path]
touches: [docs/architecture/api.md]
enforced_by: none
---

# 0010. A full accept queue refuses the append

## Context

Source: from the reviewed design (decision 7).

When callers append faster than the device can flush, the accept queue fills. Kafka's producer blocks for `max.block.ms` (default 60 seconds) in this case. In a guest with thousands of agents, blocked callers hide the overload and hold memory.

## Options & trade-offs

### Option A - refuse at once with `Full`

- **For:** the caller sees backpressure and decides: retry later, shed load, or fail. The queue size limits memory.
- **Against:** callers must handle `Full`.

### Option B - block until there is space, with a timeout

- **For:** simpler for callers.
- **Against:** hides overload. Blocked threads hold memory. The timeout is a second policy to tune.

## Decision

We will refuse an append at once with `Full` when the accept queue is full (option A). We accept that callers must handle `Full`. `append` never blocks on a full queue. It returns `Full`. The queue capacity is an option ([api.md](../architecture/api.md)).

## Consequences

- [ADR 0038](0038-m1-admission-and-recovery-boundaries.md) proposes a deterministic admission order when malformed input, impossible size, writer failure and capacity pressure overlap. The API defines the order and option bounds.
- [ADR 0028](0028-too-large-is-distinct-from-temporary-full.md) limits `Full` to temporary capacity exhaustion of size-eligible requests. Impossible size fits and charge overflow get `TooLarge` before capacity reservation.
- [ADR 0025](0025-admission-byte-reservations-live-until-the-answer.md) adds a configurable byte budget alongside queue-count capacity from M1. Byte reservations last until the request is answered, including grouped requests and matched retry waiters.
- Queue capacity still counts queued submits. [ADR 0026](0026-admission-charges-retained-allocations.md) defines retained-allocation charges. [ADR 0027](0027-default-admission-budget-is-an-accounting-ceiling.md) sets the initial default. ADR 0028 resolves requests that cannot fit it.
- A caller that wants to wait builds that on top, for example with a backoff.

## Enforcement

None yet. The M1 admission-budget cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will check immediate `Full` when either limit prevents admission.

## Revisit if

- Every consumer wraps `Full` in the same blocking loop. Then a blocking variant in the library may be simpler.
