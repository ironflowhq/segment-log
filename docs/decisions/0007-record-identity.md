---
id: "0007"
title: "Record identity: log offset, file position, stream revision"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, identity]
touches: [docs/architecture/api.md]
enforced_by: none
---

# 0007. Record identity: log offset, file position, stream revision

## Context

Source: from the reviewed design (decision 4).

A reader that catches up on the whole log needs a cursor across all segments. A stream needs its own order for optimistic concurrency. Storage needs a byte position. If these are mixed, a caller can store a byte position as an identity, and it breaks when segments are deleted.

## Options & trade-offs

### Option A - three separate numbers

- **For:** each number has one job. The caller stores only the log offset and the revision.
- **Against:** the log must map offsets to positions (the sparse index).

### Option B - the byte position as the identity

- **For:** no index.
- **Against:** the identity depends on the file layout and breaks across segments.

### Option C - stream revision only

- **For:** simple for one stream.
- **Against:** no cursor for reading all streams in order.

## Decision

We will keep the log offset, the file position and the stream revision as three separate numbers (option A). We accept that the log must map offsets to positions with the sparse index.

- **Log offset**: one monotonic index across all segments, starting at 0, assigned by the writer. It is the tail cursor. A durable offset is never used again. Offsets from a torn tail can be.
- **File position**: the byte position inside one segment. The log uses it internally. It is not an identity for callers.
- **Stream revision**: the index inside the stream. The first record is revision 0. The record stores the revision assigned after the check. The request's expected revision is a precondition and is not stored.

A rejected submit uses no log offset.

## Consequences

- Reading one stream from an arbitrary revision is a filtered scan. The tail map finds only the latest record of a stream, not revision N.
- The sparse index is a cache that the log rebuilds by a scan.

## Enforcement

None yet. Oracle checks 1 and 10 in [testing.md](../architecture/testing.md#the-oracle) will check that offsets and revisions survive recovery and that a durable offset is never used again.

## Revisit if

- Callers need fast reads of one stream from a revision. That needs a per-stream index and a new ADR.
