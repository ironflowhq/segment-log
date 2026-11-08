---
id: "0029"
title: "M1 completion requires correctness and measurement evidence"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [testing, durability, performance, milestones]
touches: [docs/prd.md, docs/architecture/testing.md, docs/decisions/0005-flush-calls-and-platforms.md]
enforced_by: none
---

# 0029. M1 completion requires correctness and measurement evidence

## Context

The M1 milestone names crash tests as its completion gate. Other documents also require platform flush checks and a core benchmark, but the milestone does not include that evidence. ADR 0005 places a roll-specific syscall check in M1 even though roll ships in M3.

The performance targets remain provisional. No measurement yet shows whether the target workload meets them. The owner approved a mandatory correctness, platform I/O and benchmark-evidence gate, with performance misses reported as findings, during the M1 readiness discussion on 8 November 2026.

## Options & trade-offs

### Option A - require correctness and measured evidence, with diagnostic performance targets

- **For:** establishes crash safety and real I/O behavior, produces performance evidence, and records misses without assuming that provisional thresholds are achievable.
- **Against:** M1 can complete with known performance work remaining. Linux measurement and cross-platform evidence are required even when local tests pass.

### Option B - require every provisional performance target to pass

- **For:** M1 completion would also demonstrate performance against those thresholds.
- **Against:** unvalidated targets could block the correctness milestone. Several targets depend on later features absent from M1.

### Option C - require crash tests alone

- **For:** the smallest completion checklist.
- **Against:** allows M1 to finish without verifying platform flush calls or producing the benchmark already required by the docs.

## Decision

M1 is complete only when the [M1 completion gate](../architecture/testing.md#m1-completion-gate) passes. Correctness tests must pass on Linux and macOS, including simulated crashes, real-process `SIGKILL`, recovery, poisoning, retries and admission limits.

The evidence must show one `fdatasync` per new committed group on Linux, and successful directory-sync checks on both platforms during creation and recovery. Initialization and recovery syncs are distinct from steady-state group commits. The roll-specific directory-sync trace moves to M3.

A reproducible Linux core benchmark report is mandatory. It records the applicable measurements and conditions in [testing.md](../architecture/testing.md). Missing measurements or missing reproducibility information block completion. A provisional M1 performance target miss is reported as a finding rather than blocking M1. Correctness or durability failures always block completion.

We accept completing M1 with documented performance misses so later work starts from actual measurements rather than assumed thresholds. Passing tests alone, macOS benchmark results alone, or a successful build cannot satisfy the gate.

## Consequences

- [ADR 0032](0032-qualify-deployment-storage-before-m6.md) distinguishes this mandatory M1 evidence from qualification of the actual deployment stack before M6. The M1 gate is unchanged.
- This decision amends the completion evidence around ADR 0014 and the milestone timing of ADR 0005's checks.
- Later milestones remain behind the M1 gate. An absent Linux run is missing evidence, not a pass.
- M1 measures only features it implements. Footer-based open targets and seal measurements wait for M4; server footprint and socket benchmarks wait for M6.
- Existing later-milestone gates are unchanged. Diagnostic M1 targets do not relax the M4 open-time gate or later requirements.
- Reports distinguish measurements, target misses and unmeasured conditions. They do not silently change targets or claim a target passed under different conditions.

## Enforcement

None yet. Repository checks and tests will run in Linux and macOS CI. The M1 completion record must link their results, the syscall and directory-sync evidence, and the reproducible Linux benchmark report before M1 is marked complete.

## Revisit if

- Measurements justify adopting an explicit performance threshold as a future milestone gate.
