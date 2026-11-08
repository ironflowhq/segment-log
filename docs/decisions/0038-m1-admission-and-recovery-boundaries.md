---
id: "0038"
title: "M1 admission and recovery boundaries"
status: proposed
date: 2026-11-08
deciders: []
supersedes: null
superseded_by: null
tags: [api, admission, recovery, milestones]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: none
---

# 0038. M1 admission and recovery boundaries

## Context

The owner requested documentation updates after the M1 specification review. The accepted retry, admission and durability decisions leave several boundary cases implicit. An implementer needs one order for simultaneous admission failures, valid option ranges and a directory-creation contract that preserves reachability after a power loss.

M1 writes one segment without roll. Multi-segment recovery starts in M3. `Pending::wait` already returns `Indeterminate` if the writer stops without an answer. The API is the only source for signatures, defaults and option rules. This proposal fills the remaining boundary details.

## Options & trade-offs

### Option A - define the boundaries before implementation

- **For:** gives callers and tests deterministic admission outcomes. Creating one directory level avoids an unimplemented recursive durability protocol. An unsupported layout fails before recovery can change it.
- **Against:** malformed requests can report input errors even on a poisoned log. Deployment must provision durable ancestors. M1 cannot open a rolled log produced by a later milestone.

### Option B - leave the boundaries to the implementation

- **For:** lets the first implementation choose its most convenient check order and layout handling.
- **Against:** tests and callers can encode different expectations. Recursive directory creation can publish records while an ancestor entry remains volatile.

## Decision

Use option A. [api.md](../architecture/api.md#admission-and-answer-ordering) defines structural checks before permanent size checks, then writer state, then current capacity. A stopped writer refuses later eligible appends with `Poisoned`; existing handles retain their documented lost-answer behavior. The [option rules](../architecture/api.md#option-validation) define nonzero capacities and roll size, a group cap that can hold a valid record, and valid zero linger. Defaults do not change.

`open` creates only the final directory component beneath already durable ancestors. It syncs the log directory and its parent before publication. A missing ancestor fails rather than triggering recursive creation. M1 and M2 reject multiple segment files before cleanup or recovery writes, using the existing `Io` variant with kind `Unsupported`. M3 adds the multi-segment algorithm.

The writer completes classified rejections and all-durable retry matches without waiting for another group flush. Pending matches still wait. Durable-key lookup failures use ADR 0004's current-group and queue outcomes. The API and test plan state these consequences alongside the distinction between pending revision checks and durable public heads.

We accept explicit deployment preparation and early layout refusal to keep M1's supported behavior small and testable.

## Consequences

- This proposal supplements the boundaries of ADRs 0001, 0004 and 0010. It does not replace their decisions.
- API, recovery and test documentation must keep the same check order and milestone scope.
- No new error variant, default value or on-disk field is needed.
- Recursive directory creation or support for later layouts would require its own durability and compatibility evidence.

## Enforcement

None yet. The M1 option, admission-precedence, terminal-answer, lookup-failure, directory-creation and supported-layout cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will enforce these boundaries. Runtime code is still a placeholder.

## Revisit if

- A caller needs recursive creation and can define which component makes every ancestor entry durable.
- A concrete downgrade workflow requires M1 to open a rolled log.
- A caller needs writer-state errors to take precedence over malformed input.
