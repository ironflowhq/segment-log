---
id: "0012"
title: "No existing log crate as a dependency"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [dependencies]
touches: [segment-log-core/Cargo.toml]
enforced_by: none
---

# 0012. No existing log crate as a dependency

## Context

Source: the survey result is from the reviewed design (decision 5). The rule for new dependencies is new in this repository and not reviewed.

A survey on 5 November 2026 looked for a crate with the whole contract: one writer, group commit, a durable commit boundary, many streams in one file, expected revision, idempotency keys, a queue that refuses when full, and seal. None showed it. The details are in [prior art](../architecture/prior-art.md#crates-surveyed).

## Options & trade-offs

### Option A - write the log; use `std` only in the core crate

- **For:** the contract is ours to test. The binary stays small. No pre-1.0 dependency in the durability path.
- **Against:** more code to write: the CRC, the queue, the file layout.

### Option B - build on `open-wal` or `raft-engine`

- **For:** the physical layer exists.
- **Against:** neither has streams, expected revision or keys, so the product half is still to write. Each has its own recovery rules that would need checking.

## Decision

We will write the log ourselves and use only `std` in the core crate (option A). We accept writing the CRC, the queue and the file layout ourselves. A new runtime dependency in `segment-log-core` needs an ADR that gives the measured or tested reason. Dev-dependencies (for example `proptest`) and dependencies of `segment-log-server` do not need an ADR, but they must not reach the core's write path.

## Consequences

- Read `open-wal`'s book for the physical contract. Do not link it.
- The CRC-32C table and the queue are written here or come from `std`.

## Enforcement

None. A CI step that fails when `segment-log-core/Cargo.toml` has a `[dependencies]` entry would enforce it. Until then, a reviewer checks the manifest.

## Revisit if

- A crate appears that shows the whole contract with tests for crashes and power loss.
- A measurement shows that a dependency, such as a hardware CRC crate, is needed for a target.
