---
id: "0071"
title: "A clean drop writes a close frame, so open no longer truncates a synced last group as a torn tail"
status: proposed
date: 2026-11-20
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [format, recovery, corruption, durability, release-0.2]
touches: [segment-log-core/src/codec/**, segment-log-core/src/engine/writer.rs, segment-log-core/src/engine/recovery.rs, segment-log-core/src/engine/survey.rs, segment-log-core/src/engine/repair.rs, segment-log-core/tests/fixtures/**, docs/architecture/format.md, docs/architecture/overview.md, docs/architecture/api.md, docs/architecture/testing.md, CHANGELOG.md]
enforced_by: test
---

# 0071. A clean drop writes a close frame, so open no longer truncates a synced last group as a torn tail

## Context

[Issue #183](https://github.com/ironflowx/segment-log/issues/183), found while adopting 0.1.1 in ironflowx/foundation. [ADR 0068](0068-open-refuses-damage-with-acknowledged-groups-after-it.md) makes open refuse damage in an unsealed segment when a later group frame proves that the damaged one was acknowledged. The last group frame of the newest unsealed segment has no later frame. If its bytes are damaged after its `fsync` returned, open truncates it as a torn tail and returns `Ok` with a lower watermark. Its offset is assigned again (I6), and its idempotency key is gone, so a retry runs twice. This happens even after a clean drop: in 0.1.1, three acknowledged groups, a clean drop and one flipped byte in the last frame reopen at watermark 2.

These facts are fixed:

- One barrier cannot record its own completion ([ADR 0003](0003-recovery-keeps-the-checksum-valid-prefix.md)). From the last frame's bytes alone, damage after `fsync` and a torn unsynced write look the same. ADR 0003 rejected a second barrier per group because it doubles the `fsync` cost.
- [ADR 0011](0011-sealed-footer-and-no-shutdown-marker.md) chose no clean-shutdown marker. Its objections were to a marker file: one more file, a directory barrier to delete it at open, and a second open path whose only gain was skipping the scan.
- The writer has at most one unsynced group frame, the segment's last ([write path](../architecture/overview.md#the-write-path), steps 4–5).
- A clean drop already waits for the writer: when the queue disconnects, the writer runs `write_pending(&view, true)`, which joins the sealer and makes any pending closing run durable ([ADR 0061](0061-a-sealer-thread-builds-the-key-frames-off-the-writer.md), rule 7), and the log joins the writer before it releases the lock. So after a clean drop the second-newest segment is always sealed, and damage to its last group is class B, which reads catch with the CRC.
- Frame CRCs cover the log id ([ADR 0049](0049-frame-crcs-cover-the-log-id.md)), so a caller's payload cannot place a valid frame.
- [ADR 0064](0064-the-0-1-compatibility-promise.md) is accepted, and 0.1.0 and 0.1.1 are released and in use. A new frame kind is a new format version (format.md, Status). An older build treats an unknown kind as a bad frame. A writer never appends to an older-version segment (rule 12), and an upgrade is one-way (rule 14).
- No version-dispatch point exists yet; ADR 0066 deferred it to the first real version 4.
- ADR 0068's repair plan writes a seal run that must end at or before the start of the last proof frame, so that every crash point fails closed.

## Options & trade-offs

### The witness

#### Option A - a close frame written at a clean drop

After the writer drains and its last `fsync` returned, it appends one frame that says so, and `fsync`s it.

- **For:** nothing on the append path; one `fsync` per clean drop. No new file and no directory barrier. Open keeps one path: the scan always runs, and the frame stays in place for the next session's groups to follow.
- **Against:** a new frame kind, so a format version. It covers only a clean drop: after a crash or power loss, a damaged last group before the next group or clean drop is still truncated.

#### Option B - a marker file (ADR 0011 option B)

- **For:** no format change.
- **Against:** ADR 0011's objections stand: one more file, a directory barrier to delete it at open, and a second open path. It covers no more than A.

#### Option C - a second barrier per group, opt-in through `Options`

- **For:** covers the crash window too.
- **Against:** twice the `fsync` per group, which this design exists to spread. An option that most deployments leave off protects them from nothing.

#### Option D - document the limit

- **For:** no work.
- **Against:** a clean drop and a later bit flip lose an acknowledged record silently, and a retry runs twice.

### The format

#### Option E - format version 4, under ADR 0064 rule 12

- **For:** the promise holds: a 0.1 build refuses a log with a version 4 segment with `UnsupportedVersion`, and never misreads it.
- **Against:** the first real version 4 brings the version dispatch, a `format-v4` fixture and real `FORMAT-MIXED` runs. An existing log gains the protection only from its first version 4 segment, and the upgrade is one-way.

#### Option F - amend version 3

- **Against:** a 0.1 build reads kind 7 as a bad frame. After a newer build appends groups behind a close frame, that build sees a bad frame with a group proof after it and refuses the log with `Corrupt` under ADR 0068. Rolling back would break the log, which ADR 0064 forbids.

### Repair when the close frame is the only proof

#### Option G - keep ADR 0068's rule

- **Against:** the close frame sits right after the damaged group, so the room for the seal run is only the damaged frame's bytes. A seal run (footer, seal frame, locator) rarely fits, and the plan nearly always refuses with exit code 3, which leaves the case manual.

#### Option H - overwrite the close frame last

- **Against:** a crash while the run overwrites it leaves no proof. Open then truncates at the cut and returns a writable log that assigns the dropped offsets again.

#### Option I - copy the close frame past the seal run first

- **For:** the proof search already tries every byte position, so a copy past the run is a proof at every crash point. A rerun finds the copy as the last proof, with room, and converges with no special case.
- **Against:** one more storage step, with its own crash cases.

## Decision

We will build options A, E and I.

**Format version 4.** Version 4 adds one frame kind, `7`, the close frame: a body header with `count = 0` and nothing after it, the same shape as the seal frame. Its `epoch` is the writer's epoch, and its `base_offset` is the next unused log offset, as for every kind that is not a group; it uses no log offset. Every other byte matches version 3. A version 3 segment never holds a close frame, and a reader rejects kind 7 in one. The build writes version 4, so the first open by it closes a version 3 active segment and rolls, or rewrites a header-only one, as rule 12 says. This is the first real version 4, so it adds the version dispatch that [ADR 0066](0066-append-time-is-advisory-and-versions-never-go-back.md) deferred.

**Writing it.** When the queue disconnects, the writer runs `write_pending(&view, true)` as today. Only if that returns `Ok`, and the writer is not poisoned, does it append a close frame to the newest segment and call `sync_data`, the same barrier as a group. It skips the frame when the newest segment already ends with a close frame, so that a guest that starts and stops without appending pays no `fsync`. It writes the frame even when the newest segment holds only its header, so drop has one path. If the write or the `fsync` fails, the writer stops: it does not retry ([ADR 0004](0004-io-errors-poison-the-writer.md)), and drop returns nothing. The next open then finds no close frame and behaves as today. `seal` writes no close frame: a sealed log ends in a seal run, and its last segment is never scanned at open.

**Reading it.** The close frame holds no records. Open's scan, `LogReader`, the sealer's closing-run build, the sparse frame tables ([ADR 0069](0069-reads-keep-sparse-frame-tables-under-a-byte-budget.md)) and the repair tool step over it. The repair tool lists it as `close`. The next session's groups follow it, so a segment can hold many close frames. Every rule that says a segment "holds only its header", or "only its header and group frames", reads instead "holds no group frame", or "only group and close frames".

**Proof.** ADR 0068's proof gains one case. After a scan of an unsealed segment stops at `p` while it expects offset `n` next, a close frame after `p` that passes every per-frame rule for readers and has `base_offset > n` is a proof: open fails with `OpenError::Corrupt { segment, position: p }` and changes no file. A close frame is written only after every group before it was synced, so if it is valid, those groups were acknowledged. A close frame with `base_offset <= n` is from an earlier session or a lookalike copy, and proves nothing. The search's cheap filter accepts kind 7 beside kind 1. A torn close frame is a torn tail: open truncates it and keeps every group before it. The same rule serves the newest segment, the second-newest segment without its closing run, and `LogReader`.

**Repair.** A class F plan whose seal run would pass the start of the last proof, when that proof is a close frame, first moves the proof. `--apply` appends to the segment zero bytes up to the end of the seal run, if the file ends before it, and then a copy of the close frame, and calls `sync_all`. Then it writes the seal run at `p`, calls `sync_all`, truncates the file to the end of the run and calls `sync_all` again, as ADR 0068 does. Every crash point fails closed: before the first `sync_all` the original close frame is intact; after it, the copy lies past the run and survives the run's write. Zero bytes have a `body_len` out of range, so the search skips them. A rerun finds the copy as the last proof, with room, and writes the same run. `dropped_offsets` ends at the close frame's `base_offset`. A last proof that is a group frame keeps ADR 0068's no-room refusal.

**Release.** Ships in 0.2.0, not a 0.1.x patch: the format bump makes the upgrade one-way, and Cargo takes patch releases without asking.

We chose the close frame because it closes the clean-drop case at the cost of one `fsync` per drop and nothing per group, and keeps ADR 0011's single open path. We accept that a crash or power loss, followed by damage to the last group before the next group or clean drop, still truncates that group.

## Consequences

- Amends ADR 0003: after a clean drop, a damaged last group is no longer kept as a torn tail; open fails with `Corrupt`.
- Amends ADR 0011: a close frame now records a clean drop. It is a frame, not a marker file, and open never skips the scan because of it.
- Amends ADR 0065 and ADR 0068: a close frame is a proof, and a class F plan may move a close-frame proof before it writes the seal run.
- format.md gains format version 4 and the close frame. overview.md gains the close frame in drop, open steps 5–7 and the repair order. api.md's `Drop` says that a clean drop writes a close frame. testing.md gains the scenarios below as `Planned`.
- A clean drop costs one more `sync_data` when the session wrote a group or the segment does not yet end with a close frame.
- The writer keeps one more fact: whether the newest segment ends with a close frame. Open sets it from the scan.
- The server's `SIGTERM` path already drops the log, so a guest stopped by `SIGTERM` gets a close frame. A guest stopped by `SIGKILL` or a crash does not.
- A 0.1 build refuses a log that a 0.2 build has opened for writing, with `UnsupportedVersion(4)`. [CHANGELOG.md](../../CHANGELOG.md) states the format bump and the one-way upgrade. The unreleased section becomes 0.2.0. No API change and no protocol version change.
- The repair JSON keeps `schema` 1 and gains no field. The printed plan names the moved proof.

## Enforcement

Scenarios in [testing.md](../architecture/testing.md), on simulated storage in `segment-log-core/src/acceptance/`:

- `OPEN-DAMAGE-LAST-GROUP-AFTER-CLOSE`: the issue's reproduction. After a clean drop, one flipped byte in the last group frame makes open fail with `Corrupt` at that frame and change no file. `LogReader` does the same.
- `OPEN-TORN-LAST-GROUP-NO-CLOSE`: a crash, then a torn last group. Open truncates, as today.
- `OPEN-TORN-CLOSE-FRAME`: a torn close frame. Open truncates only the close frame and keeps every group.
- `OPEN-OLD-CLOSE-FRAME`: damage in the last group after an earlier session's close frame, with nothing after it. Open truncates.
- `DROP-CLOSE-FRAME`: a clean drop writes one close frame; a second open and drop with no append writes none; a poisoned writer, a failed pending run and a failed close-frame `fsync` write none and retry nothing; a crash at every storage call of the drop under `Kill`, `PowerLoss` and `PowerLossTorn` loses no acknowledged record.
- `REPAIR-CLASS-F-CLOSE-FITS`, `REPAIR-CLASS-F-CLOSE-MOVED` and `REPAIR-CLASS-F-CLOSE-CRASH`: class F with a close frame as the only proof, with and without room before it, and a crash at every storage call of `--apply`. Open returns `Corrupt` or the sealed repaired log, never a writable one, and a rerun converges.
- `OPEN-DAMAGE-RANDOM` extended: after a clean drop, flip one byte anywhere in the newest segment's last group frame. Open never returns `Ok` with a lower watermark.
- `FORMAT-GOLDEN` gains `format-v4/`, whose history includes a clean drop and appends after its close frame. `FORMAT-MIXED` runs on real version 4 bytes and drops its test hook. A version 3 segment with a kind 7 frame fails as a bad frame.

## Revisit if

- A deployment needs the crash window covered. Then reconsider option C as an opt-in.
- Close frames from many short sessions show up in segment size or scan time.
- A group-frame proof with no room keeps refusing in practice. The move of option I works for a group frame too, and would retire ADR 0068's no-room refusal.
