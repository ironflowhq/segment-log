---
id: "0056"
title: "M6 builds and tests the server; guest deployment validation moves to M7"
status: accepted
date: 2026-11-14
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [milestones, server, deployment]
touches: [docs/prd.md, docs/architecture/testing.md, segment-log-server/**]
enforced_by: none
---

# 0056. M6 builds and tests the server; guest deployment validation moves to M7

## Context

M5 passed on 2026-11-13, so M6 is next. The PRD's M6 "done when" check has two kinds of evidence:

- Evidence that this repository can produce: a server over a Unix socket and vsock, a Go test client, and shared conformance cases for retries, revisions and admission outcomes ([ADR 0034](0034-m6-builds-unix-socket-before-vsock.md)).
- Evidence that needs a real guest deployment: a qualification record for the actual storage stack ([ADR 0032](0032-qualify-deployment-storage-before-m6.md), V1–V11 in [volume-requirements.md](../architecture/volume-requirements.md)), the mount paths, reader component and handoff procedure ([ADR 0033](0033-deployment-owns-volumes-and-reader-handoff.md)), vsock evidence from the selected guest topology, and a guest benchmark.

These facts are fixed:

- The guest stack is not identified. Issue #46 is open, and nothing in the repository names the hypervisor, the virtual block device, the backing storage, the filesystem or its mount options.
- The milestone order is a gate. A milestone passes only when its "done when" check passes.
- The two footprint targets in the PRD, idle resident set and binary size, measure the server binary. They do not depend on the storage under it.
- The throughput and latency targets assume local NVMe. A hosted runner is a shared virtual machine, so it cannot show a pass.
- vsock has a host loopback on Linux (`vsock_loopback`), so the shared cases can run over vsock without a guest. [testing.md](../architecture/testing.md#m6-transport-order-and-conformance) already says that loopback does not replace evidence from the guest topology.

## Options & trade-offs

### Option A - keep M6 as one milestone

- **For:** no change to the PRD or to ADRs 0032–0034.
- **Against:** M6 stays open until a deployment exists, however complete the server is. The gate shows nothing about the server's own progress, and no check that someone can run today marks it done.

### Option B - start M6 and leave the gate as written

- **For:** server work starts now with no document change.
- **Against:** the server work would then proceed against a gate that it cannot pass, and the deployment evidence would have no milestone of its own.

### Option C - split M6: the server in M6, the guest deployment in M7

- **For:** each gate can be run when its work is done. M6 is checked from this repository alone. M7 collects every item that needs the guest stack, so the stack question has one place to land.
- **Against:** one more milestone to track. vsock appears in both: loopback evidence in M6, guest-topology evidence in M7. The binding throughput and latency check waits for M7.

## Decision

We will split the current M6 in two (option C).

**M6** builds and tests `segment-log-server`. It passes when:

1. A Go test client appends and reads over a Unix socket, and then over vsock through the Linux host loopback, using one protocol. The protocol has its own ADR.
2. `TRANSPORT-CONFORMANCE`, `TRANSPORT-UNKNOWN` and `TRANSPORT-RESTART` pass over both transports, from conformance cases that both runs share as data.
3. A protocol fuzz target, `FUZZ-PROTOCOL`, runs for one hour in CI with no crash.
4. A Linux CI job meets the idle resident set and binary size targets for the release musl server binary.
5. A report in `docs/` gives append throughput and latency over the Unix socket beside the same workload in process. These numbers are diagnostics under [ADR 0029](0029-m1-completion-requires-correctness-and-measurement-evidence.md): a miss is a reported finding, not a blocker.

**M7** validates the server on the guest deployment. It passes when:

1. The qualification record for the actual guest stack passes every MUST requirement (ADR 0032).
2. The handoff record names the mount paths, the reader component and the procedure, and `HANDOFF` passes (ADR 0033).
3. The shared conformance cases pass over vsock on the selected guest topology (ADR 0034).
4. A guest benchmark report in `docs/` measures the PRD throughput and latency targets on the qualified stack.

M6 can be checked now and M7 later, so the gate shows the server's real progress. The deployment items keep their content: this decision moves when they are checked, not what they require. We accept that the binding throughput and latency results, and guest vsock connectivity, wait for M7.

## Consequences

- Accepted by the owner on 2026-11-17 for the 0.1 release ([issue 133](https://github.com/ironflowx/segment-log/issues/133)).
- Amended by [ADR 0063](0063-the-guest-benchmark-shares-pipelined-connections.md): the guest benchmark must meet the throughput target, not only measure it, with its callers sharing pipelined connections.
- The PRD milestone table gets the M6 and M7 rows above. Open items 3 and 4 and the risk on the storage contract now say "before M7".
- ADR 0032's qualification and guest benchmark, and ADR 0033's handoff record, are required before M7 instead of before M6. Their requirements do not change. ADR 0034's transport order holds within M6; its guest-topology vsock evidence moves to M7.
- Complete-history retention ([ADR 0037](0037-retain-complete-logs-through-current-milestones.md)) covers M1–M7.
- In testing.md, the `M6-DEPLOYMENT` group becomes `M7-DEPLOYMENT`, and `M6-TRANSPORT` gains `FUZZ-PROTOCOL` and the footprint check. The headings for qualification and handoff say M7. Each keeps its old anchor, so existing links still work.
- Server work can start once this ADR and the protocol ADR are written. `segment-log-server` is no longer a placeholder from M6.
- The guest stack (issue #46) blocks M7 only.

## Enforcement

None. The PRD milestone table and the group table in [testing.md](../architecture/testing.md) carry the two gates. A reviewer checks that the M6 completion record claims no M7 item.

## Revisit if

- The guest stack is identified before M6 is done. Then M6 and M7 can run together, but each keeps its own gate.
- A consumer needs the server in production before M7 passes. Production durability claims still need the qualification record.
