---
id: "0032"
title: "Qualify deployment storage before M6"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [durability, deployment, testing, milestones]
touches: [docs/prd.md, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: none
---

# 0032. Qualify deployment storage before M6

## Context

The library's durability contract assumes flush persistence, sector-atomic writes and filesystem behavior after metadata-write errors. Simulated storage models those assumptions. A syscall trace verifies the library's calls without establishing the behavior of a guest's hypervisor, backing storage and filesystem.

PRD open item 3 requires checking the actual stack before the M6 benchmark. The owner approved keeping that deployment gate without blocking M1 implementation against the explicit conditional contract during the readiness discussion on 8 November 2026.

## Options & trade-offs

### Option A - qualify the actual deployment stack before M6 validation

- **For:** M1 can implement and test its stated contract. Deployment durability claims still require evidence for the stack that will run the log.
- **Against:** deployment qualification remains required work. Passing library tests does not make an eventual guest deployment qualified.

### Option B - require deployment qualification before implementing M1

- **For:** determines support for the chosen guest stack before implementation begins.
- **Against:** makes core implementation depend on an environment-specific task even though its storage assumptions are explicit and can be simulated.

### Option C - treat passing library tests as deployment qualification

- **For:** no separate deployment evidence.
- **Against:** does not verify whether the real stack preserves the assumptions needed by the durability and recovery contract.

## Decision

Keep the [storage contract](../architecture/overview.md#storage-contract) explicit and conditional. M1 implements and tests against it. The full M1 completion gate in ADR 0029 remains mandatory, including the Linux benchmark and cross-platform correctness and directory-sync evidence.

The actual guest storage stack must be qualified before the M6 guest benchmark and before making production durability claims for that stack. Record the hypervisor, virtual block device, backing storage, filesystem and mount options, with versions and configurations needed to identify the tested setup.

Qualification must record supporting evidence and limitations for successful-flush persistence, atomic sector writes needed by recovery's identical-byte rewrite, and the assumed filesystem response to metadata-write errors. The methods and supporting evidence must apply to the identified stack. Passing syscall checks alone is not sufficient.

M1's tests and measurements do not certify an unspecified or different deployment. Unverified assumptions or evidence contradicting the contract prevent qualification; they must not be recorded as successes. PRD open item 3 remains an evidence task until the actual stack is identified and checked.

We accept leaving deployment qualification until that stack is available while preserving its pre-M6 requirement. This decision schedules evidence; it does not establish that any platform meets the assumptions.

## Consequences

- The gate and M1 boundary of PRD item 3 are settled. The actual qualification result remains open.
- A qualification record in `docs/` must distinguish verified support, limitations and unresolved assumptions before M6 guest validation proceeds.
- This decision does not relax R14, the storage contract, or the M1 completion gate.
- A change to the qualified stack requires reviewing whether its evidence still applies.

## Enforcement

None yet. The deployment qualification checklist in [testing.md](../architecture/testing.md#deployment-storage-qualification-before-m6) defines the required record. M6 completion evidence must identify the qualified stack used for its guest benchmark.

## Revisit if

- The chosen deployment cannot satisfy the contract. Its recovery or storage design needs a separate decision before it can be qualified.
