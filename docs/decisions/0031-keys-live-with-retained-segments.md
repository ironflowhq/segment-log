---
id: "0031"
title: "Keys live with retained segments"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, idempotency, recovery, milestones]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/format.md, docs/architecture/testing.md]
enforced_by: none
---

# 0031. Keys live with retained segments

## Context

R13 ties a key's lifetime to the segment holding its record. M1 rebuilds its key map by scanning one segment. M3 can scan all retained segments. M4 introduces footer-based open without scanning old records, but footers contain no keys.

PRD open item 1 offers either another lookup mechanism or a shorter key lifetime. The owner approved preserving segment-lifetime keys while deferring the lookup mechanism and key-cache memory budget to M4 during the M1 readiness discussion on 8 November 2026. This ADR confirms R13 in ADR 0008 and defines that milestone boundary.

## Options & trade-offs

### Option A - preserve lifetime and settle optimized lookup in M4

- **For:** retries continue to find retained records after roll and reopen. M1 can implement scanning without choosing a future index format.
- **Against:** M4 still needs a lookup design and memory policy that preserve the contract without scanning old records at every open.

### Option B - expire keys when a segment stops being active

- **For:** a key map covering only the active segment is smaller and easy to rebuild.
- **Against:** retained records can be appended again after a retry whose key expired on roll. The retry window depends on file management rather than record retention.

### Option C - choose the M4 index and memory budget now

- **For:** settles the future optimization earlier.
- **Against:** fixes an index design before the single-segment implementation and measurements exist. M1 does not need that index.

## Decision

A key remains valid while the segment holding its record is retained, including after roll and reopen. Rolling or sealing a segment does not expire its keys. Deleting its segment expires the key under R13; retention policy remains a separate decision.

M1 scans its single segment to rebuild the key map. Through M3, recovery scans retained segment records and rebuilds their keys. M4 must choose the lookup mechanism and key-cache memory budget before replacing those scans with summary-based open.

The optimized lookup must preserve the same key lifetime and remain rebuildable from segment records under I9. A per-segment key index is a candidate, not an approved format. This decision introduces no index file or new byte layout in M1.

We accept leaving the lookup optimization and memory policy open until M4 to preserve the retry guarantee without making M1 depend on that design.

## Consequences

- [ADR 0037](0037-retain-complete-logs-through-current-milestones.md) keeps complete history through M1–M6 and assigns whole-log archival and expiry to deployment. The key lifetime is unchanged; only a future retention feature could add valid-segment deletion.
- The lifetime part of PRD open item 1 is resolved. Its lookup mechanism and key-cache memory budget remain blockers for M4, not M1.
- Footer summaries alone cannot implement retained-key lookup. M4 must address that gap rather than silently shortening key lifetime.
- The admission byte budget remains separate from key-cache memory. This decision creates no total-resident-memory guarantee for M1.
- M3 can implement roll with full scans. It must not drop old keys merely because their segment is sealed.

## Enforcement

None yet. The retained-key cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will check reopening in M1 and roll plus reopen in M3. M4 must add tests for its chosen lookup mechanism, cache rebuilds and memory budget before skipping retained record scans.

## Revisit if

- The product explicitly chooses a shorter retry lifetime and accepts that retries of retained records can append duplicates after expiry.
