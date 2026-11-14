# Wire protocol

Status: protocol version 1, M6 ([ADR 0057](../decisions/0057-a-hand-rolled-binary-protocol-answered-in-order.md)). Until the first production release, a change to these bytes increments the version, and a server accepts only its own version.

This file is the only source for the protocol's message layout, widths, status codes and size caps. The server's process behavior is in [ADR 0058](../decisions/0058-one-server-process-serves-one-log.md). The record limits (stream id and key lengths) are in [format.md](format.md#limits) and are not copied here.

## Conventions

- All integers are little-endian and unsigned.
- There is no alignment and no padding, except reserved fields.
- Reserved fields are written as zero. A receiver treats a nonzero reserved field as malformed.
- The same bytes run over a Unix stream socket and a vsock stream socket. The transport is a reliable byte stream, so messages carry no CRC.

## Session

1. The client connects and sends `Hello`. Any other first message is malformed.
2. The server answers `Hello`. If the versions differ, the answer carries `UnsupportedVersion`, and the server closes the connection.
3. The client may then send any number of requests without waiting for answers (pipelining). The server answers each connection's requests in the order it read them, one answer for each request.
4. A malformed message gets a `Malformed` answer, and the server closes the connection. Requests read before it are still answered first, in order.

A connection that closes before a request's answer arrives leaves that request's outcome unknown to the client. It is never a rejection ([`TRANSPORT-UNKNOWN`](testing.md#m6-transport-order-and-conformance)). The client resolves an unknown append by a fully keyed retry ([api.md](api.md#retry-after-an-unknown-outcome)).

## Message header (8 bytes)

Every message, in both directions, starts with the same header.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | `body_len` — the size of the body in bytes. The whole message is `8 + body_len` bytes. |
| 4 | 1 | `kind` — a request kind, or for an answer, the request's kind with the high bit set (`kind | 0x80`) |
| 5 | 3 | reserved |
| 8 | `body_len` | body |

| Request | `kind` | Answer `kind` |
| --- | --- | --- |
| `Hello` | `1` | `0x81` |
| `Append` | `2` | `0x82` |
| `Read` | `3` | `0x83` |
| `Watermark` | `4` | `0x84` |
| `StreamHead` | `5` | `0x85` |
| `Seal` | `6` | `0x86` |

An unknown `kind` is malformed. A `Malformed` answer to an unknown or unreadable request uses kind `0x80`.

## Size cap

`frame_cap = max(2 * group_byte_cap, 65536)`, from the server's [options](api.md#target-signatures-and-defaults). The server sends it in its `Hello` answer. The floor keeps every fixed-size request and the longest stream id within the cap when `group_byte_cap` is small.

- A request body that passes `frame_cap` is never buffered. For `Append`, the server reads and discards its `body_len` bytes in bounded chunks and answers `TooLarge`; the connection stays open. The core would refuse the same submit with `TooLarge`: a wire record entry is at most one byte larger than its group record entry, and each group record entry is at least 17 bytes, so a submit whose wire body passes `frame_cap`, which is at least `2 * group_byte_cap`, has a group body above `group_byte_cap`. For any other kind, a body that passes its fixed size is malformed.
- An answer body is at most `frame_cap`. A `Read` answer holds the longest prefix of the records that fits, and at least one record when any is available, because a single record is at most `group_byte_cap` minus the group body header.

## Answers

Every answer body starts with a status.

| Offset in body | Size | Field |
| --- | --- | --- |
| 0 | 2 | `status` — `0` for success, otherwise a code from the [status table](#status-codes) |
| 2 | … | on success, the answer fields below; otherwise the status's detail fields |

## Requests and successful answers

### Hello

Request body (8 bytes):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | `magic` — the bytes `S G L P` (`0x53 0x47 0x4C 0x50`) |
| 4 | 2 | `version` — `1` |
| 6 | 2 | reserved |

A wrong `magic` is malformed. Answer fields after the status (8 bytes):

| Offset | Size | Field |
| --- | --- | --- |
| 2 | 2 | `version` — the server's version |
| 4 | 4 | `frame_cap` |
| 8 | 2 | reserved |

### Append

Request body: `count` (4 bytes, at least 1 is checked by the core), then `count` record entries one after another. The entries must fill the body exactly.

| Offset in entry | Size | Field |
| --- | --- | --- |
| 0 | 2 | `stream_len` |
| 2 | 2 | `key_len` — 0 means no key |
| 4 | 4 | `payload_len` |
| 8 | 1 | `expected` — `0` `Empty`, `1` `At` |
| 9 | 8 | `revision` — for `At`, its revision; for `Empty`, zero |
| 17 | `stream_len` | stream id |
| … | `key_len` | idempotency key |
| … | `payload_len` | payload |

The server checks only the layout: the entries fill the body, `expected` is `0` or `1`, and `revision` is zero for `Empty`. Field limits, an empty submit and repeated keys are the core's checks, and they answer `Invalid`. The server answers when `Pending::wait` returns. Answer fields after the status:

| Offset | Size | Field |
| --- | --- | --- |
| 2 | 1 | `deduplicated` — `0` or `1` |
| 3 | 3 | reserved |
| 6 | 4 | `count` — one position for each request record, in request order |
| 10 | 16 × `count` | positions: `offset` (8), `revision` (8) |

### Read

Request body (12 bytes): `from` (8), `max` (4). Answer fields after the status: `count` (4), then `count` record entries:

| Offset in entry | Size | Field |
| --- | --- | --- |
| 0 | 8 | `offset` |
| 8 | 8 | `revision` |
| 16 | 2 | `stream_len` |
| 18 | 2 | `key_len` — 0 means no key |
| 20 | 4 | `payload_len` |
| 24 | `stream_len` | stream id |
| … | `key_len` | idempotency key |
| … | `payload_len` | payload |

The records are those of `read(from, max)`, cut to the [size cap](#size-cap). A client reads on from the offset after the last record it got.

### Watermark

Request body: empty. Answer fields after the status: `watermark` (8).

### StreamHead

Request body: the stream id, `body_len` bytes. An empty body or one above the stream id limit gets `Invalid`. Answer fields after the status (24 bytes):

| Offset | Size | Field |
| --- | --- | --- |
| 2 | 1 | `present` — `0` if the stream has no durable record, `1` otherwise |
| 3 | 7 | reserved |
| 10 | 8 | `revision` — zero when not present |
| 18 | 8 | `offset` — zero when not present |

### Seal

Request body: empty. The answer has no fields after the status. After a successful seal answer, the server stops as [ADR 0058](../decisions/0058-one-server-process-serves-one-log.md) describes.

## Status codes

Each core outcome has its own code. Detail fields follow the status in the answer body.

| Code | Name | Core outcome | Detail fields |
| --- | --- | --- | --- |
| 1 | `Full` | `AppendError::Full` | none |
| 2 | `TooLarge` | `AppendError::TooLarge`, or an `Append` body above `frame_cap` | none |
| 3 | `Invalid` | `AppendError::Invalid`, or an invalid `StreamHead` stream id | `reason_len` (2), `reason` (UTF-8) |
| 4 | `Sealed` | `AppendError::Sealed` | none |
| 5 | `Poisoned` | `AppendError::Poisoned`, `SealError::Poisoned` | none |
| 6 | `Exhausted` | `CommitError::Exhausted` | none |
| 7 | `WrongRevision` | `CommitError::WrongRevision` | `index` (4), `has_actual` (1), `actual` (8, zero when `has_actual` is 0) |
| 8 | `KeyConflict` | `CommitError::KeyConflict` | `index` (4) |
| 9 | `PartialRetry` | `CommitError::PartialRetry` | none |
| 10 | `Indeterminate` | `CommitError::Indeterminate` | none |
| 11 | `PoisonedBeforeWrite` | `CommitError::Poisoned` | none |
| 12 | `Corrupt` | `ReadError::Corrupt` | `segment` (8), `position` (8) |
| 13 | `Io` | `ReadError::Io` | `kind_len` (2), `kind` (UTF-8, the `std::io::ErrorKind` name) |
| 14 | `Malformed` | none: the server could not decode the request | none |
| 15 | `UnsupportedVersion` | none: the `Hello` version is not the server's | `version` (2), the server's |

`Indeterminate` is the only code that leaves an append's outcome unknown. Every other code is a known outcome, as in [api.md](api.md#target-signatures-and-defaults). `Io` carries no operating-system message.
