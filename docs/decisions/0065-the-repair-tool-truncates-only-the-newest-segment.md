---
id: "0065"
title: "The repair tool truncates only the newest segment, and seals a log whose repair drops acknowledged records"
status: accepted
date: 2026-11-17
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [recovery, corruption, operator, server, release-0.1]
touches: [segment-log-core/src/repair/**, segment-log-server/src/**, docs/architecture/api.md, docs/architecture/testing.md]
enforced_by: test
---

# 0065. The repair tool truncates only the newest segment, and seals a log whose repair drops acknowledged records

## Context

`OpenError::Corrupt { segment, position }` stops open until a person acts ([issue #1](https://github.com/ironflowx/segment-log/issues/1)). Open fails closed on purpose: at open step 7 it cannot tell a torn tail from a bad frame in a sealed segment, and truncating a sealed segment drops acknowledged records (R15, I7). Today the repair is manual: copy the directory, inspect the bytes, truncate the copy. 0.1 is the first production release ([issue #133](https://github.com/ironflowx/segment-log/issues/133)), so it ships a tool for this.

These facts are fixed:

- The storage trait, the frame scan and the closing-run builder are `pub(crate)` in `segment-log-core`. Every file write in the core goes through the storage trait, so that the crash simulation can cover it.
- Open reads only the locator, footer and key filter of each older sealed segment ([open step 5](../architecture/overview.md#open-and-recovery)). A bad group frame or key index page there surfaces later, as `ReadError::Corrupt` or as a poisoned writer at a key lookup.
- One known trigger for `Corrupt` in the newest segment is a power loss that tears its last group frame when that record's payload ends with bytes that form a valid footer and locator of this log. Frame CRCs cover the log id ([ADR 0049](0049-frame-crcs-cover-the-log-id.md)), so only a payload that holds this log's own closing bytes can do this.
- After 0.1, every build reads every format version from 0.1's onward, and a log can hold segments of several versions. A writer rolls rather than append to an older segment. The server's flags and exit codes are public API ([ADR 0064](0064-the-0-1-compatibility-promise.md)).
- `segment-log-server` has one command line, flags only ([ADR 0058](0058-one-server-process-serves-one-log.md)). The handoff reader is the same binary with `--read-only` ([ADR 0060](0060-the-server-reads-a-sealed-log-with-read-only.md)).

## Options & trade-offs

### Packaging

#### Option A - core module plus a server subcommand

- **For:** the logic uses the storage trait, the scan and the closing-run builder where they are, and the crash simulation covers the repair. 0.1 ships one musl binary, which is already on the reader host.
- **Against:** the server's argument parser grows a subcommand, and the serve command line must stay as it is.

#### Option B - core module plus a separate `segment-log-repair` binary or crate

- **For:** the server's command line stays untouched.
- **Against:** a second artifact to build, release and put on the host. A separate crate would also need the storage trait made public, or would lose the crash simulation.

### What repair may change

#### Option C - repair every class it can explain

- **For:** fewer manual cases. Key frames are derived data and could be rebuilt; damage in an older segment could be cut with every later segment.
- **Against:** rebuilding key frames rewrites a sealed segment (I7). Cutting an older segment drops every later segment and its acknowledged records. Each class needs its own crash-safe protocol before 0.1.

#### Option D - repair only open's step 7 refusal in the newest segment; diagnose the rest

- **For:** the one case that has a crash explanation, and the one #1 describes. A single protocol to prove crash-safe.
- **Against:** a damaged older segment still needs a backup or a person. The tool only explains it.

### When repair drops acknowledged records

Truncating the newest segment drops acknowledged records when CRC-valid group frames follow the damage. The next writer would then reuse those durable offsets (I6), and the dropped idempotency keys would let a retry run twice.

#### Option E - refuse

- **For:** no record is ever dropped by the tool.
- **Against:** the log never opens again without manual byte editing, which is what the tool exists to replace.

#### Option F - allow with an explicit flag, and leave the log sealed

- **For:** the operator chooses the loss with the lost offsets in front of them. A sealed log takes no writer, so no offset is used again.
- **Against:** the deployment must start a new log to keep writing.

#### Option G - allow and leave the log writable

- **Against:** breaks I6. Not acceptable without changing the invariant.

## Decision

We will build option A with options D and F.

**Packaging.** A `segment_log_core::repair` module holds the logic, std-only, with every file access through the storage trait. It is `pub` and `#[doc(hidden)]`: the server crate calls it, and it carries no semver promise in 0.1. The command line carries the promise. `segment-log-server repair <verify|truncate> …` runs it. The command line without a subcommand stays the serve command. [api.md](../architecture/api.md#operator-repair) is the only source for the subcommands, flags, exit codes and JSON output.

**Locking.** Every subcommand takes `LOCK` without waiting, as a writer does, and fails with the "locked" exit code if another process holds it. The tool never runs from `SegmentLog::open`; open keeps failing closed.

**Versions.** The tool reads each segment by its own format version, through the same reader code as open. It supports exactly the versions the build reads. It never rewrites a segment into another version; what it writes into a segment uses that segment's version.

**`verify`.** Read-only, and always deep. It checks every header, every frame of every segment including the group frames of older sealed segments, every closing run against its groups ([ADR 0054](0054-a-closing-run-must-match-its-groups.md)), every key filter and key index page, and the per-version rules. It reports each finding with its segment, position and class, and the frames around it. A torn tail that open would truncate on its own is reported and is not damage.

**Classes.** Only class A is repairable in 0.1:

| Class | Finding | 0.1 |
| --- | --- | --- |
| A | Open step 7 refuses: the newest segment's scan stops before a closing run whose locator and footer are valid | `truncate` |
| B | A group frame in an older sealed segment is not valid | diagnose |
| C | An older segment's closing run is not valid, or differs from its groups | diagnose |
| D | A key filter or key index page in an older segment is not valid | diagnose |
| E | `Gap`, `LogIdMismatch`, a bad header, or `UnsupportedVersion` | diagnose |

**`truncate`.** For class A, the plan names the cut position: where the scan stopped. It is a dry run unless `--apply` is given with `--at <segment>:<position>` equal to the plan's cut; a different plan refuses. The plan has one of two kinds:

- **Torn tail.** The footer of the refused run names no position at or after the cut. The tool truncates at the cut, calls `sync_all` on the segment and `fsync`s the directory. No acknowledged record is lost, and the log stays writable. This is what open would have done without the lookalike run.
- **Lossy.** The footer names a position at or after the cut. The footer names the last group frame of the segment for each of its streams, so any group frame after the cut makes the plan lossy; the tool needs no scan of the damaged bytes for frame boundaries. `--apply` refuses without `--drop-acknowledged`. With it, the tool writes a seal run (footer, seal frame and locator) built from the group frames before the cut, at the cut position, over the damaged bytes, with the highest kept epoch plus one. It calls `sync_all`, truncates the file to the end of that run, and calls `sync_all` again. The log is then sealed. The report lists every dropped offset.

The lossy order fails closed at every crash point. While the seal run is incomplete, or before the truncation, the file still ends with the old valid locator and the footer it names, and the scan stops at the cut, before the old run, so open returns `Corrupt` again and a rerun of the tool continues. Truncating first and sealing after would leave a writable log between the two steps, whose next open reuses the dropped offsets. The argument needs the old footer and locator to stay untouched until the truncation: open proves the old run from both. If the seal run would reach the old footer, the plan refuses, and that case stays manual. It happens when the bytes from the cut to the old footer, the damaged frame, the frames after it and any key frames, are fewer than the seal run's: its footer, which names only the streams before the cut, and 72 bytes. `REPAIR-CRASH` found this: a first version that guarded only the locator let a crash leave a writable log. Writing the sealed segment to a new file and renaming it over the old one would cover that case too, but the storage trait has no rename, and the volume requirements say nothing about rename atomicity.

A doubtful case counts as lossy. A lookalike footer inside a torn payload can name positions at or after the cut, so a torn tail can be classed as lossy: the cost is a sealed log, never a writable log that reuses offsets. A real closing run always gives a lossy plan, because open step 7 refuses only when bytes lie between the cut and the run's start, and the footer names the last group frame among them. A torn-tail plan comes only from a lookalike run.

**Originals.** Before any change, the tool copies the whole newest segment file, byte for byte, into a backup directory inside the log directory, named by the command line (`repair-<unix seconds>`), then `sync_all`s the copy and `fsync`s the backup directory and the log directory. Open ignores it, because open lists only `*.seg` files in the log directory. The tool never deletes a backup. A crash during the backup leaves the segment unchanged.

**Salvage.** No export of records after the damage in 0.1. Its output would be a new file format to promise. The backup keeps the exact bytes, and `verify` lists every frame after the damage.

We chose this because class A is the only `Corrupt` with a crash explanation, and one protocol is all 0.1 needs to prove. We accept that classes B–E stay a restore-from-backup or manual job, and that a lossy repair ends the log.

## Consequences

- [ADR 0071](0071-a-clean-drop-writes-a-close-frame.md) amends this decision. A class F plan whose only room is blocked by a close-frame proof first copies that proof past the seal run.
- [ADR 0068](0068-open-refuses-damage-with-acknowledged-groups-after-it.md) amends this decision. It adds class F, an unsealed newest segment with acknowledged group frames after the damage, whose `truncate` plan is always lossy.
- Accepted by the owner on 2026-11-19 for the 0.1 release ([issue 133](https://github.com/ironflowx/segment-log/issues/133)).
- api.md gains the [Operator repair](../architecture/api.md#operator-repair) section, and its note "Corrupt is for a human to resolve" points to it.
- testing.md gains the `REPAIR` coverage group, `Planned`. Implementation waits for format version 3 and builds against it.
- ADR 0058's command line gains the `repair` subcommand. The serve flags and exit statuses do not change.
- I7 still holds for every segment that has a successor. A class A cut changes only the newest segment, which open itself would truncate if it could tell the tear apart. A lossy repair seals the log, so I6 holds: no dropped offset is assigned again.
- The tool can drop acknowledged records, only with `--drop-acknowledged`, and only from a segment that broke the storage contract or was edited. I3 is conditional on that contract.
- Adding a repair for another class, a `salvage` subcommand, a full-copy `--out` mode or a lock-free verify on a read-only volume later adds to the promise without breaking it.

## Enforcement

The `REPAIR` scenarios in [testing.md](../architecture/testing.md#property-and-unit-tests): core scenarios on simulated storage in `segment-log-core/src/acceptance/`, including `REPAIR-CRASH` at every storage call of the apply step under `Kill`, `PowerLoss` and `PowerLossTorn`, and `REPAIR-CLI` in `segment-log-server` against a real temporary directory.

## Revisit if

- A real case of class B, C or D shows up in production.
- An operator needs the records after the damage in a form other than the raw backup.
- The deployment cannot fit a copy of the newest segment on the surviving volume.
