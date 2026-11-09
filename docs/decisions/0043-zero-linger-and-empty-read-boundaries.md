---
id: "0043"
title: "Zero linger drains queued work and boundary reads return empty"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, group-commit, read, testing]
touches: [docs/architecture/api.md, docs/acceptance-tests.md, docs/architecture/testing.md]
enforced_by: none
---

# 0043. Zero linger drains queued work and boundary reads return empty

## Context

[ADR 0039](0039-scenario-first-acceptance-tests-in-native-rust.md) leaves two outcomes open before deterministic M1 scenarios can use them. The owner accepted the recommendations in the M1 implementation discussion on 2026-11-08.

## Options & trade-offs

### Option A - drain queued work without waiting

- **For:** queued callers can share a flush without deliberate waiting.
- **Against:** a caller can wait for other queued submits to be classified.

### Option B - commit after the first new submit

- **For:** less classification work before that submit's flush.
- **Against:** already queued submits cannot share that flush.

### Option C - return empty reads at the boundaries

- **For:** polling beyond the current end and requesting no records need no special error handling.
- **Against:** an accidentally advanced cursor returns no records instead of an error.

### Option D - reject reads beyond the watermark or with a zero limit

- **For:** detects those caller inputs explicitly.
- **Against:** requires an API error choice for otherwise harmless empty reads.

## Decision

Use options A and C. The [API](../architecture/api.md#option-validation) defines zero-linger gathering. Its [read signature](../architecture/api.md#target-signatures-and-defaults) defines empty boundary results. These choices preserve queued flush sharing and make bounded reads usable for polling. We accept the classification delay and lack of cursor validation described above.

## Consequences

- Deterministic group tests can queue work behind a held flush without relying on a sleep.
- `GROUP-FORMATION` and `READ-BOUNDED` no longer depend on unresolved outcomes.
- No format change, new error variant or default change is needed.

## Enforcement

None yet. The M1 `GROUP-FORMATION` and `READ-BOUNDED` scenarios in [testing.md](../architecture/testing.md) must cover these outcomes.

## Revisit if

- Measured queued classification cost makes zero-linger latency unsuitable for callers.
- A consumer needs explicit detection of a cursor beyond the current end.
