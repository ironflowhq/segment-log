---
id: "0039"
title: "Scenario-first acceptance tests in native Rust, with private storage injection"
status: proposed
date: 2026-11-08
deciders: []
supersedes: null
superseded_by: null
tags: [testing, milestones]
touches: [docs/acceptance-tests.md, docs/architecture/testing.md, segment-log-core/src/**, segment-log-core/tests/**]
enforced_by: none
---

# 0039. Scenario-first acceptance tests in native Rust, with private storage injection

## Context

M1 is the first implementation milestone. [ADR 0014](0014-crash-testing-strategy.md) fixes the crash strategy: a storage trait, simulated storage, real-process `SIGKILL` and an optional LazyFS job. It does not say how a behavior moves from contract to test, where tests live, how they reach the simulated storage, or how a failing randomized run is replayed.

Constraints that are already fixed: the core uses only `std` ([ADR 0012](0012-no-log-crate-dependency.md)). The public API in api.md has no storage type parameter. One writer thread runs as a real OS thread, so a seed alone does not reproduce how threads interleave. M5 fuzz targets build as a separate crate on a nightly toolchain. M6 needs the same conformance cases over two transports from a Go client.

## Options & trade-offs

### Option A - native Rust scenarios in the crate, private storage injection, trace-based replay

- **For:** no new dependency or file format. The in-crate suite can pass simulated storage through a private constructor and use probes compiled only for tests, so the public API stays unchanged. Production and tests run the same writer and recovery code. Replay from a recorded storage trace is deterministic even though thread interleavings are not.
- **Against:** in-crate tests can reach internals, so reviewers must keep assertions on results that callers can see. Requirement traceability needs case IDs and a coverage check instead of a feature-file format.

### Option B - Gherkin feature files with a runner such as `cucumber`

- **For:** a language-neutral format that non-Rust readers can edit.
- **Against:** a second format and step-definition layer for one Rust team. Step glue still needs private access to the simulated storage. It adds a dev-dependency and an async-oriented runner for no M1 gain.

### Option C - a public or feature-gated testing API used from `tests/`

- **For:** integration tests, benches and fuzz targets can all reach the simulated storage the same way.
- **Against:** the storage seam and the simulator become part of the published surface, or a feature that users can enable. Every test-only change becomes an API decision.

## Decision

We will use option A. Scenarios are ordinary Rust `#[test]` functions named for behavior, with the Given/When/Then example and the covered case IDs in the doc comment. Each behavior starts with a scenario that fails on its behavioral assertion. testing.md gives each required case a stable ID without a milestone, and a `std`-only coverage check maps tests to case IDs.

`SegmentLog` holds its storage as a trait object. A private constructor takes simulated storage, which is compiled only under `cfg(any(test, fuzzing))`. Scenarios stop the writer by holding storage calls, not by adding hooks to the writer. Randomized crash tests record a storage trace, derive the crash state from the trace and a seed, and replay from a saved trace. M6 keeps its conformance cases as shared data run by the Go client.

We accept the discipline cost of in-crate tests in exchange for an unchanged public API and no new dependency.

## Consequences

- [ADR 0043](0043-zero-linger-and-empty-read-boundaries.md) settles the two boundary outcomes left open below. Their definitions are in api.md.
- [ADR 0014](0014-crash-testing-strategy.md)'s rule to print each failing seed now also saves the trace needed to rebuild the crash state.
- The [acceptance-test workflow](../acceptance-tests.md) and testing.md's case IDs must stay in step. A new required case gets an ID in testing.md in the same change.
- Two outcomes that deterministic scenarios depend on are not yet defined: zero linger with queued submits, and `read` past the watermark or with `max` of zero. They must be settled in api.md first.
- M5 adds items compiled under `cfg(fuzzing)` for the decoder and recovery scan, and a nightly CI job. M6 adds a Go toolchain job.
- loom and shuttle stay out of the core unless a later ADR accepts them under ADR 0012.

## Enforcement

None yet. The coverage check that lands with the first M1 scenario will fail on unknown case IDs and report uncovered cases. `cargo test --workspace` in [CI](../../.github/workflows/ci.yml) will run it on Linux and macOS.

## Revisit if

- Contributors who do not write Rust need to author executable core scenarios.
- A test need cannot be met without a public testing API.
- In-crate tests are found asserting on internals instead of results that callers can see.
