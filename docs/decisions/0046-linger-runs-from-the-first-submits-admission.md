---
id: "0046"
title: "A group's linger runs from the admission of its first submit"
status: proposed
date: 2026-11-10
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [group-commit, performance, write-path]
touches: [segment-log-core/src/engine.rs, docs/architecture/overview.md, docs/architecture/testing.md, docs/glossary.md]
enforced_by: test
---

# 0046. A group's linger runs from the admission of its first submit

## Context

The writer gathers submits into a group until the linger ends or the byte cap is reached ([ADR 0002](0002-group-commit-and-the-watermark.md)). The linger started when the writer took the group's first submit from the accept queue.

The writer takes nothing from the queue while it writes and flushes a group. A submit that arrived at the start of that flush therefore waited three times: for the rest of the flush, then for a whole linger, then for its own flush. With no processor work at all, its latency was one flush, plus the linger, plus one flush.

That sum is the PRD's latency bound: the linger plus twice the device `fsync` p99 ([targets](../prd.md#footprint-and-performance-targets)). The bound left nothing for encoding, publication or the wake-up of the caller. The [combined record](../evidence/issue20/combined/README.md) shows the result at the paced rate of [ADR 0045](0045-latency-target-is-measured-at-a-paced-rate.md): the p99 was 0.13 to 0.25 ms above the bound in four runs of five.

ADR 0002 accepted a smaller wait: "each append waits up to the linger time plus one `fsync`". The implementation exceeded it by the time a submit spent queued behind the previous flush.

These facts are fixed:

- An ack follows its group's `fsync` (I1). One thread writes the files.
- The writer takes submits in admission order. A zero linger takes only what is already queued ([ADR 0043](0043-zero-linger-and-empty-read-boundaries.md)).
- Defaults and targets do not change without a record ([ADR 0036](0036-measure-the-existing-defaults-before-tuning.md)).

The [linger record](../evidence/issue20/linger-admission/README.md) holds the measurements for this decision.

## Options & trade-offs

### Option A - the linger runs from the moment the writer takes the first submit

- **For:** the behavior before this decision. A group gathers for a whole linger after the previous flush, so a moderate load makes the fewest flushes. A request carries no time.
- **Against:** a submit can wait one flush longer than ADR 0002 accepted. The latency bound has no allowance for processor work, so a correct log on a fast device still misses it.

### Option B - the linger runs from the admission of the group's first submit

- **For:** the time a submit spent queued behind the previous flush counts toward its linger. The worst wait before a group is written is the linger, or the rest of the previous flush if that is longer. This is the wait that ADR 0002 accepted, and it leaves one flush of the bound for processor work.
- **Against:** at a moderate load the writer closes a group after each linger, not after each linger plus a flush, so it flushes more often. `append` reads the clock once for each admitted request, and each request holds one more value.

### Option C - shorten the default linger

- **For:** no change to the writer.
- **Against:** it changes a default, and the bound contains the linger, so the bound shrinks by the same amount. A submit still waits flush, linger, flush.

### Option D - write at once whatever queued during the previous flush

- **For:** the lowest latency. The previous flush acts as the linger.
- **Against:** the configured linger stops meaning anything under load. Groups shrink to what arrives during one flush, and the flush rate rises to one for each flush time. No measurement here supports that device load.

## Decision

We will count a group's linger from the admission of its first submit (option B).

- `append` records the time when it queues a request.
- The linger of a group ends one linger after the admission of the first new submit that joins it. Until then the writer waits for more submits, as before.
- After the linger has ended, the writer takes only submits that are already queued, up to the byte cap. This is the zero-linger rule of ADR 0043. A flush that outlasts the linger therefore does not split its backlog into groups of one.
- A submit that arrives alone still waits its whole linger for company.
- Rejections and retries that match only durable records are answered when they are classified, as before. They do not start a linger.

This option gives each submit the wait that ADR 0002 already promised, without a new default. We accept more flushes at a moderate load, one clock read for each request, and a larger request.

## Consequences

- This decision amends ADR 0002: the linger is a bound on the wait of a group's first submit, not a pause that the writer adds after each flush.
- The worst wait before a group is written falls by up to one flush. The linger record gives the measured change at the paced rate.
- While submits keep arriving, the writer starts at most one group for each linger. With the default linger that is up to 1,000 flushes each second. Before, the limit was one group for each linger plus flush. The storage under the log must sustain that flush rate at the planned density ([V11](../architecture/volume-requirements.md#v11-flush-latency-allows-the-throughput-target)).
- When the device is slower than the linger, every group is ready as soon as the previous flush returns. Group size then follows the flush time, as it did before under a full queue.
- The [admission charge](../architecture/api.md#admission-charge) of each request grows by the size of one `Instant`, through `bookkeeping`.
- The `GROUP-FORMATION` case in [testing.md](../architecture/testing.md) states the rule.

## Enforcement

Two scenarios in `segment-log-core/src/acceptance/durability.rs`. `linger_runs_from_the_admission_of_a_groups_first_submit` holds a flush for longer than the linger and fails if the next group waits another linger or splits its queued submits. `a_group_formed_after_its_linger_ended_fails_and_recovers_whole` fails that group's flush and replays each of its storage boundaries through every crash mode.

Nothing in CI measures the latency. The M1 benchmark reports it.

## Revisit if

- A measurement on target hardware, or at the planned density on shared media, shows that the higher flush rate at a moderate load costs more than the shorter wait gains.
- A measurement supports option D.
- A default linger change is proposed under ADR 0036.
