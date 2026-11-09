---
id: "0014"
title: "Crash testing with simulated storage and real-process kills"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [testing]
touches: [docs/architecture/testing.md]
enforced_by: none
---

# 0014. Crash testing with simulated storage and real-process kills

## Context

Source: the gate is from the reviewed design ("Build order": `kill -9` during a group commit, and a power loss that drops the unsynced tail). The way to test it is new in this repository and not reviewed.

`kill -9` keeps the bytes in the page cache, so it does not test power loss. A real power cut cannot run in CI. A crash test that passes but does not model the real fault gives false confidence.

## Options & trade-offs

### Option A - a storage trait with simulated storage, plus real-process `SIGKILL` tests, plus an optional LazyFS job

- **For:** deterministic, seeded power-loss tests that run on every OS in CI. Real-process tests catch problems the simulation cannot. LazyFS checks power loss on a real Linux filesystem.
- **Against:** the simulation is only as good as its crash model. A storage trait adds a small layer.

### Option B - real filesystems only (`dm-log-writes`, `dm-flakey`, LazyFS)

- **For:** tests the real kernel.
- **Against:** Linux only, needs root or FUSE, slow, and hard to replay.

### Option C - `kill -9` only

- **For:** easy.
- **Against:** does not test the power-loss rule at all.

## Decision

We will test crashes with simulated storage, real-process `SIGKILL` tests and an optional LazyFS job, as [testing.md](../architecture/testing.md) describes (option A). We accept that the simulation is only as good as its crash model. All file I/O in the core goes through one storage trait. The simulated storage models `Kill`, `PowerLoss` and `PowerLossTorn`, and it injects `fsync` errors, short writes and `ENOSPC`. The oracle checks the invariants after every crash. M1 is done only when these tests and the real-process `SIGKILL` test pass.

## Consequences

- [ADR 0039](0039-scenario-first-acceptance-tests-in-native-rust.md) proposes how scenarios reach the simulated storage, and replay from a saved storage trace as well as the seed.
- [ADR 0029](0029-m1-completion-requires-correctness-and-measurement-evidence.md) adds required platform I/O evidence and a reproducible Linux benchmark report to M1 completion. Correctness and durability failures block completion; provisional performance misses are reported findings.
- Every new write-path feature needs crash cases in the simulation.
- Every failing seed must be printed, so that the failure can be replayed.

## Enforcement

None yet. When the crash tests exist, `cargo test --workspace` in [CI](../../.github/workflows/ci.yml) will run them on Linux and macOS. The LazyFS job is optional and not in CI.

## Revisit if

- A bug found outside the simulation shows that the crash model is missing a fault. Then add the fault to the model, and add a regression seed.
