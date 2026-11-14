---
id: "0058"
title: "One server process serves one log, with blocking threads and libc as its only dependency"
status: proposed
date: 2026-11-14
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [server, dependencies, process]
touches: [segment-log-server/**, clients/go/**, conformance/**, .github/workflows/**]
enforced_by: none
---

# 0058. One server process serves one log, with blocking threads and libc as its only dependency

## Context

[ADR 0056](0056-m6-builds-the-server-m7-validates-the-guest-deployment.md) starts M6, and [ADR 0057](0057-a-hand-rolled-binary-protocol-answered-in-order.md) sets the wire protocol. The server's process model is still open: how many logs it serves, how it handles connections, which crates it uses, and how it starts and stops.

These facts are fixed:

- [ADR 0001](0001-rust-library-first-socket-boundary-later.md): no async runtime and no protobuf on the write path. A guest can be the Rust server alone.
- [ADR 0012](0012-no-log-crate-dependency.md) limits `segment-log-core` to `std`. It says nothing about the server crate.
- `std` has Unix stream sockets but no `AF_VSOCK`, no signal handling and no way to set a socket file's mode at bind. vsock exists only on Linux.
- The PRD targets a stripped musl binary of at most 4 MiB and an idle resident set of at most 8 MiB. Seal-to-exit is at most 20 ms.
- [ADR 0004](0004-io-errors-poison-the-writer.md): a poisoned writer cannot recover in place. Only a new `open` recovers the log.
- [ADR 0033](0033-deployment-owns-volumes-and-reader-handoff.md): the deployment supplies the directory and chooses when to seal. The core adds no automatic sealing.
- After `SIGKILL`, the Unix socket file stays on disk, and `TRANSPORT-RESTART` restarts the server on the same path.
- The first consumer is in Go and builds without cgo ([ADR 0001](0001-rust-library-first-socket-boundary-later.md)).

## Options & trade-offs

### Option A - one process, one log, blocking threads, `libc` only

- **For:** the smallest server that meets the protocol. The seal-to-exit target measures a process that exits. One directory means no names or routing in the protocol. `libc` covers vsock, signals and socket permissions, and is already in every Rust binary's link.
- **Against:** a host with many logs runs many processes. Each connection costs two threads.

### Option B - one process serving many logs

- **For:** one process per host.
- **Against:** the protocol needs log names and routing, and each log needs its own life cycle inside the server. Seal no longer means exit. No deployment needs it: each guest has one log.

### Option C - an async runtime (tokio) or other crates for sockets and signals

- **For:** few threads for many connections. Ready-made vsock and signal handling.
- **Against:** against ADR 0001. A larger binary and resident set, close to the footprint targets. `Pending::wait` blocks anyway, so the runtime would still need blocking threads.

## Decision

We will run one server process for one log directory, with blocking threads, and `libc` as the server's only runtime dependency (option A). Any other runtime dependency in `segment-log-server` needs its own ADR.

- **Command line.** `segment-log-server --dir <path>` and exactly one of `--unix <socket-path>` or `--vsock-port <port>`. Every `Options` field has a flag, with the defaults from [api.md](../architecture/api.md#target-signatures-and-defaults). `--max-connections` caps open connections, with a default of 64. A connection over the cap is closed at accept. There is no configuration file. Diagnostics go to stderr as plain lines, without a logging crate.
- **Start.** The server opens the log first. The log's `LOCK` makes it the only writer for that directory. Only then does it remove a stale socket file at the `--unix` path, if that path is a socket, and bind. It creates the socket with mode `0600`.
- **Connections.** Each connection has two threads. One reads requests and calls the core. The other waits for each answer in request order and writes it ([ADR 0057](0057-a-hand-rolled-binary-protocol-answered-in-order.md)).
- **Trust.** The protocol has no authentication. Anyone who can reach the socket can append, read and seal. The Unix socket's mode and directory permissions, or the deployment's vsock topology, decide who can reach it.
- **Seal.** A successful `Seal` answer is written first. Then the server stops reading requests on every connection, answers the requests it has read, closes every connection and exits with status 0. An `Append` that arrives after the seal starts gets `Sealed`.
- **Poison.** When the core answers `Poisoned`, `PoisonedBeforeWrite` or `Indeterminate`, the server stops reading requests, answers the requests it has read, closes every connection and exits with status 2. The supervisor restarts it, and the new `open` recovers the log.
- **`SIGTERM` and `SIGINT`.** The server stops reading requests, answers the requests it has read, closes every connection, drops the log without sealing and exits with status 0. A signal never seals: sealing stays the caller's decision (ADR 0033).
- **Exit status.** 0 after a seal or a signal. 1 when the log does not open or a flag is invalid, with the reason on stderr. 2 after a poisoned writer.
- **Platforms.** The Unix-socket server builds and runs on Linux and macOS. The vsock adapter compiles only on Linux.
- **Go client and cases.** The Go client is a test harness in `clients/go/`, with its own module and a pinned Go version. It makes no promise of a stable API. The conformance cases are JSON files in `conformance/`, read by the Go client. The Rust server never reads them.

We accept a process per log and two threads per connection, in exchange for a small server with nothing but `std` and `libc` in it.

## Consequences

- `segment-log-server` gains a `libc` dependency, and `unsafe` for the vsock, signal and socket calls that `libc` exposes. Each `unsafe` block states the invariant it relies on. CLAUDE.md's "no `unsafe` without an ADR" rule is met for those calls only.
- Idle memory includes two thread stacks for each open connection. The footprint target measures an empty log with no connections.
- A supervisor must restart the server after exit status 2. Requests that were unanswered when it exited are unknown outcomes for their clients.
- A caller that seals ends the service for every other caller on that log. The deployment controls who can reach the socket.
- CI gains a Go toolchain job with the first conformance case ([acceptance-tests.md](../acceptance-tests.md#m6-transport-scenarios)).
- The vsock loopback cases need the `vsock_loopback` module on the Linux runner. If a runner lacks it, the record says so; the cases are not skipped silently.

## Enforcement

None yet. The server's `Cargo.toml` will show its dependencies, and a reviewer checks that any addition has an ADR. `TRANSPORT-RESTART` will exercise the stale-socket start and the poison exit.

## Revisit if

- A deployment needs one process to serve many logs.
- The two threads for each connection show up in the footprint or in a measurement at the planned connection count.
- A transport that crosses hosts is added, which needs authentication.
