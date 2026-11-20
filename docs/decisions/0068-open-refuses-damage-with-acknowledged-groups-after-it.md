---
id: "0068"
title: "Open refuses, and does not truncate, an unsealed segment whose damage has acknowledged groups after it"
status: proposed
date: 2026-11-20
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [recovery, corruption, operator, durability, release-0.1]
touches: [segment-log-core/src/engine/recovery.rs, segment-log-core/src/engine/survey.rs, segment-log-core/src/engine/repair.rs, segment-log-server/src/**, docs/architecture/overview.md, docs/architecture/api.md, docs/architecture/testing.md]
enforced_by: test
---

# 0068. Open refuses, and does not truncate, an unsealed segment whose damage has acknowledged groups after it

## Context

[Issue #178](https://github.com/ironflowx/segment-log/issues/178). Open truncates the newest unsealed segment where its scan stops ([open step 7](../architecture/overview.md#open-and-recovery)). When the bad frame has valid group frames of this log after it, those frames were acknowledged, and open drops them, returns `Ok`, and makes the shorter file durable. The next writer reuses their offsets (I6), and their idempotency keys are gone, so a retry runs twice. In 0.1.0, one flipped byte in the first group of a three-group segment reopens the log at watermark 0.

These facts are fixed:

- [ADR 0003](0003-recovery-keeps-the-checksum-valid-prefix.md) keeps the contiguous CRC-valid prefix, for a torn tail. A torn tail lies only after the last frame whose `fsync` returned.
- The writer writes one group frame and calls `fsync` once before it writes the next ([write path](../architecture/overview.md#the-write-path), steps 4–5). So at most one group frame of a segment is unsynced, and it is the segment's last group frame. The sealer's closing-run writes ([ADR 0067](0067-the-sealer-writes-and-flushes-the-closing-run.md)) go to the second-newest segment, not the newest.
- A closing run is written as two batches, each followed by one `fdatasync`, so a power loss can keep one frame of a batch and drop another. Open step 7 already treats a kept locator and footer with a lost run page as a torn tail.
- A frame has no marker and no alignment: `body_len`, then a CRC over the log id, `body_len` and the body ([format.md](../architecture/format.md)). After a bad frame, its `body_len` cannot be trusted, so the next frame boundary is unknown.
- Frame CRCs cover the log id ([ADR 0049](0049-frame-crcs-cover-the-log-id.md)). A valid frame inside a torn payload is a copy of a frame of this log that existed when the payload was written, so its `base_offset` is lower than the torn group's own.
- [ADR 0065](0065-the-repair-tool-truncates-only-the-newest-segment.md)'s lossy check reads the footer of a valid closing run. An unsealed segment has none, so the repair tool classes this case as a torn tail today.
- `OpenError` variants are public API after 0.1 ([ADR 0064](0064-the-0-1-compatibility-promise.md)).

## Options & trade-offs

### What proves durable damage

#### Option A - any CRC-valid frame of this log after the cut

- **For:** simplest check.
- **Against:** a closing-run frame after a torn run page proves nothing, and a lookalike copied into a torn payload would refuse a log that only lost an unacknowledged group.

#### Option B - a group frame with `base_offset` at or past the next expected offset

- **For:** only a frame written after the damaged one can carry such an offset. A lookalike copy carries a lower one, and forging a higher one needs the log id, which only a reader of the segment files has.
- **Against:** skips continuity with the frames before the cut, so it trusts one frame's own checks.

#### Option C - Option B, plus a valid chain from that frame to the end of the file

- **For:** stronger evidence.
- **Against:** one frame already proves it, and a torn last group after the proof would break the chain and hide the damage.

### How to find frames after the cut

#### Option D - try every byte position to the end of the file

- **For:** finds a proof wherever it lies.
- **Against:** one pass over the tail with a CRC per candidate. Cheap checks first (`body_len` in range and inside the file, kind is group) keep that rare, and it runs only on a damaged segment.

#### Option E - a bounded window after the cut

- **Against:** misses a proof beyond the window, which is the data loss this ADR exists to stop.

#### Option F - trust the bad frame's `body_len`

- **Against:** that field may hold the damaged byte.

### What the repair tool does with it

#### Option G - a new class F with a lossy plan

- **For:** reuses ADR 0065's seal run, `--drop-acknowledged` and backup. The log ends sealed, so no dropped offset is assigned again.
- **Against:** a sixth class to document and test.

#### Option H - fold it into class A

- **Against:** class A means a scan that stops before a valid closing run. Its lossy check reads that run's footer, which this case lacks.

#### Option I - diagnose only

- **Against:** the log never opens again without manual byte editing, which ADR 0065 exists to replace.

## Decision

We will build options B, D and G.

**Proof.** At open step 7, after a scan of an unsealed segment stops at position `p` while it expects offset `n` next, a proof is a group frame after `p` that passes every per-frame rule for readers (length, CRC, reserved fields, kind, entries fill the body) and has `base_offset >= n`. Continuity of offset and epoch with the frames before `p` is not checked, because the damage broke it. Closing frames are never a proof. A group frame with `base_offset < n` is a lookalike and is never a proof.

**Search.** Open tries every byte position from `p` to the end of the file and stops at the first proof. A position whose `body_len` is out of range, whose frame would pass the end of the file, or whose kind is not group is skipped before any CRC is computed.

**Order.** Step 7 first runs its existing locator and footer check. A valid closing run still fails with `Corrupt`, as class A. Only when that check does not refuse does open search for a proof. A proof fails open with `OpenError::Corrupt { segment, position: p }`, and open changes no file. No proof means a torn tail, and open truncates at `p` as ADR 0003 decides.

**Scope.** The same rule applies to the newest segment, to the second-newest segment that lacks its closing run and that open step 5 scans as the newest ([ADR 0061](0061-a-sealer-thread-builds-the-key-frames-off-the-writer.md)), and to `LogReader`, which reports `Corrupt` rather than a shorter log. One function in the core serves all three.

**Repair.** `verify` reports the case as class F: the newest segment is unsealed and has a proof after the cut. Its `truncate` plan is always lossy, with the cut at `p` and `dropped_offsets` from `n` to the end of the highest proof's offsets. `--apply` needs `--drop-acknowledged`, copies the segment aside, writes a seal run at `p` built from the group frames before the cut with the highest kept epoch plus one, calls `sync_all`, truncates the file to the end of that run and calls `sync_all` again, as ADR 0065's lossy plan does.

**Crash order.** The seal run must end at or before the start of the last proof frame in the file. Then the last proof survives until the truncation, and every crash point before it fails closed: open refuses with `Corrupt`, whether the scan stops inside an incomplete seal run or after a whole one. A rerun of the tool converges with no special case: a closing run counts only when its locator ends the file, so the scan stops at the cut again, the last proof still makes the plan class F with the same cut, and the tool writes the same seal run bytes again before it truncates. Truncating first and sealing after would leave a writable log between the two steps. If even the last proof leaves no room, the plan refuses, and the case stays manual.

**Second-newest segment.** The same damage in the second-newest segment that lacks its closing run is diagnosed and not repaired. A seal run there would leave a `Gap` before the next segment, and cutting every later segment is ADR 0065's rejected option C.

We chose this because it stops open from dropping acknowledged records without an operator, and it cannot be triggered by a caller's payload. We accept that a log with damage before an acknowledged group no longer opens on its own: it fails closed, and its repair ends the log.

## Consequences

- Amends ADR 0003: the contiguous valid prefix is kept only when no proof follows the damage. A torn tail is truncated as before.
- Amends ADR 0065: the class table gains F, and the refusal exit code covers a class F seal run with no room. `verify` no longer calls this case a torn tail.
- The repair JSON keeps `schema` 1. `"F"` is a new value of the existing `class` field, not a new field or a new meaning, so a consumer must accept class values it does not know.
- `OpenError` gains no variant. An unsealed newest segment can now return `Corrupt`, which [api.md](../architecture/api.md) states.
- Open's worst case on a damaged segment is one pass over its tail with a CRC per candidate frame. A healthy segment and a torn tail with no candidate cost nothing more.
- overview.md open step 7 gains the proof search and its order. testing.md gains the scenarios below, `Planned`.
- Ships in 0.1.1. [CHANGELOG.md](../../CHANGELOG.md) lists it under Fixed. No format version or protocol version changes.
- I3 stays conditional on the storage contract. A log whose durable bytes broke it now fails closed instead of losing records silently.

## Enforcement

Scenarios in [testing.md](../architecture/testing.md), on simulated storage in `segment-log-core/src/acceptance/`:

- `OPEN-DAMAGE-FOLLOWED`: the issue's reproduction. Open returns `Corrupt` at the cut and changes no file. `LogReader` and the second-newest segment without a closing run do the same.
- `OPEN-TORN-LOOKALIKE-COPY`: a torn last group whose payload holds a copy of an earlier frame of this log. Open truncates.
- `OPEN-TORN-RUN-LATER-FRAME`: a closing run with a lost inner page and a kept locator and footer. Open truncates, as today.
- `REPAIR-CLASS-F-VERIFY`, `-REFUSED`, `-SEALS`, `-NO-ROOM`, `-SECOND-NEWEST` and `-CRASH`: the repair cases above, with a crash at every storage call of `--apply` under `Kill`, `PowerLoss` and `PowerLossTorn`.
- A randomized oracle check: flip one byte anywhere before the start of the last group frame of an unsealed newest segment. Open returns `Corrupt` or keeps every acknowledged offset; it never returns `Ok` with a lower watermark. A failing seed prints and saves its storage trace.

## Revisit if

- The writer ever has more than one group frame unsynced in a segment, for example by pipelining writes ahead of their `fsync`. Then a torn earlier group can precede a kept later unsynced one, and the proof must lie past the last group that could be unsynced.
- The tail search shows up in open's time on a real damaged log.
- An operator needs the records after the damage in a form other than the backup.
