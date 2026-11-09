---
id: "0037"
title: "Retain complete logs through current milestones"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [retention, deployment, recovery, milestones]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: none
---

# 0037. Retain complete logs through current milestones

## Context

PRD open item 11 defers segment retention until after M4. ADR 0011 proposes deletion only when a later footer covers every stream in a segment. A stream that never appears in a later footer prevents that rule from reclaiming its segment. The target workload keeps history from short-lived guests.

The owner approved retaining complete logs through M1–M6, with whole-log archival and expiry owned by deployment, during the readiness discussion on 8 November 2026. This ADR amends the retention proposal in ADR 0011 and extends the lifecycle ownership in ADR 0033.

## Options & trade-offs

### Option A - retain complete history in the current milestones

- **For:** preserves stream revisions, retained-record reads and retry keys without a segment-deletion protocol. Matches a log kept as one history after the guest exits.
- **Against:** storage grows with history. Deployment must manage the lifetime of complete logs rather than expecting the library to reclaim segments.

### Option B - delete segments only after later footer coverage

- **For:** could reclaim a segment whose stream heads are all preserved in later segments.
- **Against:** cannot reclaim a segment with a stream absent from every later footer. Still needs explicit key expiry, deleted-offset reads and crash-safe deletion behavior.

### Option C - design bounded segment retention now

- **For:** can support long-running logs with a storage-reclamation requirement.
- **Against:** needs durable revision checkpoints and new read, retry and deletion contracts outside the current milestone scope.

## Decision

M1–M6 retain each log's complete valid history. The core implements no segment-level retention or automatic deletion of valid history segments, including when a later footer covers their streams. It does not silently reclaim acknowledged history to handle storage pressure.

The deployment layer owns whole-log archival and expiry under its lifecycle policy. This assigns responsibility; it adds no core or server archival API, deletion schedule or automatic expiry threshold. Concrete lifecycle integration remains deployment work under ADR 0033.

Recovery cleanup is unchanged. Keeping the valid prefix, truncating an invalid uncommitted tail, or removing an incomplete new segment under the existing recovery rules is not retention of valid history. Storage errors retain the existing poison and recovery contract.

If bounded long-running logs become a requirement, segment retention needs a separate feature decision covering durable revision checkpoints, key expiry, reads from deleted offsets, and crash-safe deletion. It is not automatically included after M4 or in M5–M6.

We accept storage growth and explicit deployment lifecycle work to preserve one complete history without a premature deletion protocol.

## Consequences

- [ADR 0040](0040-volumes-meet-testable-minimum-requirements.md) makes capacity sizing and a timely free-space alert a volume requirement (V9), because complete retention grows storage and `ENOSPC` poisons the writer.
- PRD open item 11 is resolved for the current milestones. R22 no longer schedules segment retention after M4.
- Stream heads and keys remain reconstructible from retained history. ADR 0031's M4 lookup and key-cache memory work still applies.
- No retention option or segment-deletion endpoint is added to the public API.
- Whole-log archival and expiry belong to deployment, separately from the library's recovery cleanup.

## Enforcement

None yet. The complete-history cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will verify retained records and keys after reopen, retention of valid segments across M3 rolls, and readable sealed history from M4. Recovery cleanup cases remain required and distinct.

## Revisit if

- A concrete long-running workload needs bounded history and can define the associated revision, read and retry semantics.
