---
id: "0049"
title: "Frame CRCs cover the log id"
status: proposed
date: 2026-11-11
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [format, recovery, integrity]
touches: [segment-log-core/src/codec.rs, segment-log-core/src/engine/recovery.rs, docs/architecture/format.md, docs/architecture/overview.md, docs/architecture/api.md, docs/architecture/testing.md]
enforced_by: test
---

# 0049. Frame CRCs cover the log id

## Context

Open step 7 ([overview.md](../architecture/overview.md#open-and-recovery)) handles a newest segment whose forward scan stopped before the end of the file. If the last 40 bytes are a valid locator that points to a valid footer, and the scan stopped before that footer's closing run, open fails with `Corrupt`. Otherwise it truncates where the scan stopped. The check exists because the same bytes can be a sealed segment with a bad group frame. Truncating that segment would drop acknowledged records and break I3 and I7.

Payload bytes can pass this check. A caller writes a payload that ends with a CRC-valid footer, seal frame and locator. Power fails during that group's `fsync`. The page with the payload's end survives, and an earlier page of the group is lost. Recovery sees the same evidence as a sealed segment with a bad frame, and open fails with `Corrupt`, although no acknowledged record depends on that group. The log stays closed until a person repairs it. An adversarial review of the M4 docs reproduced this with three independent models of the spec. The M4 docs record it as a known cost.

These facts are fixed:

- A frame's CRC is CRC-32C over its `body_len` and its body ([format.md](../architecture/format.md#frame)). A caller controls payload bytes and can compute that CRC for a frame inside its payload.
- By accident, a payload needs CRC-valid frames in an exact closing-run layout. In practice only a deliberate caller can cause this.
- Recovery cannot drop the check: real corruption would then truncate acknowledged records without an error.
- The CRC of a frame that failed cannot be trusted, so the frame's own length cannot show that the closing run lies inside its payload.
- Each segment header holds `log_id`, 128 random bits chosen at creation. The public API does not expose it. Callers append and read records; they do not read segment files.
- The format is provisional until M5. A change before M5 increments `format_version` and needs no migration ([ADR 0013](0013-provisional-v0-byte-layout.md)). ADR 0013 lists "which fields are inside the CRC" for review at M5. The design that ADR 0013 started from had a log id inside the frame CRC.
- `segment-log-core` uses only `std` ([ADR 0012](0012-no-log-crate-dependency.md)).

## Options & trade-offs

### Option A - keep the check and document the cost

- **For:** no format change. The M4 docs already say this.
- **Against:** a caller can turn a power loss into an outage that needs a person. `Corrupt` stops meaning "the storage damaged a sealed segment".

### Option B - drop the check and always truncate the newest segment

- **For:** open never stops for a torn tail.
- **Against:** a sealed segment with a bad group frame loses its later acknowledged records and its seal, with no error. Retries of those keys append duplicates. It breaks I3 and I7.

### Option C - infer from the torn frame's length

- **For:** no format change. If the frame where the scan stopped claims a length that covers the footer, the footer is payload.
- **Against:** that frame failed its CRC, so its length is not trustworthy. Damage to the length field turns real corruption into a wrong truncation. Useful as a hint in a repair report, not as an action.

### Option D - store the log id in the locator

- **For:** only the locator changes. Step 7 rejects a locator whose `log_id` differs from the header's.
- **Against:** the locator grows from 40 to 56 bytes, which contradicts [ADR 0017](0017-footer-locator-offset-and-size.md) and [ADR 0048](0048-sealed-segments-carry-a-key-filter-and-index.md). It protects step 7 only. A rule that differs by frame kind is one more thing to get wrong.

### Option E - start every frame's CRC with the log id

- **For:** no field or size changes. A frame's CRC covers the segment header's 16 `log_id` bytes, then `body_len`, then the body. A caller that does not know the log id cannot make a valid frame of any kind. One rule covers every frame. The writer computes the CRC state after the log id once per log, so a frame costs the same as now.
- **Against:** every frame encoder and decoder needs the log id. Hand-built test files and the M5 fuzz targets must include it. A frame copied from another log's segment no longer validates. It is not a cryptographic check: anyone who can read the segment files can read the log id.

### Option F - a keyed MAC in place of the CRC

- **For:** SipHash-2-4 keyed by the log id is already in the format ([ADR 0048](0048-sealed-segments-carry-a-key-filter-and-index.md)). A 64-bit tag is harder to forge than a 32-bit CRC.
- **Against:** the key is stored in every segment header, so a reader of the files can forge either one. The frame envelope changes, and hashing every payload with SipHash costs more than CRC-32C. It protects against nothing that E does not.

## Decision

We will use option E. Every frame's CRC is CRC-32C over the segment's `log_id`, then `body_len`, then the body. The segment header's own CRC does not change. Step 7 keeps its check. A payload can no longer pass it unless the caller knows the log id, so `Corrupt` from step 7 means a sealed segment with a bad frame. The M4 layout, including this change and [ADR 0048](0048-sealed-segments-carry-a-key-filter-and-index.md), is `format_version` 1.

E removes the forged case without the data loss of B or the guessing of C, and it changes no field width, unlike D and F. We accept that the check is not cryptographic. A caller that can read the segment files can still forge a closing run. That caller is outside the threat model, because such a caller could also damage or copy the log by other means.

## Consequences

- The M4 docs drop the known cost from open step 7, the `Corrupt` note in api.md and the `RECOVERY-FIXTURES` case added for it. That fixture now expects a truncation.
- format.md defines the CRC input and gives a check value with a stated log id. A forward scan, `LogReader` and the writer's key lookup all validate frames with the log id from the segment header.
- A frame from another log fails its CRC. In the active segment it ends the valid prefix. In a sealed segment it is `Corrupt`. Header checks already reject segment files from another log.
- The writer and recovery code keep one CRC state per log, computed after the log id. Frames cost the same to write and check.
- A log written by M1–M3 cannot open under M4. M4 already breaks that through its closing frames, so the version bump covers both.
- ADR 0013's M5 review of "which fields are inside the CRC" starts from this rule.

## Enforcement

The M4 tests must add these, and testing.md names their cases:

- `CODEC-GROUP` checks the frame CRC against a check value with a stated log id, and checks that a frame validates under its own log id and fails under another one.
- `RECOVERY-FIXTURES` builds a torn last group whose payload ends with a closing run whose CRCs omit the log id, keeps the tail page and drops an earlier one, and expects a truncation.
- `RECOVERY-FIXTURES` keeps the case where a bad group frame before an intact closing run fails with `Corrupt`.

## Revisit if

- The public API, a log file name or a log path starts to expose the log id to callers.
- A deployment gives callers read access to the log directory. A forged closing run is then possible again, and a check that callers cannot compute would need a key stored outside the segments, which [I9](../architecture/overview.md#invariants) does not allow today.
- The M5 review of the CRC fields chooses another integrity check.
