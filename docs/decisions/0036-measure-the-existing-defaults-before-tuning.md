---
id: "0036"
title: "Measure the existing defaults before tuning"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, performance, testing, milestones]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/testing.md]
enforced_by: none
---

# 0036. Measure the existing defaults before tuning

## Context

The API specifies initial linger, group-byte-cap and roll-size defaults. PRD open item 7 leaves tuning to measurements. The queue-count and admission-budget defaults have also been retained or settled during the M1 readiness discussion.

The owner approved keeping those existing defaults as the initial baseline and requiring measurement-backed changes on 8 November 2026. The API remains the only source for numeric defaults.

## Options & trade-offs

### Option A - retain the defaults and measure a baseline first

- **For:** gives later experiments a reproducible reference. Separates measurements under defaults from results obtained with changed settings.
- **Against:** the initial run may miss provisional targets. The defaults are not yet a measured optimum.

### Option B - adjust defaults before the first baseline

- **For:** can tailor the initial settings to estimates of the expected workload.
- **Against:** lacks measurement evidence for the change and loses a direct baseline for the current specification.

## Decision

Retain the existing linger, group-byte-cap and roll-size defaults in [api.md](../architecture/api.md). Queue-count capacity and the admission-byte budget keep their already agreed defaults. Applications can still supply explicit options within the existing limits.

Benchmark defaults first for the features implemented in that milestone. M1 measures linger and group behavior while remaining single-segment without roll. The roll-size setting becomes effective from M3, not M1.

Report experiments with changed options separately, with exact configuration and reproducibility information. Keep their results distinct from the default baseline and preserve the same correctness and durability contract across variants.

A change to a default requires measurement evidence, a recorded reason, and corresponding code and documentation updates in the same change. Record the decision in an ADR. A missed provisional M1 performance target remains a finding under ADR 0029; it does not justify silently changing settings or targets.

We accept that the baseline may expose performance work rather than assuming that an unmeasured configuration meets the targets.

## Consequences

- PRD open item 7 is settled as an initial configuration and tuning policy. Tuning itself remains measurement work.
- The baseline and any experimental variants can be compared without confusing different configurations.
- This decision does not add roll, adaptive tuning or a new runtime option to M1.
- No numeric default changes in this decision, and no target pass or measured optimum is claimed.

## Enforcement

None yet. The benchmark reporting requirements in [testing.md](../architecture/testing.md#benchmarks-m1-for-the-core-m6-for-the-server) require the default baseline, exact variant settings and evidence for any proposed default change. API defaults remain the values used by the default-options checks.

## Revisit if

- Reproducible measurements show another default better serves the target workload while preserving the contract.
