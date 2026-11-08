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

- The queue capacity limits the number of submits, not their bytes, so it does not by itself limit memory. That is PRD open item 10.
- A caller that wants to wait builds that on top, for example with a backoff.

## Enforcement

None yet. A unit test that fills the queue and expects `Full` would enforce it. [testing.md](../architecture/testing.md) does not list one yet.

## Revisit if

- Every consumer wraps `Full` in the same blocking loop. Then a blocking variant in the library may be simpler.
