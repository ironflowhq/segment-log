---
id: "0028"
title: "TooLarge is distinct from temporary Full"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, admission, errors]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: none
---

# 0028. TooLarge is distinct from temporary Full

## Context

M1 limits both encoded group size and retained-allocation admission charges. ADRs 0025–0027 define the byte budget but leave requests that cannot fit the entire budget unresolved. Retrying such a request later cannot help under the same configuration.

The owner approved distinguishing permanent size refusal from temporary exhaustion during the M1 readiness discussion on 8 November 2026. This ADR amends ADRs 0010, 0025 and 0026 and resolves PRD open item 10.

## Options & trade-offs

### Option A - use TooLarge for impossible fits and Full for temporary exhaustion

- **For:** callers can distinguish requests that need changing from requests that may fit after outstanding work completes. Reuses existing error variants.
- **Against:** `TooLarge` now covers both encoded group size and admission charge, so its documentation must explain both limits.

### Option B - use Full for all byte-budget refusals

- **For:** one response for any unavailable admission bytes.
- **Against:** callers can retry indefinitely when their request can never fit under the current configuration.

## Decision

From M1, `append` returns `AppendError::TooLarge` before acceptance if the encoded submit cannot fit in one group, its admission charge exceeds the entire configured byte budget, or calculating its charge overflows.

For a request that fits those size limits, `append` returns `AppendError::Full` if the queue is currently full or outstanding requests leave insufficient byte budget. Equality with a size limit is allowed; admission still needs the corresponding available capacity.

Permanent size checks precede capacity reservation and queue insertion. Neither `TooLarge` nor `Full` accepts a request or retains a reservation. Neither consumes an offset or revision, changes key state, or poisons the writer. Structural input validation continues to use `Invalid` under the existing rules.

The distinction applies to new submits and retries alike. A later retry of an unchanged `TooLarge` request cannot succeed under the same limits merely because other requests finish. The caller must reduce the request's retained allocation or encoded size, or change the relevant configured limit.

We accept broadening the documented meaning of `TooLarge` so callers can distinguish size eligibility from temporary capacity.

## Consequences

- PRD open item 10 is resolved for M1, including retained-allocation accounting, the configurable default, reservation lifecycle and size refusal.
- ADR 0025's `Full` rule applies to size-eligible requests. It does not apply to a request that cannot fit the entire budget.
- Charge arithmetic uses checked operations and returns `TooLarge` rather than wrapping, panicking or accepting an unrepresentable charge.
- The core API needs no new error variant or on-disk metadata.

## Enforcement

None yet. The M1 permanent-size and temporary-exhaustion cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will check empty and occupied budgets, exact-boundary requests, charge overflow, no reservation leaks, and successful admission after temporary pressure clears.

## Revisit if

- Callers need separate errors for encoded-size and retained-allocation limits rather than one `TooLarge` variant.
