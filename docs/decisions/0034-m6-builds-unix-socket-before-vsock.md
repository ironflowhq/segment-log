---
id: "0034"
title: "M6 builds Unix socket before vsock"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [architecture, transport, milestones]
touches: [docs/prd.md, docs/architecture/overview.md, docs/architecture/testing.md]
enforced_by: none
---

# 0034. M6 builds Unix socket before vsock

## Context

M6 includes a server over Unix socket and vsock, with a Go integration client. PRD open item 5 leaves their implementation order undecided. The production caller's location remains a deployment integration choice.

The owner approved Unix socket first, followed by vsock using the same protocol, during the M1 readiness discussion on 8 November 2026. Both transports remain in M6. This ADR amends the sequencing around ADR 0001 without changing M1's library-only scope.

## Options & trade-offs

### Option A - Unix socket first, then vsock

- **For:** establishes a local Go-client path for validating the server protocol and core behavior before integrating guest transport setup. Both adapters can reuse the same protocol cases.
- **Against:** callers that require vsock wait for the second part of M6. Unix-socket evidence does not verify cross-guest connectivity.

### Option B - vsock first, then Unix socket

- **For:** addresses the cross-guest path first when that is the deployment's primary consumer.
- **Against:** protocol integration and guest transport setup must be exercised together before the local Unix-socket path exists. The actual deployment topology has not been selected by this decision.

## Decision

In M6, implement the Unix-socket server path and Go-client integration first. Then add vsock using the same protocol and core operations. Both transports must be implemented and validated before M6 is complete.

Keep transport adapters at the boundary. Both use the same protocol semantics and delegate to the same core behavior. The wire format remains an M6 decision with its own ADR under ADR 0001. This decision chooses no message encoding, framing or server dependencies.

The implementation order does not select the production transport. Actual caller placement determines which transport a deployment uses. Unix-socket tests cannot substitute for vsock tests against the selected guest topology.

Protocol design and server implementation remain M6 work. The server stays a placeholder through M1–M5, and the core API gains no transport-specific types or behavior.

We accept postponing the vsock integration until the second part of M6 so the local protocol path can be validated first, without removing either transport from the milestone.

## Consequences

- PRD open item 5 is resolved as an implementation sequence.
- M6 needs Go-client append and read evidence over both transports, with shared conformance cases for the protocol's retry, revision and admission outcomes.
- Actual guest integration, storage qualification and surviving-volume handoff remain separate requirements under ADRs 0032 and 0033.
- M1 implementation and its completion gate are unchanged.

## Enforcement

None yet. The M6 transport checks in [testing.md](../architecture/testing.md#m6-transport-order-and-conformance) will record Unix-socket integration first, then vsock integration, and verify both before M6 completion.

## Revisit if

- A concrete consumer requires vsock integration before the local server path and the project explicitly changes the implementation order.
