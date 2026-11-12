---
id: "0017"
title: "Footer locator, last log offset, and footer size"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [format, recovery]
touches: [docs/architecture/format.md]
enforced_by: test
---

# 0017. Footer locator, last log offset, and footer size

## Context

Source: new in this repository, from a review of the docs on 7 November 2026. Not reviewed by the source design.

[ADR 0011](0011-sealed-footer-and-no-shutdown-marker.md) puts a footer at the end of each sealed segment, with stream id, last revision and last file position for each stream. Open reads the footers and scans only the active segment. That design has three gaps:

- **Open cannot find a footer.** A frame starts with its length, so frames can be read only from the front. Nothing at a fixed position says where the footer starts. Open would have to read every frame header of every sealed segment. With small groups, that is about 100,000 reads for each 64 MiB segment (an estimate), which misses the 50 ms open target.
- **The footer has no log offset.** `stream_head` returns a stream's last log offset. To get it from the footer, open would have to read the entries of the frame at `last_position`. The M4 gate forbids reading sealed payload bytes at open.
- **A footer can pass the frame limit.** A footer entry is 28 bytes plus the stream id. A 64 MiB segment of 256-byte records holds about 200,000 records. With a distinct stream for each record and 100-byte stream ids, the footer is about 24 MiB, which is over the 16 MiB frame limit.

## Options & trade-offs

### Finding the footer

#### Option A - a fixed-size locator frame at the end of each sealed segment

- **For:** the same envelope, CRC and scan rules as every other frame. Open reads a fixed number of bytes from the end of the file.
- **Against:** one more frame kind.

#### Option B - a raw trailer that is not a frame

- **For:** a few bytes smaller.
- **Against:** a forward scan sees the trailer as a bad frame, so the reader rules need a special case.

#### Option C - a separate file that holds each footer's position

- **For:** the segment format does not change.
- **Against:** a second copy of a fact that must stay in step with the segment (I9).

### Keeping the footer inside one frame

#### Option A - roll early, and cap a group at half the frame limit

- **For:** the writer knows the footer size before it writes a group. Half the frame limit is enough: a footer entry is at most 12 bytes longer than the group entry it summarizes, and the smallest group entry is 17 bytes, so a group's footer entries are less than twice its size.
- **Against:** a segment with many streams can be smaller than the roll size.

#### Option B - a footer that spans several frames

- **For:** no early roll.
- **Against:** more reader states, and the locator must point to a list.

## Decision

We will end each sealed segment with a fixed-size locator frame (option A for finding the footer), and roll early so that the footer fits in one frame (option A for its size). We accept one more frame kind, and segments with many streams that are smaller than the roll size.

- A locator frame (kind `4`) is the last frame of every sealed segment from M4. It follows the footer, and in the final segment of a sealed log it follows the seal frame. It holds the footer's file position.
- A footer entry also holds `last_offset`, the stream's last log offset in the segment.
- The writer rolls before a group whose new streams would make the footer pass the frame limit. The byte cap is at most half the frame limit.

This extends the footer contents in ADR 0011. The layout and the limits are in [format.md](../architecture/format.md).

## Consequences

- [ADR 0031](0031-keys-live-with-retained-segments.md) preserves key lifetime across roll and reopen. M4 must choose key lookup and its memory budget before summary-based open skips retained record scans; footer contents alone do not solve it.
- A sealed segment from M4 ends with the footer, then the seal frame if it is the final segment of a sealed log, then the locator. A roll writes two frames before one `sync_data`, and a seal writes three.
- Open reads the locator of each sealed segment except the newest, then its footer. A successor segment proves that a segment's real locator is its last frame. The newest segment has no successor, and a payload can end with bytes that look like a locator, so open scans the newest segment forward ([overview.md](../architecture/overview.md#open-and-recovery)). The M4 gate counts the bytes that open reads through the storage trait.

## Enforcement

The `ORACLE-13` scenario in `segment-log-core/src/acceptance/summary_open.rs` checks the exact reads of open, including the locator and the footer. The `ROLL-FOOTER` scenarios check that the writer rolls before a footer or a key index passes the frame limit.

## Revisit if

- Reading 100 footers at open misses the open target. Then a checkpoint of footers (ADR 0011, option C) may be needed.
