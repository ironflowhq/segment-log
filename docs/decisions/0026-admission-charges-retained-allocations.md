---
id: "0026"
title: "Admission charges retained allocations"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, admission, memory]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md, docs/glossary.md]
enforced_by: none
---

# 0026. Admission charges retained allocations

## Context

ADR 0025 adds a byte reservation for every accepted request until it is answered. It leaves the charge formula undecided. The API accepts vectors whose capacity can exceed their length. Encoded record size does not include that unused capacity or all per-record structures.

The owner approved charging retained allocations, including unused buffer capacity and explicit bookkeeping and result allowances, during the M1 readiness discussion on 8 November 2026. This ADR amends ADR 0025.

## Options & trade-offs

### Option A - charge retained allocations and explicit request allowances

- **For:** accounts for the input allocations the log owns, including preallocation and structural overhead for many small records.
- **Against:** preallocation can cause a small-on-disk request to consume more budget. Charges depend on platform and type layout. Opaque allocator and standard-library internals cannot be measured exactly through this accounting.

### Option B - charge encoded record bytes

- **For:** simple, portable, and directly related to the bytes written to disk.
- **Against:** undercounts unused input capacity, record structures and request bookkeeping. A small encoded payload can retain a large allocation.

## Decision

M1 calculates each request's admission charge from its retained allocations, including unused capacity. The authoritative formula is in [api.md](../architecture/api.md#admission-charge).

The charge includes record-vector capacity, the capacities of every initialized record's stream, key and payload buffers, result-position storage, and explicit allowances for library-owned request bookkeeping. Bookkeeping includes the request envelope, reply metadata and a possible retry waiter. Allowances derive from the concrete types' sizes and account for additional request-owned copies or buffers if implementation introduces them.

Compute the charge before acceptance and keep that reservation until the answer under ADR 0025. Freeing or moving some data during processing does not reduce the reservation. Reserve result and possible retry-waiter storage at admission so those states cannot grow the charge after acceptance.

Use checked arithmetic. An unrepresentable charge must not wrap or be admitted. Its error classification is settled with requests that cannot fit the entire budget under PRD open item 10.

We accept platform-dependent charges and stricter treatment of preallocated inputs to account for the resources each request owns.

## Consequences

- [ADR 0028](0028-too-large-is-distinct-from-temporary-full.md) resolves open item 10. Impossible size fits and unrepresentable charges get `TooLarge`; temporary exhaustion gives `Full` for size-eligible requests.
- [ADR 0027](0027-default-admission-budget-is-an-accounting-ceiling.md) sets the initial default without preallocating a budget-sized buffer.
- Encoded-size limits and admission charges are separate. A request that fits one group can still exceed the admission budget.
- The charge is an accounting bound for outstanding requests, not a bound on total resident memory. Caches, shared group-encoding buffers, allocator overhead, opaque standard-library channel internals, and results retained after an answer are outside it.
- Bookkeeping allowances must follow the actual request types. They cannot be arbitrary constants that silently undercount known library-owned storage.
- A request that cannot fit the entire budget gets `TooLarge` under ADR 0028.

## Enforcement

None yet. The M1 retained-allocation cases in [testing.md](../architecture/testing.md#property-and-unit-tests) will vary capacities independently of contents, cover unused record slots and many small records, verify result and bookkeeping allowances, and check that processing never increases the reserved charge or releases it early.

## Revisit if

- The API starts copying inputs into compact library-owned buffers before admission. Then accounting may follow those allocations through a new decision.
