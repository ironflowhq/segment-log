---
id: "0047"
title: "The writer epoch counts every segment, not only the newest"
status: accepted
date: 2026-11-10
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [concurrency, recovery]
touches: [segment-log-core/src/engine/recovery.rs, docs/architecture/overview.md]
enforced_by: test
---

# 0047. The writer epoch counts every segment, not only the newest

## Context

[ADR 0018](0018-writer-epoch-from-the-segments.md) sets the writer epoch to one more than the highest epoch in the newest segment's header and its kept frames. It was written when a log had one segment.

From M3, a log has several segments. The [rules for readers](../architecture/format.md#rules-for-readers) say that a frame's epoch is not lower than the epoch of the frame before it, and recovery applies that rule across files. A roll writes the writer epoch into the new header, and that epoch is above every epoch in the log, so after a normal roll the two rules agree.

They disagree when the newest header carries an epoch below a frame in an older segment, and the newest segment keeps no frame with a higher epoch. A log in this state can come from damage or from files placed by hand. Under the ADR 0018 rule, the writer then takes an epoch below that older frame. Its first frame is lower than the frame before it, so the next recovery cuts it, and an acknowledged record is lost (I3).

## Options & trade-offs

### Option A - keep the newest segment only

- **For:** the ADR 0018 text stays as written. Open needs the newest segment only for the epoch.
- **Against:** in the state above, the next recovery cuts frames that the writer acknowledged.

### Option B - the highest epoch in any header or kept frame

- **For:** the writer epoch is above every frame that recovery compares with, so its frames stay valid. M3 already scans every segment, so this costs no read. It gives the ADR 0018 result whenever the files came from rolls.
- **Against:** M4 must keep each sealed segment's highest epoch within the summary that open reads. Its header and its last frame hold it, because frame epochs never decrease.

### Option C - fail open when a header epoch is below an earlier frame

- **For:** it reports the damage instead of working around it.
- **Against:** it refuses a log whose records are all valid, and the format does not call that header invalid.

## Decision

We will set the writer epoch to one more than the highest epoch in any segment header or kept frame (option B). It is the only option that keeps the acknowledged records in every state that recovery accepts. A new log still starts at epoch 1. We accept that the M4 summary must carry each sealed segment's highest epoch.

## Consequences

- [overview.md](../architecture/overview.md#open-and-recovery) step 9 states this rule.
- With one segment, the result equals ADR 0018's.
- M4's footer-based open must read the highest epoch of each sealed segment from the data it already reads.

## Enforcement

The scenario `a_lower_epoch_than_the_segment_before_ends_the_log` in `segment-log-core/src/acceptance/roll/layouts.rs` appends after such an open and checks that the record survives a reopen and a power loss. Under option A, the reopen loses it.

## Revisit if

- M4 chooses a summary that does not hold each sealed segment's highest epoch.
- The format starts to reject a header whose epoch is below an earlier frame. Then option C costs nothing.
