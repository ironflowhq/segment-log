---
id: "0052"
title: "Freeze the byte layout at format version 1, with no byte change"
status: accepted
date: 2026-11-13
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [format]
touches: [docs/architecture/format.md, segment-log-core/src/codec/**, segment-log-core/src/engine/survey.rs, segment-log-core/src/engine/recovery.rs]
enforced_by: test
---

# 0052. Freeze the byte layout at format version 1, with no byte change

## Context

[ADR 0013](0013-provisional-v0-byte-layout.md) made the byte layout provisional until M5. Until then, any change increments `format_version` and needs no reader for older files. At M5, a new ADR freezes the layout. After that, a change needs a new version and a reader for the older one. The M5 gate needs that ADR Accepted ([PRD](../prd.md#milestones)).

ADR 0013 lists four things for M5 to review: the width of the log id, alignment, which fields are inside the CRC, and the frame size limit.

These facts are fixed:

- M4 writes `format_version` 1 ([format.md](../architecture/format.md#segment-header-64-bytes)). It holds every structure the code writes: the header, group frames, and the closing frames from [ADR 0048](0048-sealed-segments-carry-a-key-filter-and-index.md) and [ADR 0017](0017-footer-locator-offset-and-size.md). M1–M3 wrote version 0, which M4 already cannot open.
- Every frame's CRC is CRC-32C over the log id, `body_len` and the body ([ADR 0049](0049-frame-crcs-cover-the-log-id.md)). The header has its own CRC. Each key index page has its own CRC.
- The log id is 128 random bits. It is also the 128-bit SipHash-2-4 key for the key hash, so the key frames depend on its width.
- The frame body limit is 16 MiB and the group body limit is 8 MiB. The group limit keeps a group's footer entries inside one footer frame. The 4,000 MiB roll limit keeps every file position inside the 4-byte fields of the key index.
- Reserved fields and `flags` must be zero, and a reader rejects a nonzero one. A reader rejects any `format_version` other than its own (`segment-log-core/src/engine/survey.rs`).
- format.md also describes a sparse index file (`.idx`). No code writes or reads it. It has no header, no version and no CRC.
- No golden file pins the layout. The codec check values pin single frames, and nothing fails if the whole-file layout changes without a version bump (ADR 0013, Enforcement).
- [ADR 0051](0051-a-log-key-filter-in-memory-in-front-of-the-segment-filters.md) chose a log key filter kept only in memory, so that M5 does not wait for a format change.

## Options & trade-offs

### Option A - freeze version 1 as it is

- **For:** M4 logs stay readable. The layout already has every structure through M6: the server adds a wire protocol, not a file change ([ADR 0001](0001-rust-library-first-socket-boundary-later.md)). The four review points have answers that keep the current bytes (see Decision). The crash tests, the oracle and the M4 evidence all ran against these bytes.
- **Against:** any later change needs a reader for version 1 kept for good. A choice found wrong after M6 costs more than one found wrong now.

### Option B - align key index pages to 4 KiB, then freeze version 2

- **For:** a key index page sits at an arbitrary file position, so a 4,096-byte page read crosses two OS pages. Padding before the key index frame would make one page read touch one OS page, which can halve the device reads of a cold lookup.
- **Against:** a format change and up to 4 KiB of padding per sealed segment, and "frames follow with no gap" gains an exception that recovery and the fuzz targets must handle. Only a false hit or a real retry reads a page, and ADR 0051's log filter cuts false hits. No measurement shows the cost of the second OS page.

### Option C - a 64-bit frame checksum, then freeze version 2

- **For:** a random corruption passes CRC-32C with probability 2^-32. A 64-bit check lowers that.
- **Against:** a new envelope width, a second CRC table in a `std`-only crate ([ADR 0012](0012-no-log-crate-dependency.md)), and a slower check on every frame write and read. A damaged frame must also pass the `kind`, reserved, `base_offset`, epoch and exact-fill checks. A frame also has no reason to fail other than a crash or the device, and the deployment qualifies the device ([ADR 0040](0040-volumes-meet-testable-minimum-requirements.md)).

### Option D - wait to freeze until M6 has run

- **For:** the guest benchmark could still change the layout.
- **Against:** the milestone order puts the freeze in M5, before the fuzzing that tests the frozen layout. The M6 server changes no file structure. Data written before the freeze must not outlive it (ADR 0013), so every month without a freeze is a month in which no one can keep data in the log.

## Decision

We will freeze the byte layout at `format_version` 1, exactly as format.md describes it, with no byte change (option A). The four points from ADR 0013:

1. **Log id width.** It stays 16 bytes. It is the SipHash key, which is 128 bits. 128 random bits make a collision between two logs' ids negligible, and a mixed-up log fails the header check.
2. **Alignment.** There is none, and frames follow with no gap. Records are sliced from their frame and copied out, so no reader needs aligned fields. Key index pages stay unaligned (option B rejected).
3. **Fields inside the CRC.** They stay as they are: the header CRC covers header bytes 0..60, each frame CRC covers the log id, `body_len` and the body ([ADR 0049](0049-frame-crcs-cover-the-log-id.md)), and each key index page CRC covers page bytes 0..4092. The check stays CRC-32C (option C rejected).
4. **Frame size limit.** It stays a 16 MiB frame body and an 8 MiB group body.

The frozen layout covers the segment files (`*.seg`) and the `LOCK` file. It does not cover the sparse index. format.md marks `.idx` as not frozen. A later sparse index, or any other cache file, gets its own decision and its own version header, and needs no segment version bump, because open ignores files that it does not know.

After the freeze:

- Any change to the bytes of a segment file increments `format_version`. A build reads every frozen version from 1 up to its own.
- Each segment declares its own version, and a reader checks each segment against the rules of that version. Whether a writer may append to an active segment of an older version, or must roll first, is for the ADR that introduces version 2.
- Version 0 is never frozen. No build reads it.
- A new `kind`, a nonzero `flags` value or a used reserved field is a format change. A version 1 reader rejects all three, so none can be added without a version bump.

A changes nothing that has been tested, and B and C fix costs that no measurement shows. We accept that a mistake found after M6 needs a reader for version 1 kept for good.

## Consequences

- [ADR 0055](0055-format-version-2-aligns-key-index-pages.md), proposed, makes format version 2: key index pages start on a 4 KiB boundary (this ADR's option B, after a measurement), and format.md drops the sparse index.
- [ADR 0053](0053-format-changes-need-no-older-reader-before-production.md), proposed, replaces the rule for later changes until the first production release: a change increments `format_version` and replaces the `FORMAT-GOLDEN` fixture, and no build keeps a reader for older versions. The field choices and the scope here stand.
- format.md drops the "provisional" status, links this ADR, and marks the sparse index as not frozen. The `log_id` note "M5 reviews it" goes.
- [ADR 0013](0013-provisional-v0-byte-layout.md) is fulfilled and stays accepted: it planned this ADR. It gains a bullet that links here.
- Data written from now on can outlive later builds, so the "no data that must outlive a format change" rule in ADR 0013 ends with the acceptance on 2026-11-13.
- The M5 fuzz targets test version 1 and need not change for the freeze.
- Every later format change costs a reader for each older version, plus fixtures for them.

## Enforcement

The M5 tests must add `FORMAT-GOLDEN`, and testing.md names the case:

- A small log committed as files under `segment-log-core/tests/fixtures/format-v1/`: two rolled segments with keyed and unkeyed records and an active segment, and the same log after `seal`. A test opens the first with a writable log, because a sealed log never checks a key, and checks every record, revision and key retry against expected values in the test. A test opens the sealed copy with `LogReader` and checks every record and revision.
- A test writes the same records with the fixture's log id and epoch, then seals, and checks that the bytes equal both copies byte for byte.

A change to the encoder fails the second check. Whoever changes the layout must then bump `format_version` and keep the first check passing through a version 1 reader.

## Revisit if

- A measurement shows that cold key index reads miss a target because of the second OS page (option B).
- The qualified deployment stack shows a corruption rate at which a 32-bit check no longer makes a missed corruption negligible (option C).
- A record needs a payload larger than the group body limit allows.
- A roll size above 4,000 MiB is needed, which needs wider positions in the key index.
