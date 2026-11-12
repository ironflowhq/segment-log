---
id: "0050"
title: "The M4 open-time gate uses a warm page cache"
status: accepted
date: 2026-11-11
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [performance, recovery, milestones]
touches: [docs/prd.md, docs/architecture/testing.md, docs/evidence/m4/**]
enforced_by: none
---

# 0050. The M4 open-time gate uses a warm page cache

## Context

The M4 gate in the [PRD](../prd.md#milestones) says that open time for M4 conditions meets the 50 ms target, and that a Linux report gives open time cold and warm. The target row does not say which of the two the gate checks.

The [M4 Linux report](../evidence/m4/completion.md) measured the PRD fixture on a hosted ubuntu-24.04 runner: 100 sealed segments of 64 MiB with every record keyed, and a 4 MiB active segment. Keyed open took 38.6 ms warm and 122.8 ms cold (p50). The same fixture with no keys took 18.1 ms warm and 81.4 ms cold. So on that runner, a cold open misses the target even with no key frames to read. The cold time is set by the device, not by the log's design.

## Options & trade-offs

### Option A - the gate checks warm open, and the report gives cold open beside it

- **For:** it measures the work the log does. The cold number is still reported and can be compared from run to run.
- **Against:** a guest whose page cache is empty at start pays the cold time.

### Option B - the gate checks cold open

- **For:** it matches a guest that starts with an empty page cache.
- **Against:** on the measured runner no design meets it, because an open with no key frames already takes 81.4 ms cold. The gate would measure the disk.

## Decision

We will check the M4 open-time gate with a warm page cache (option A). The report keeps the cold measurement and states it as a measured result, not a pass.

## Consequences

- M4 passes its open-time part at 38.6 ms warm.
- Cold open, 122.8 ms keyed on the hosted runner, is a known finding. Deployment storage qualification before M6 ([ADR 0032](0032-qualify-deployment-storage-before-m6.md)) measures cold start on the real guest stack.
- The PRD target row names the warm page cache.

## Enforcement

None. The M4 completion record cites the warm and cold numbers.

## Revisit if

- The M6 guest benchmark shows that guests usually start with an empty page cache and the cold open time matters to callers. Loading the key filters after open returns is the next option ([ADR 0048](0048-sealed-segments-carry-a-key-filter-and-index.md)).
