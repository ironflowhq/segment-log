---
id: "0002"
title: "Group commit and the watermark"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [write-path, durability]
touches: [docs/architecture/overview.md]
enforced_by: none
---

# 0002. Group commit and the watermark

## Context

Source: from the reviewed design (decision 3).

Many agents in one guest append to one log. The fleet runs tens of thousands of agents across many guests ([overview](../architecture/overview.md#placement)). One `fsync` costs from tens of microseconds to milliseconds, depending on the device. One `fsync` per append cannot reach the PRD throughput target on one device. A caller must still know when its append is durable.

## Options & trade-offs

### Option A - one `fsync` per group; ack and publish after it returns

- **For:** the flush cost is spread over every submit in the group. An ack means durable.
- **Against:** each append waits up to the linger time plus one `fsync`.

### Option B - one `fsync` per submit

- **For:** lowest latency for a single caller.
- **Against:** the throughput limit is the device's `fsync` rate. This is the shape the design rejects.

### Option C - ack before `fsync` (like Kafka with `acks=1` and `flush.messages` at never)

- **For:** fastest.
- **Against:** an ack does not mean durable. On one machine there is no replica to make up for it.

### Option D - `fsync` on a timer, separate from acks

- **For:** simple.
- **Against:** the same problem as C, or the same latency as A without the clear rule.

## Decision

We will call `fsync` once per group, and ack and publish only after it returns (option A). We accept that each append waits up to the linger time plus one `fsync`. The writer gathers submits until the linger ends or the byte cap is reached, writes them as one frame, and calls `fsync` once. Only after it returns success does the writer move the watermark, ack the submits, and let readers see the records. Two submits in one group are not atomic with each other ([ADR 0009](0009-caller-submit-is-the-atomic-unit.md)). The default linger, byte cap and roll size are in [api.md](../architecture/api.md).

## Consequences

- [ADR 0036](0036-measure-the-existing-defaults-before-tuning.md) keeps the existing defaults for the initial baseline. Later default changes require measurements, a recorded reason and corresponding code and documentation updates.
- Append latency depends on the linger and the device's `fsync` latency. The benchmark must report both.
- Readers never see a record that could still be lost.
- The contract depends on the device honoring the flush. The library cannot check that (PRD open item 3).

## Enforcement

None yet. Oracle checks 1 and 8 in [testing.md](../architecture/testing.md#the-oracle) will fail if the writer acks or publishes a record before its `fsync` returns. The M1 `strace` check in [ADR 0005](0005-flush-calls-and-platforms.md) will show one `fdatasync` per group.

## Revisit if

- Devices with power-loss-protected write caches make one `fsync` per submit cheap enough to meet the targets.
- A measurement shows that the linger alone causes the latency target to fail.
