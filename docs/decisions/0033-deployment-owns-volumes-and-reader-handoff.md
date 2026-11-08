---
id: "0033"
title: "Deployment owns volumes and reader handoff"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [architecture, deployment, storage, read-path]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: none
---

# 0033. Deployment owns volumes and reader handoff

## Context

The log runs inside a short-lived guest, but its history must survive the guest. The core API takes a directory path. PRD open item 4 leaves the surviving volume's mount location and post-guest reader unspecified. The architecture already places orchestration above the log.

The owner approved assigning the volume lifecycle and reader handoff to the deployment layer, with concrete paths and components settled before M6, during the M1 readiness discussion on 8 November 2026.

## Options & trade-offs

### Option A - deployment manages the volume and reader handoff

- **For:** the embedded core works on a supplied directory. File-format validation, recovery and reading do not depend on a volume provider or runner topology.
- **Against:** the deployment must preserve the volume, arrange access after guest exit, and coordinate writer ownership during handoff.

### Option B - the core manages the volume and reader lifecycle

- **For:** callers could delegate more lifecycle work to the library.
- **Against:** introduces environment-specific provisioning, mounts, permissions and process policy into an embedded log that currently needs only file I/O.

### Option C - defer ownership along with the concrete components

- **For:** makes fewer decisions before the deployment is selected.
- **Against:** leaves preservation and handoff without a named responsible layer, despite being required for post-guest access.

## Decision

The deployment or runner layer provisions a volume that outlives the guest, mounts it, preserves its lifetime, and passes the log directory to `SegmentLog::open`. The core owns validation, recovery and reading inside that supplied directory, including directory and file creation under its existing API.

The deployment layer also arranges post-guest access and selects the reader service or tool. From M4, `LogReader` reads sealed history without writes. An unsealed crash log requires writable recovery through `SegmentLog::open`. Using the sealed reader afterward requires an explicit `seal` call under the existing API. This ownership decision does not add automatic sealing or choose an application's resume-versus-finalize policy.

Before M6, identify the concrete mount paths and reader component and document the handoff procedure. It must preserve the volume, respect the single-writer contract, and explain how the reader obtains sealed history or access for required writable recovery. Storage qualification remains a separate requirement under ADR 0032.

M1 continues to work with a caller-supplied directory. It does not depend on selecting a volume provider, mount path or post-guest reader process.

We accept explicit deployment work to keep environment-specific lifecycle policy outside the embedded core.

## Consequences

- [ADR 0037](0037-retain-complete-logs-through-current-milestones.md) assigns whole-log archival and expiry to the deployment layer. Current milestones retain all valid history segments without a core retention API.
- The responsibility boundary in PRD open item 4 is settled. Concrete paths, components and handoff procedure remain pre-M6 integration work.
- Provisioning and mount permissions belong to the deployment layer. The core retains its existing file I/O and recovery responsibilities.
- The caller must arrange volume survival; destroying an ephemeral guest disk cannot be repaired by log recovery.
- Post-guest read-only access applies to sealed history. A deployment must arrange writable recovery when it needs to process an unsealed crash log.
- No core API signature, on-disk format or M1 runtime dependency changes.

## Enforcement

None yet. The surviving-volume handoff checks in [testing.md](../architecture/testing.md#surviving-volume-handoff-before-m6) require a recorded integration procedure and post-guest reading evidence before M6 deployment validation.

## Revisit if

- A supported deployment cannot supply a mounted directory or arrange required writable recovery. That needs a separate storage or reader contract decision.
