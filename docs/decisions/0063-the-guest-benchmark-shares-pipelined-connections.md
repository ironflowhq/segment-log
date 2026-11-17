---
id: "0063"
title: "The guest is held to the throughput target, and its callers share pipelined connections"
status: proposed
date: 2026-11-17
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [m7, benchmark, server, vsock]
touches: [scripts/m7-guest-bench.sh, segment-log-server/examples/socket_bench.rs, segment-log-server/src/connection.rs, docs/prd.md, docs/architecture/testing.md]
enforced_by: test
---

# 0063. The guest is held to the throughput target, and its callers share pipelined connections

## Context

The PRD's throughput target is ≥ 50,000 records/s for 256-byte payloads from 1,000 caller threads in a closed loop. M7's "done when" asks the guest benchmark report to measure the targets, not to meet them.

On the qualified host ([ADR 0059](0059-m7-qualifies-firecracker-on-a-dedicated-nvme-host.md)), the guest benchmark did 46,187 records/s ([qualification record](../evidence/m7/qualification.md#guest-benchmark-and-49)). The same workload in process on the same volume did 261,793. The bound is the VMM's main thread, at 99.9 % CPU, which runs Firecracker's vsock muxer. That thread handles every vsock packet of the guest, and there is one of it for each VMM.

The benchmark gave each caller its own connection with one request in flight. So every record cost at least one request packet and one answer packet through the muxer, across 1,000 connections, and the server ran 2,000 threads on 2 vCPUs. Pipelining is already allowed on a connection ([ADR 0057](0057-a-hand-rolled-binary-protocol-answered-in-order.md)), but the server wrote each answer with its own write, so a connection's answers never shared a packet.

One VMM serves one volume, and so one log. Running more guests gives more logs, not a faster log. Firecracker offers no vhost-vsock.

## Options & trade-offs

### Option A - record the miss, and keep the target for the library only

- **For:** no change. The PRD's row describes the in-process workload, and M7 asks only for a measurement.
- **Against:** the PRD's case is agent runs in guests. A target that the deployed shape cannot reach is a promise we do not keep.

### Option B - fewer packets per record: shared, pipelined connections, and a server that sends ready answers together

- **For:** stays on the qualified stack and the wire protocol. Many callers on one connection let one write carry many requests, and one server write carry many answers, so the muxer's cost per record falls. The server needs fewer threads.
- **Against:** the benchmark no longer gives each caller a connection, so its shape must be stated, or the number looks tuned. A client must pool and pipeline to get the rate.

### Option C - another transport: virtio-net and TCP, or shared memory

- **For:** avoids the vsock muxer entirely.
- **Against:** changes the ADR 0059 stack and requalifies it. Adds a device and a network attack surface to the guest. Shared memory needs `unsafe` and a new protocol.

### Option D - more guests or VMMs for one workload

- **For:** each VMM has its own muxer thread.
- **Against:** each VMM serves its own volume, so this gives several logs, not one log at the target.

## Decision

We will hold the guest topology to the PRD's throughput target. We measure it for one guest, one VMM and one log. The guest benchmark's 1,000 callers share C pipelined connections. C is 16, fixed in `scripts/m7-guest-bench.sh` before the bare-metal run from a [sweep in the Lima rehearsal](../evidence/m7/lima-connection-sweep/README.md), and it is not tuned to the bare-metal result. The server sends every answer that is ready, in request order, in one write, and reads requests through a buffer.

Option B keeps the qualified stack and the wire protocol, and cuts the cost on the one thread that bounds the run. We accept on purpose that the benchmark's connection shape differs from one connection per caller. The report states C and keeps a run with one connection per caller beside it as a diagnostic, so nothing is hidden. The target's numbers do not change.

## Consequences

- The PRD's M7 "done when" requires the guest benchmark to meet the throughput target at the stated C. The paced p99 is reported against the latency target. A miss there is its own finding.
- `Pending::try_wait` joins the core API ([api.md](../architecture/api.md)), so the server can send ready answers without blocking on the next one.
- A caller that wants the guest rate must pool connections and pipeline on them. The Go client cannot yet: [#134](https://github.com/ironflowx/segment-log/issues/134) tracks a pooled client.
- On one connection, `PIPELINE_DEPTH` (64) bounds the requests in flight, so C must be at least 1,000 / 64 for all callers to be in flight.

## Enforcement

`segment-log-server/src/connection.rs` tests that 201 answers that are ready together go out in far fewer writes, in request order, and that a burst of requests costs a few reads. `segment-log-core/src/acceptance/delivery.rs` covers `try_wait` in `ACK-DELIVERY`. `scripts/m7-guest-bench.sh` passes `M7_CONNECTIONS` to the benchmark and records it in `environment.txt`.

## Revisit if

- The guest misses the target on bare metal at the fixed C. Then Option C comes back with its own ADR.
- Firecracker gains vhost-vsock or a muxer with more than one thread.
- The production caller's connection shape turns out to differ from a pool, for example one process per caller.
