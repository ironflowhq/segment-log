---
id: "0060"
title: "The handoff reader is the same server with --read-only, not a separate tool"
status: proposed
date: 2026-11-15
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [server, deployment, m7, handoff]
touches: [segment-log-server/**, clients/go/handoff_test.go, scripts/m7/**, docs/architecture/protocol.md]
enforced_by: test
---

# 0060. The handoff reader is the same server with --read-only, not a separate tool

## Context

`HANDOFF` ([testing.md](../architecture/testing.md#surviving-volume-handoff-before-m6)) needs a component that reads acknowledged records from the surviving volume after the writer's guest is gone. [ADR 0033](0033-deployment-owns-volumes-and-reader-handoff.md) leaves the choice of that component to the deployment. [ADR 0059](0059-m7-qualifies-firecracker-on-a-dedicated-nvme-host.md) chose the stack: one Firecracker VMM per volume, with the caller on the host over vsock.

These facts are fixed:

- `LogReader` reads a sealed log, takes no lock and writes nothing ([api.md](../architecture/api.md)). `SegmentLog::open` can write even on a sealed log: it rewrites the tail in open step 8.
- `segment-log-server` opens only `SegmentLog` ([ADR 0058](0058-one-server-process-serves-one-log.md)), so nothing in the repository reads a log through `LogReader` from outside the process.
- An unsealed crash log needs writable recovery through `SegmentLog::open`. Sealing it afterwards is the caller's decision.
- Consumers already speak the protocol in [protocol.md](../architecture/protocol.md), through the Go client.

## Options & trade-offs

### Option A - `segment-log-server --read-only`

- **For:** one binary and one protocol for writers and readers. A reader guest runs the same image with one more flag, so the rehearsal and the qualified stack test the same reader. The `Sealed` status already says "this log takes no appends".
- **Against:** a reader takes no `LOCK`, so the server cannot rely on the lock to know that a socket file is stale. Seal has to mean something on a log that is already sealed.

### Option B - a separate offline tool that dumps records

- **For:** simple, and it fits beside the operator repair tool in issue #1.
- **Against:** consumers cannot reach it over the socket, it brings a second output format, and the guest would need a second program.

### Option C - both

- **For:** covers consumers and operators.
- **Against:** two readers to build and keep in step before either one is needed twice.

## Decision

We will add `--read-only` to `segment-log-server` (option A).

- **Open.** With `--read-only` the server opens the directory with `LogReader`. A log with no seal frame gives `NotSealed`: the server exits 1 and prints the reason, as for any failed open. It never recovers the log, because recovery writes.
- **Requests.** `Read`, `Watermark` and `StreamHead` work as on the writer. `Append` gets `Sealed`. `Seal` succeeds, because the log is already sealed, as a second seal call on a writer finds it. The reader keeps serving afterwards, so no client can stop it. It stops on `SIGTERM` or `SIGINT`, exit status 0. Writer flags are accepted and have no effect.
- **Socket.** A reader holds no lock, so it removes an existing socket file at `--unix` only when nothing answers on it. A live socket makes the bind fail, exit status 1.
- **Unsealed logs.** The handoff for a crashed guest takes three steps: a writable server recovers the log, the deployment's policy seals it, and then a reader serves it. The seal stays explicit, as ADR 0033 requires.

Option A beat a separate tool because the deployment then needs one binary, one protocol and one client, on both the writer and the reader side. We accept that a reader cannot use the lock to tell a stale socket from a live one, and decide that from whether the socket answers.

## Consequences

- ADR 0058's command line gains `--read-only`. Its exit statuses do not change.
- protocol.md's Seal section notes the reader's answer. The bytes do not change.
- On the ADR 0059 stack, `scripts/m7/fc-server` attaches the volume read-only for a reader, and the guest mounts it `ro`. So the reader cannot write the volume even through a bug.
- `HANDOFF` runs as `TestHandoffSealed` and `TestHandoffUnsealed` over every transport. `TestReadOnlyRefusesAnUnsealedLog` checks the exit status.
- The offline tool of option B stays possible for issue #1, and it is not needed for M7.

## Enforcement

`clients/go/handoff_test.go`. CI runs it over the Unix socket and the vsock loopback, and `scripts/m7-rehearse.sh` runs it over Firecracker guests.

## Revisit if

- A consumer needs to read an unsealed log while its writer runs. That needs a live-reader design, not this flag.
- Two readers have to share one socket path.
