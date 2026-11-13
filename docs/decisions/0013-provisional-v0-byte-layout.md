---
id: "0013"
title: "A provisional v0 byte layout until M5"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [format]
touches: [docs/architecture/format.md]
enforced_by: none
---

# 0013. A provisional v0 byte layout until M5

## Context

Source: the frame shape is from the reviewed design ("File and recovery"): length-prefixed groups, a CRC-32C over the length, the log offset and the writer epoch, and a log id. The field widths, magic and kinds in [format.md](../architecture/format.md) are new in this repository and not reviewed. The design said that the layout should be frozen only after the recovery tests, the key checks, the atomic submit and the footer exist.

M1 cannot be built without some layout. A frozen layout too early would lock in widths before the tests show what is needed.

## Options & trade-offs

### Option A - write a provisional v0 now; freeze it at M5

- **For:** M1 can start. The tests shape the final layout.
- **Against:** files written before M5 may not open after a format change.

### Option B - design the final layout before M1

- **For:** no format break later.
- **Against:** decides widths and fields before the code that uses them exists.

## Decision

We will write a provisional v0 layout now and freeze it at M5 (option A). We accept that files written before M5 may not open after a format change. [format.md](../architecture/format.md) is the v0 layout and the only source for widths and offsets. Until M5, any change is allowed. It increments `format_version`, and old files are not migrated. At M5, a new ADR freezes the layout. After that, a change needs a new version and a reader for the older one.

Unread Kafka 4.4 format pages and unread Kurrent gRPC names do not block a private format.

## Consequences

- [ADR 0052](0052-freeze-format-version-1.md) freezes the layout at version 1, with no byte change. From 2026-11-13, data may outlive later builds, and a change needs a new version and a reader for the older ones.
- [ADR 0049](0049-frame-crcs-cover-the-log-id.md) makes every frame's CRC start with the segment's log id, from `format_version` 1 in M4. The M5 review of the CRC fields starts from that rule.
- Before M5, no data that must outlive a format change goes into this log.
- M5 must review: the width of the log id, alignment, which fields are inside the CRC, and the frame size limit.

## Enforcement

None yet. The codec round-trip tests in [testing.md](../architecture/testing.md#property-and-unit-tests) will pin the layout. Nothing fails when the layout changes without a new `format_version`.

## Revisit if

- A consumer needs to keep data from before M5. Then freeze earlier, or write a migration.
