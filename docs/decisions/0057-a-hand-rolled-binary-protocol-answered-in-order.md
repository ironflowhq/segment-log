---
id: "0057"
title: "A hand-rolled binary protocol, pipelined and answered in order on each connection"
status: proposed
date: 2026-11-14
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [server, protocol]
touches: [docs/architecture/protocol.md, segment-log-server/**, clients/go/**, conformance/**]
enforced_by: none
---

# 0057. A hand-rolled binary protocol, pipelined and answered in order on each connection

## Context

[ADR 0001](0001-rust-library-first-socket-boundary-later.md) leaves the wire format to its own M6 ADR, and [ADR 0034](0034-m6-builds-unix-socket-before-vsock.md) requires one protocol for the Unix socket and vsock. [ADR 0056](0056-m6-builds-the-server-m7-validates-the-guest-deployment.md) adds a protocol fuzz target to the M6 gate.

These facts are fixed:

- The write path has no async runtime and no protobuf (ADR 0001). The server's only runtime dependency is `libc` ([ADR 0058](0058-one-server-process-serves-one-log.md)).
- Group commit pays off only when many appends are in flight. One caller with one blocking request at a time gets one record per `fsync`.
- `Pending::wait` blocks the calling thread. The core has no completion callback and no way to wait on several results at once.
- The server binary must stay within 4 MiB stripped, and its idle resident set within 8 MiB (PRD targets).
- Payloads, stream ids and keys are arbitrary bytes.
- Every core outcome must stay distinct on the wire. In particular, a known rejection must never look like an unknown outcome ([testing.md](../architecture/testing.md#m6-transport-order-and-conformance)).
- Callers can reach the socket only if the deployment lets them ([ADR 0058](0058-one-server-process-serves-one-log.md)). The bytes they send are still untrusted input: a bad length must not make the server allocate without bound.

## Options & trade-offs

### Encoding

#### Option A - hand-rolled, length-prefixed binary

- **For:** no dependency. Fixed-width fields and length-prefixed bytes match format.md's style, and a decoder of the same style already passed fuzzing. Small code, small binary.
- **Against:** every field width is ours to document and test. Adding a field means a version change.

#### Option B - a self-describing format (CBOR, MessagePack)

- **For:** fields can be added without a version change. Libraries exist in Go.
- **Against:** a Rust dependency on the server, against ADR 0058. Its decoder is a larger fuzz surface. Before production, we do not need to add fields without a version change (as with [ADR 0053](0053-format-changes-need-no-older-reader-before-production.md)).

#### Option C - a text protocol

- **For:** readable in a packet capture.
- **Against:** binary payloads need escaping or base64, which costs bytes and CPU on the hot path.

### Concurrency on a connection

#### Option D - one request at a time on each connection

- **For:** the simplest client and server.
- **Against:** a caller needs a connection, and the server a thread pair, for every request in flight. Many in-flight appends need many connections.

#### Option E - pipelining, answered in request order

- **For:** many appends in flight on one connection, with no request ids. The server needs one thread to read requests and one to wait on their `Pending`s in order.
- **Against:** one slow answer delays the later answers on its connection. Only a matched retry that waits for an older group, or a group that waits for linger, is slow, and the later answers would mostly wait for the same `fsync` anyway.

#### Option F - pipelining with request ids, answered as they finish

- **For:** no head-of-line delay.
- **Against:** the server needs a thread for each waiting request or a completion mechanism the core does not have. The client must match ids.

### Size cap

#### Option G - a constant cap, the format's frame body limit (16 MiB)

- **For:** one number, the same for every server.
- **Against:** each connection can hold a 16 MiB buffer before admission charges it. With the default 1 MiB `group_byte_cap`, almost all of that is useless.

#### Option H - a cap derived from `group_byte_cap`

- **For:** the buffer follows the server's own group size. The server can prove that a larger append would be refused anyway.
- **Against:** the client learns the cap at `Hello`, not from a constant.

## Decision

We will use a hand-rolled, length-prefixed, little-endian binary protocol (option A), pipelined and answered in order on each connection (option E), with a cap on every message body of `2 * group_byte_cap`, and at least 64 KiB so that a small `group_byte_cap` still admits the longest stream id (option H). [protocol.md](../architecture/protocol.md) is the only source for its bytes.

- **Session.** The client sends `Hello` with a magic and a protocol version. The server accepts one version. Until the first production release, a change to the bytes increments the version and keeps no older one, as ADR 0053 does for the format.
- **Operations.** `Append`, `Read`, `Watermark`, `StreamHead` and `Seal`, mirroring `SegmentLog`. No `LogReader` mode, no subscription or waiting cursor ([ADR 0035](0035-waiting-cursors-belong-above-the-core.md)), no statistics.
- **Outcomes.** Every variant of `AppendError`, `CommitError`, `ReadError` and `SealError` has its own status code, with its detail fields. `Io` carries only its `ErrorKind` name. The server invents no outcome of its own, such as a timeout. A connection that closes before an answer leaves that request's outcome unknown; the client resolves it by a fully keyed retry.
- **Size.** A message body above the cap is never buffered. An oversized `Append` is read and discarded and answered `TooLarge`, which is what the core would answer. An oversized message of any other kind is malformed. A `Read` answer holds the longest prefix of records that fits the cap.
- **Malformed input.** The server answers `Malformed` and closes the connection after it answers the requests it read before.
- **No CRC.** Unix and vsock stream sockets are reliable byte streams inside one host. A CRC would cost time on every message and protect against nothing those transports do.

This keeps the server without dependencies, gives group commit many appends from few connections, and keeps a known rejection apart from an unknown outcome. We accept head-of-line delay within a connection; a caller that cannot accept it opens more connections.

## Consequences

- [ADR 0064](0064-the-0-1-compatibility-promise.md) (Proposed) replaces "the server accepts one version" with range negotiation in protocol version 2, and from 0.1 the server speaks every version from 2 up to its own.
- The server needs two blocking threads for each connection, and no async runtime.
- The Go client must handle pipelining, in-order answers and the `Hello` cap. It also treats a closed connection as unknown, never as a rejection.
- `FUZZ-PROTOCOL` fuzzes the decoder in protocol.md ([testing.md](../architecture/testing.md#fuzzing-m5)).
- The conformance cases in `conformance/` name status codes from protocol.md.
- A change to any byte in protocol.md increments the protocol version, and the server and client change together.
- The vsock path uses the same bytes. It needs no protocol change.

## Enforcement

None yet. `FUZZ-PROTOCOL` will check that a decoded request encodes back to the same bytes, and the conformance cases will check every status code over both transports. A golden test of protocol bytes, like `FORMAT-GOLDEN`, would stop silent changes to the layout.

## Revisit if

- A measurement shows head-of-line delay on one connection that more connections cannot avoid.
- A consumer needs an operation this protocol leaves out.
- A transport other than a local stream socket is added, for example TCP across hosts. That would need authentication, and possibly a CRC.
