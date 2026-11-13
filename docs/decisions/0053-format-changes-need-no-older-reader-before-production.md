---
id: "0053"
title: "Format changes need no reader for older versions until the first production release"
status: accepted
date: 2026-11-13
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [format, compatibility]
touches: [docs/architecture/format.md, docs/architecture/testing.md, docs/prd.md, segment-log-core/src/codec/header.rs, segment-log-core/src/acceptance/format_golden.rs, segment-log-core/tests/fixtures/**]
enforced_by: test
---

# 0053. Format changes need no reader for older versions until the first production release

## Context

[ADR 0052](0052-freeze-format-version-1.md) froze the byte layout at `format_version` 1 for M5. It also set the rule for later changes: a change increments `format_version`, and a build reads every frozen version from 1 up to its own. Its Consequences say that data written from 2026-11-13 can outlive later builds.

These facts are fixed:

- The log is not in production. No deployment keeps data in it. The owner stated on 2026-11-13 that breaking changes are acceptable until the first production release, and that each choice should be the correct one, not the one that avoids a break.
- The M5 review left work that may change the layout: a cross-check of the footer against the groups, a measurement of key index page alignment, and a review of reserved and unused fields. Under ADR 0052, each such change costs a version 1 reader kept for good, plus fixtures for it. That cost would push each choice toward leaving the layout as it is.
- A reader rejects a segment whose `format_version` is not its own with `OpenError::UnsupportedVersion` (`segment-log-core/src/engine/survey.rs`). An old log therefore fails clearly under a new build, and is never misread.
- `FORMAT-GOLDEN` pins the layout with committed fixture files. A change to the encoder fails it ([testing.md](../architecture/testing.md)).
- [ADR 0013](0013-provisional-v0-byte-layout.md) ran the same policy before M5: a change incremented `format_version`, with no migration and no reader for older versions.

## Options & trade-offs

### Option A - keep ADR 0052's rule: a reader for every older version

- **For:** any log written today opens in every later build.
- **Against:** no log needs that before production. Each pre-production fix to the layout adds a reader, its fixtures and its tests to maintain for good, and the reader code grows with every fix. It turns "is this layout right?" into "is this fix worth a permanent reader?", which keeps known mistakes in the format.

### Option B - until production, bump the version, replace the fixture, keep no older reader

- **For:** each layout fix costs only the fix. The version bump still makes an old log fail with `UnsupportedVersion`, never with a misread. `FORMAT-GOLDEN` still catches an accidental change, because the fixture changes only on purpose, in the same pull request that bumps the version. The layout that production freezes is the one the evidence supports.
- **Against:** a log written by one pre-production build may not open in a later one. Anyone who keeps test data across builds loses it.

### Option C - withdraw the freeze and make the format provisional again

- **For:** the same freedom as B.
- **Against:** it reopens M5 without a reason: the M5 layout and its evidence stand, and only the rule for later changes is wrong. "Provisional" also loses the discipline that B keeps: a version bump and a reviewed fixture for every change.

### Option D - a migration tool in place of readers

- **For:** one tool can carry data across many versions.
- **Against:** there is no data to carry before production. Its design depends on the production compatibility promise, which does not exist yet.

## Decision

We will use option B. Until the first production release:

1. A change to the bytes of a segment file increments `format_version` in the same pull request. Several changes in one pull request share one increment.
2. A build reads only its own version. It rejects every other version with `UnsupportedVersion`, as it does today.
3. The change replaces the `FORMAT-GOLDEN` fixture: it deletes `format-vN/`, writes `format-vN+1/` with the new code, and states in the pull request what changed in the bytes.
4. No log holds data that must outlive a format change. This is ADR 0013's rule, extended to production.
5. Every other part of ADR 0052 stands: the field choices, the scope (segment files and `LOCK`, not the sparse index), and the rule that a new `kind`, a nonzero `flags` value or a used reserved field is a format change.

The first production release is the first release that the owner declares production in the PRD. Before it, an ADR states the production compatibility promise: which versions a build reads, and whether a migration exists. Until then, ADR 0052's rule "a build reads every frozen version from 1 up to its own" does not apply.

B gives the correct layout at production for the cost of each fix alone, and keeps the two checks that matter: an old log fails clearly, and no change to the bytes goes unnoticed. We accept that pre-production logs do not survive a format change.

## Consequences

- ADR 0052 gains a bullet that links here. ADR 0013's bullet about data outliving later builds is corrected.
- format.md's status says: version 1 as of M5; until production, a change increments the version and keeps no older reader.
- The PRD records the policy beside M5 and in the risk table. M5 stays passed: its gate needed an Accepted format ADR and a fuzz run, and both stand.
- The fixture README and testing.md's `FORMAT-GOLDEN` case say how to replace the fixture. The scenario's failure message names this ADR.
- The M5 follow-ups (footer cross-check, alignment measurement, reserved-field review) can change the layout in one version bump.
- Before production, someone must write the compatibility ADR. Without it, the first production release has no stated promise.

## Enforcement

- `FORMAT-GOLDEN` fails when the encoder changes the bytes. The fix is either to revert the change or to follow rule 3.
- The existing recovery fixtures reject a header whose `format_version` differs from the build's with `UnsupportedVersion`.
- Nothing yet enforces rule 1 when a byte change and a fixture change land without a version bump. Review of the fixture diff catches it.

## Revisit if

- Someone needs to keep data across builds before production. Then freeze for real, with readers or a migration.
- The owner declares a production release. Then the compatibility ADR replaces this one.
