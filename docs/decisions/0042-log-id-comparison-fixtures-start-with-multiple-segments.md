---
id: "0042"
title: "Log-id comparison fixtures start with multiple segments"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [testing, identity, milestones]
touches: [docs/architecture/testing.md, docs/architecture/api.md]
enforced_by: none
---

# 0042. Log-id comparison fixtures start with multiple segments

## Context

The M1 recovery fixture list included a header with a wrong log id. M1 supports only one segment, whose header supplies the log id. A replacement id with a valid CRC has no independent trusted copy for comparison. The owner accepted moving comparison fixtures to M3, when multiple headers can disagree.

## Options & trade-offs

### Option A - compare identities when multiple segments are supported

- **For:** follows the existing format and milestone boundary.
- **Against:** cannot detect a standalone CRC-valid replacement id in M1.

### Option B - add another durable identity anchor

- **For:** gives M1 another value to compare.
- **Against:** expands the format and durability protocol solely for this fixture.

## Decision

Use option A. Keep the [format](../architecture/format.md#rules-for-readers) unchanged. The [fixture contract](../architecture/testing.md#property-and-unit-tests) and [API milestone availability](../architecture/api.md#milestone-availability) identify when comparisons apply.

## Consequences

- M1 still tests bad header CRCs and filename/base-offset mismatches.
- M3 must test inconsistent log ids across segment headers.
- No new metadata file, identity copy or M1 format field is introduced.

## Enforcement

None yet for cross-segment comparison. M3 extends `RECOVERY-FIXTURES` under [testing.md](../architecture/testing.md).

## Revisit if

- A separate requirement needs detection of a standalone CRC-valid identity replacement.
