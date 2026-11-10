---
id: "0044"
title: "Woken callers relay a group's acks"
status: accepted
date: 2026-11-09
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [group-commit, performance, write-path]
touches: [segment-log-core/src/engine/reply.rs, segment-log-core/src/engine.rs, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: test
---

# 0044. Woken callers relay a group's acks

## Context

After a group's `fsync`, the writer thread answered each request with one channel send. A send to a caller that is parked in `wait` makes one wake-up system call. The writer therefore made one wake-up call for each caller of the group, one after another, and it could not start the next group until the last one returned.

The [issue 20 trace](../evidence/issue20/ack-relay/README.md) ran the PRD's 1,000 caller threads on a Linux guest. The writer spent 3.23 ms of each 5.87 ms group cycle on these wake-ups. The write and the `fsync` together took 0.45 ms. The wake-ups, not the device, limited throughput and set most of the latency.

These facts are fixed:

- An ack follows its group's `fsync` (I1, I3). The delivery method cannot change when an answer exists, only how its caller learns of it.
- One thread writes the files ([overview](../architecture/overview.md#components)).
- The core uses only `std` ([ADR 0012](0012-no-log-crate-dependency.md)) and forbids `unsafe`. `std` has no call that wakes many threads at once without a shared lock.

The owner chose the relay over a second thread in the issue 20 discussion on 2026-11-09.

## Options & trade-offs

The wake-up figures are medians for 1,000 parked threads from the [microbenchmark](../evidence/issue20/ack-relay/README.md#wake-up-strategies) on the same guest.

### Option A - the writer wakes every caller

- **For:** an ack depends on no thread except the writer. This is the behavior before this decision.
- **Against:** the writer pays for every wake-up: 3.4 ms for 1,000 callers. The cost grows with the group and delays the next group.

### Option B - a second thread wakes the callers

- **For:** the writer returns to the next group at once. No caller depends on another caller.
- **Against:** the wake-ups still run one after another, so the last of 1,000 callers still waits 3.4 ms. That is longer than the group cycle the relay reaches, so the thread falls behind unless it fans out too. It adds a second engine thread with its own queue, shutdown and failure cases.

### Option C - one condition variable for each group

- **For:** one call from the writer.
- **Against:** not faster than option A in the measurement (3.4 to 3.5 ms to the last caller). Every woken caller must take the same lock.

### Option D - woken callers relay the wake-ups

- **For:** the writer wakes four callers in 46 µs, whatever the group size. The other wake-ups run in parallel on the callers' threads, and the last of 1,000 callers wakes after 2.1 ms. It needs only `std`, no `unsafe` and no new thread.
- **Against:** a parked caller's wake-up depends on the callers above it being scheduled. Each woken caller makes up to four wake-up calls for others before `wait` returns.

### Option E - one system call that wakes many threads

- **For:** the least work for the writer.
- **Against:** it needs `unsafe` or a dependency, and platform code for Linux and macOS.

## Decision

We will let woken callers relay the wake-ups (option D).

- The writer records every answer of a durable group before it wakes any caller. An answer never depends on the relay. A caller that calls `wait` later finds its answer.
- The callers of a group form a tree with a fan-out of four. The writer wakes the first four. A woken caller wakes its four children inside `wait`, before `wait` returns, so caller code cannot delay the relay. In the [fan-out comparison](../evidence/issue20/ack-relay/README.md#fan-out), fan-outs of 2 and 4 gave the same throughput and 8 and 16 about 5% less. Four has half the depth of two.
- A caller that is not parked in `wait` cannot relay. Whoever reaches its place wakes its children instead. No caller depends on a handle that is dropped, unused or not yet waited on.
- The relay covers the answers of a durable group: its new submits and its matched retry waiters. The writer still sends every other answer directly: a rejection, an all-durable retry, an exhausted counter and every poison answer.

This option removes the cost from the writer without a second thread, `unsafe` or a dependency. We accept that a parked caller's wake-up now depends on the scheduling of at most four other callers in a group of 1,000.

## Consequences

- The writer's delivery cost no longer grows with the number of parked callers. In the [measurement](../evidence/issue20/ack-relay/README.md) it fell from 3.23 ms to 0.26 ms for each group of 1,000.
- A caller's thread does work for other callers: at most four wake-up calls, plus a walk over places whose callers are not parked.
- If the operating system delays a woken caller's thread, the callers below it wait longer for `wait` to return. Their answers already exist, and durability, the watermark and reads are not affected.
- The request's reply state is now a concrete type, so the [admission charge](../architecture/api.md#admission-charge) includes it in `bookkeeping`. The charge of each request grows by the size of that type.
- `ACK-DELIVERY` in [testing.md](../architecture/testing.md) defines the cases that every delivery change must keep.

## Enforcement

The `ACK-DELIVERY` scenarios in `segment-log-core/src/acceptance/delivery.rs` park callers below callers that are late, idle or gone. They fail under their watchdog if a relay drops a place or a woken caller does not relay. The seeded trace oracle runs the relay with real caller threads.

Nothing measures the wake-up latency in CI. The M1 benchmark reports it.

## Revisit if

- Callers run at different scheduling priorities and a measurement shows late wake-ups below a slow caller.
- Callers stop parking threads, for example behind the M6 transport, so that a wake-up is no longer a system call for each caller.
- An ADR admits `unsafe` or a dependency that can wake many threads with one call.
- A measurement on the target hardware favors another fan-out.
