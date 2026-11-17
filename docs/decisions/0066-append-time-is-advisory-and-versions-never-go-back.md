---
id: "0066"
title: "The append time is advisory, and a log never goes back a format version"
status: accepted
date: 2026-11-17
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [format, compatibility, recovery, release-0.1]
touches: [docs/architecture/format.md, docs/architecture/testing.md, segment-log-core/src/engine/survey.rs, segment-log-core/src/acceptance/mixed_versions.rs]
enforced_by: test
---

# 0066. The append time is advisory, and a log never goes back a format version

## Context

[ADR 0064](0064-the-0-1-compatibility-promise.md) is accepted. Its implementation (format version 3, and the mixed-version open of rule 12) left four questions open, which the owner settled on 2026-11-17 ([issue 133](https://github.com/ironflowx/segment-log/issues/133)):

- **Rule 2 bounds the append time from below only.** The writer stamps the larger of its clock and the previous group's time. Nothing bounds it from above, so one clock reading far in the future holds every later group at that time.
- **Readers do not check the order of append times** (format.md, group body). Rule 2 makes the writer keep it, but no ADR says why readers trust it.
- **Open accepts any order of segment versions.** Survey checks each header against the range from 3 up to the build's own, one segment at a time. Under rule 12 a writer only moves forward: it closes an older active segment and rolls into its own version. So a newer-version segment followed by an older one comes only from hand-copied files or a damaged log, and open accepts it today.
- **The code has no version-dispatch point.** Version 4 exists only as the test-only bump of the `FORMAT-MIXED` scenarios, which writes version 3's bytes.

## Options & trade-offs

### Upper bound on the append time

#### Option A - no upper bound; the time is advisory

- **For:** no new failure mode. The writer never refuses or rewrites a group because of its clock.
- **Against:** a bad future reading becomes the floor for all later groups of the log. A time-based seek cannot treat the time as exact.

#### Option B - clamp or reject a time too far ahead of the previous group

- **For:** limits the damage of one bad reading.
- **Against:** needs a threshold with no measurement behind it, and a reject makes the clock a reason for an append to fail.

### Reader check of the order

#### Option C - readers trust the writer

- **For:** one place keeps the order. A frame CRC already catches a damaged time.
- **Against:** a writer bug that breaks the order goes unnoticed by readers.

#### Option D - readers reject a decreasing time

- **For:** catches such a writer bug.
- **Against:** a new reason to fail open or a read, for a field that is advisory anyway (option A).

### A newer-version segment followed by an older one

#### Option E - open fails with `Corrupt` at the older segment's header

- **For:** it is a bad header in the sense open already uses: a header field that disagrees with the log around it, as a `base_offset` that differs from the file name does, which also fails with `Corrupt` at position 0. The repair tool already classes a bad header as class E, diagnose only ([ADR 0065](0065-the-repair-tool-truncates-only-the-newest-segment.md)), so `verify` reports it with no new class. No new public error variant.
- **Against:** the error does not name the cause; the operator reads it from the headers or from `verify`.

#### Option F - a new `OpenError` variant

- **For:** names the cause.
- **Against:** a public API addition for a case no writer produces, and a new class for the repair tool.

#### Option G - accept it

- **Against:** a writer would then append to, or rewrite the header of, an older segment that rule 12 says it never touches, and the log's history would no longer show the order of its builds.

### Version dispatch

#### Option H - add it when version 4 exists

- **For:** no code for a version that has no bytes yet.
- **Against:** none known; the test-only bump already proves the open path.

#### Option I - add a dispatch layer now

- **Against:** speculative code with one branch.

## Decision

We will use options A, C, E and H.

1. **The append time is advisory.** It never decreases within a log (ADR 0064, rule 2) and has no upper bound. A clock reading in the future becomes the floor for every later group of the log. It is not a source of truth: a later feature that seeks by time treats it as a hint.
2. **Readers do not reject a decreasing append time.** The writer keeps the order, a reader check would add a failure mode for an advisory field, and a frame CRC already catches a damaged time.
3. **A segment's version is never lower than the version of the segment before it**, in base offset order. Open, and `LogReader::open`, fail with `OpenError::Corrupt { segment, position: 0 }` at the first segment that breaks this, before any file changes. format.md's rules for readers state the rule.
4. **No version-dispatch point yet.** When a real version 4 exists, each segment's decoder, and the encoder of its closing run (roll, recovery and seal), are chosen by that segment's own version (ADR 0064, rule 12).

A, C and H keep 0.1 free of rules that no measurement or real version asks for. E reuses the error and the repair class that a bad header already has, so the check costs one comparison per segment and no API.

## Consequences

- This ADR amends ADR 0064 without replacing it; ADR 0064's Consequences link it.
- A host with a bad clock can leave every later append time of a log stuck at a future value. Retention or seek by time must cope with that.
- A log copied together by hand from two builds can fail open with `Corrupt` at a header whose bytes are all valid on their own. `verify` reports it as class E.
- Version 4 work must add the dispatch by segment version, and the `FORMAT-MIXED` scenarios then run on real version 4 bytes.

## Enforcement

`an_older_version_after_a_newer_one_fails_open` in `segment-log-core/src/acceptance/mixed_versions.rs` (`FORMAT-MIXED` in [testing.md](../architecture/testing.md)). The crash scenario of the same file opens the log at every storage call of the version change, so it fails if a writer ever leaves an older segment after a newer one.

## Revisit if

- A feature needs an exact append time, such as retention by time with a legal bound.
- A real writer bug breaks the order of append times.
- Version 4 lands; then the dispatch of decision 4 is built.
