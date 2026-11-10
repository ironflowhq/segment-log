---
id: "0045"
title: "The latency target is measured at the throughput target's rate"
status: proposed
date: 2026-11-09
deciders: []
supersedes: null
superseded_by: null
tags: [performance, testing, targets]
touches: [docs/prd.md, docs/architecture/testing.md, segment-log-core/src/acceptance/evidence.rs]
enforced_by: none
---

# 0045. The latency target is measured at the throughput target's rate

## Context

The [PRD](../prd.md#footprint-and-performance-targets) has two append targets under the same conditions, with 1,000 caller threads: at least 50,000 records/s, and a p99 latency of at most the linger plus twice the device's `fsync` p99. It does not say how the callers arrive. The M1 benchmark runs a closed loop: each caller submits again as soon as it is answered.

In a closed loop, the mean latency is the number of callers divided by the throughput. The [paced benchmark record](../evidence/issue20/paced-benchmark/README.md) confirms it: five runs at 165,235–168,230 records/s predict a mean of 5.944–6.052 ms and measured 5.931–6.038 ms. Two things follow.

- The two targets describe different loads. A log that serves exactly 50,000 records/s to 1,000 closed-loop callers has a mean latency of 20 ms. The latency bound was about 2 ms in the issue 20 runs.
- In the closed loop, the latency target is a second throughput target. A mean of 2 ms needs 500,000 records/s, ten times the throughput target, and the p99 needs more.

So every closed-loop run has missed the bound, and it will keep missing it however far the log exceeds the throughput target. After the six issue 20 changes, the [combined record](../evidence/issue20/combined/README.md) shows 414,634 records/s with a p99 of 4.275 ms against a bound of 1.981 ms. When the same build is offered 50,000 records/s, its p99 is 2.045–2.958 ms against a bound of 1.884–2.037 ms.

These facts are fixed:

- The targets are provisional, and a measured miss is a finding ([ADR 0029](0029-m1-completion-requires-correctness-and-measurement-evidence.md)). Defaults and targets do not change without a record ([ADR 0036](0036-measure-the-existing-defaults-before-tuning.md)).
- `append` and `wait` block one caller thread for each request. The core has no client that sends without waiting. The M6 transport may add one.
- The benchmark has a paced variant since issue 20: each caller submits once in an interval, and latency runs from the scheduled send.

The owner asked for this record in the issue 20 discussion on 2026-11-09. It is a proposal until the owner accepts it.

## Options & trade-offs

### Option A - read the latency target from the closed loop

- **For:** one run gives both numbers. This is how every M1 report read the target so far.
- **Against:** the result depends on the caller count and the throughput, not on what a caller waits for at the planned load. The log misses the bound at eight times the throughput target. The comparison cannot guide work, because nothing short of 500,000 records/s changes its outcome.

### Option B - measure the latency at the throughput target's rate, with paced callers

- **For:** both targets describe one load: at 50,000 records/s from 1,000 callers, the p99 stays within the bound. The paced variant exists. Latency from the scheduled send counts the delay of a late caller.
- **Against:** each caller still has one request outstanding, so under a long stall the offered rate falls. One slow device flush of 0.4 s sets the p99 of a 20 s run, so a report needs several runs and a device without such stalls. It measures one rate.

### Option C - measure the latency with arrivals that do not wait for answers

- **For:** this is the usual model for a latency goal. No caller holds back load.
- **Against:** the core's API blocks a thread for each request, so the benchmark would need more threads than callers or a client that the core does not have before M6. It changes the PRD's "1,000 caller threads" condition.

### Option D - change the bound

- **For:** a bound with an allowance for processor work and wake-ups could be met in a closed loop.
- **Against:** it changes a target, not the way to measure it. No measurement on target hardware exists to choose the allowance. It stays possible later under ADR 0029 and ADR 0036.

## Decision

We will measure the latency target at the throughput target's rate, with paced callers (option B).

- **Throughput target.** The default closed-loop run measures it, as before.
- **Latency target.** The paced run measures it. The 1,000 callers together offer the throughput target: with 50,000 records/s, each caller submits once in 20 ms, and the callers are spread evenly over the interval. Latency runs from the scheduled send to the answer. The bound is the linger plus twice the device `fsync` p99 that the same run measures.
- **Closed-loop latency.** A report still gives it, as a diagnostic, beside the number of callers divided by the throughput.
- **Stalls.** A report gives every paced run. A run with a device stall is reported and explained, not removed.

This makes the two targets one statement about one load, with the variant that already exists. We accept that the paced run is not a true open loop and that its p99 needs a device without long flush stalls. This decision changes no target number, no default and no M1 gate decision.

## Consequences

- The PRD names the load of each target, and [testing.md](../architecture/testing.md#benchmarks-m1-for-the-core-m6-for-the-server) says which run measures which target.
- The closed-loop p99 misses in the M1 report and in issue 20 remain true measurements. They are no longer the target comparison.
- The first paced comparison is in the [combined record](../evidence/issue20/combined/README.md): on a virtual Linux guest, the p99 is 0.13 to 1.07 ms above the bound in five runs. That is still a miss there. Target hardware has not been measured.
- The required Linux benchmark report gains a paced run. The M6 guest benchmark must state its load model too.
- If the throughput target changes, the paced rate follows it.

## Enforcement

None. The benchmark prints a `PACED` line with the interval and the offered rate, so a report shows which load ran. Nothing checks that a report compares the latency target with a paced run.

## Revisit if

- A measurement on target hardware shows that the paced p99 cannot be measured reliably, for example because of device stalls.
- The M6 transport adds a client that sends without waiting, so that option C needs no extra threads.
- The planned deployment load is known and differs from 1,000 callers at the throughput target.
- The owner changes a target under ADR 0029 or ADR 0036.
