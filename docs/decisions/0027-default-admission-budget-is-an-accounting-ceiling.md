---
id: "0027"
title: "Default admission budget is an accounting ceiling"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, admission, memory]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/testing.md]
enforced_by: none
---

# 0027. Default admission budget is an accounting ceiling

## Context

ADRs 0025 and 0026 define admission reservations and retained-allocation charges. The default budget remained undecided. The workload targets small guests, but no M1 benchmark has measured a suitable limit yet.

The owner approved the initial default in [api.md](../architecture/api.md) during the M1 readiness discussion on 8 November 2026. This ADR amends ADRs 0025 and 0026. The API remains the only source for default values.

## Options & trade-offs

### Option A - start with a smaller configurable ceiling

- **For:** limits accepted request charges during storage stalls. Applications can raise the budget when their workload needs more outstanding work.
- **Against:** bursts of large or heavily preallocated requests can encounter `Full` sooner.

### Option B - start with a larger ceiling

- **For:** permits more outstanding work before byte-budget refusals.
- **Against:** allows more retained request memory without measured evidence that the workload needs it.

## Decision

Use the initial `pending_byte_budget` default in [api.md](../architecture/api.md). Applications can override it through `Options`. Queue-count capacity and the group-byte cap retain their existing defaults and remain independently configurable.

The budget is an accounting ceiling. Opening a log does not preallocate a buffer or arena sized to that budget. Reserving a charge accounts for accepted work; it does not allocate an equal-sized byte buffer.

The default is a provisional policy for the small-guest workload, not a measured optimum. M1 benchmarks report admission refusals and peak outstanding charges alongside the configured budget. A measurement can motivate changing the default through a new decision.

We accept earlier admission refusals in exchange for a smaller initial allowance for outstanding request charges.

## Consequences

- [ADR 0036](0036-measure-the-existing-defaults-before-tuning.md) retains this default alongside the other initial settings and requires a default baseline before separately reported tuning experiments.
- [ADR 0028](0028-too-large-is-distinct-from-temporary-full.md) resolves open item 10 with `TooLarge` for impossible size fits and `Full` for temporary exhaustion.
- The default is settled for M1. Applications can configure another budget without changing the format or durability contract.
- Increasing the budget alone does not preallocate more request storage. It permits more charges to be reserved as requests arrive.
- The admission budget remains separate from group size and total resident memory.
- A request that cannot fit the entire budget gets `TooLarge` under ADR 0028.

## Enforcement

None yet. The default-options checks in [testing.md](../architecture/testing.md#property-and-unit-tests) will verify the API defaults and independent configuration. The M1 benchmark report will include the configured budget, admission refusals and peak outstanding charges.

## Revisit if

- M1 measurements show that default-budget refusals obstruct the target workload, or the permitted outstanding charges exceed what the guest can afford.
