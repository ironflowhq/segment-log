---
id: "0011"
title: "Sealed-segment footer, caches that the log can rebuild, and no clean-shutdown marker"
status: proposed
date: 2026-11-07
deciders: []
supersedes: null
superseded_by: null
tags: [format, recovery]
touches: [docs/architecture/format.md]
enforced_by: none
---

# 0011. Sealed-segment footer, caches that the log can rebuild, and no clean-shutdown marker

## Context

Source: the footer and the rule that caches can be rebuilt are from the reviewed design (decision 2). Using a seal frame instead of a clean-shutdown marker is new in this repository and not reviewed. The design left the marker optional and open.

Open must be fast for a guest that starts and stops often. Without a summary, the last revision of a quiet stream can be anywhere in history, so open must scan every segment. A clean-shutdown marker would let open skip recovery, but deleting it at open is crash-safe only with a directory barrier. Also, the active segment is small, so a scan of it costs little.

## Options & trade-offs

### Option A - a footer in each sealed segment, a seal frame at the end of a sealed log, no marker

- **For:** open reads footers and scans only the active segment. One rule for every open: always recover the active segment. The seal frame shows that a log is finished.
- **Against:** the footer is written at each roll. A footer for many streams makes the roll and the seal slower.

### Option B - a clean-shutdown marker file

- **For:** open can skip the scan after a clean exit.
- **Against:** one more file and one more directory barrier. A second code path for open that the crash tests must also cover.

### Option C - a separate checkpoint file for the tail map

- **For:** one file to read at open.
- **Against:** a second copy of facts that the segments already hold. It must stay consistent with them.

## Decision

We will write a footer in each sealed segment and a seal frame at the end of a sealed log, with no clean-shutdown marker (option A). We accept that a footer for many streams makes the roll and the seal slower.

- A footer is the last frame of a sealed segment. It lists each stream in that segment: stream id, last revision, last file position. It does not copy idempotency keys.
- The tail map, the key map and the sparse index are caches. The log can rebuild each one from the segments. A fact with no durable home is not state.
- `seal` writes the footer and then a seal frame, and makes them durable. A log that ends with a seal frame opens read-only.
- There is no clean-shutdown marker.
- Footers are M4. Before M4, open scans every segment.

The layout is in [format.md](../architecture/format.md#footer-entry-m4).

## Consequences

- [ADR 0037](0037-retain-complete-logs-through-current-milestones.md) replaces the footer-coverage deletion proposal with complete-history retention through M1–M6. Deployment owns whole-log archival and expiry; existing recovery cleanup is unchanged.
- [ADR 0017](0017-footer-locator-offset-and-size.md) amends the footer's contents and position.
- [ADR 0031](0031-keys-live-with-retained-segments.md) preserves segment-lifetime keys. Optimized key lookup and its key-cache memory budget must be settled for M4.
- The former footer-coverage deletion rule is not part of the current milestones under ADR 0037.

## Enforcement

None yet. Oracle check 12 in [testing.md](../architecture/testing.md#the-oracle) will check that every cache can be rebuilt. From M4, oracle check 13 will check that open reads only the summary of each sealed segment.

## Revisit if

- Open time with footers misses the target. Then a checkpoint of footers (option C) may be needed.
- Footer size at roll makes the seal-to-exit target fail.
