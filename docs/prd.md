---
date: 2026-11-07
status: Draft. The owner has not approved it.
---

# Segment log — product requirements

## Summary

The segment log is an embedded, append-only event log, written as a Rust library. Many callers append records to many streams in one directory. One writer thread packs their appends into groups, and one `fsync` makes a whole group durable. The log keeps a revision per stream, checks the caller's expected revision, and removes duplicate retries by an idempotency key. When the work ends, the log seals and the process exits. The sealed files stay on disk as the history.

## Problem

Many workloads now run as short-lived processes in small guests, for example agent runs on microVMs. Each run needs a history that it can resume from and that other systems can read after the guest is gone.

The usual options fail this workload:

- **One file or one database per agent, each calling fsync.** Flush cost grows with the agent count. On one device without a power-loss-protected write cache, tens of thousands of separate flushes per second are not possible.
- **A broker such as Kafka.** Kafka gets its durability from replication. It does not `fsync` on acknowledgment by default (`flush.messages` defaults to never), and it is too large to run inside a throwaway guest.
- **An embedded WAL crate.** No crate that we surveyed has the whole contract: one writer, group commit, a durable commit boundary, many streams in one file, expected revision, idempotency keys, a queue that refuses when full, and seal. See [prior art](architecture/prior-art.md).

## Users

| User | Need |
| --- | --- |
| A guest process (Rust) | Link the library, append from many threads, seal on exit. |
| A caller in another language (first: Go) | Reach the log over a Unix socket or vsock. This is a later milestone. |
| A reader after the guest is gone | Open sealed segments from a surviving volume and read the history in order. |
| An implementer of this repository | A spec where each rule is testable and each open item is named. |

## Goals

1. **Group durability.** One `fsync` covers every submit in a group. The caller gets an acknowledgment only after that `fsync` returns success.
2. **A crash-safe contract that is honest.** An acknowledgment means the group is durable, if the storage meets its [contract](architecture/overview.md#storage-contract). No acknowledgment means the group might be durable. Recovery never loses an acknowledged group, and it never leaves a hole or changes the order.
3. **Safe retries.** A retry that uses the same idempotency key and the same bytes returns the original log offset and revision. It does not append again.
4. **Optimistic concurrency per stream.** An append that has the wrong expected revision fails and uses no log offset.
5. **Small and quick to start and stop.** Open time is proportional to the active segment, not to the full history (after milestone M4). Seal-to-exit time is short. Idle memory is small.
6. **Backpressure, not blocking.** When the accept queue is full, an append fails at once.

## Non-goals

- Replication, consensus, leader election, or a high watermark across replicas.
- Transactions across submits, control batches, or a last stable offset.
- Compaction, tombstones, or key-based retention. An event stream keeps every event.
- Compression, until a measurement shows that batches are large enough to gain from it.
- Subscriber state, consumer groups, leases, or wait matching. A layer above the log owns these.
- An in-process C ABI. The boundary for other languages is a socket.
- Tiered storage, `sendfile` reads, or the Kafka wire protocol.
- An async runtime, SQL, or protobuf on the write path.
- A stable on-disk format before milestone M5.
- Reading one stream from an arbitrary revision through an index. A filtered scan is enough for now.

## Requirements

Each requirement has an ID. Tests and issues refer to these IDs. The architecture docs give the detail.

### Append and durability

| ID | Requirement |
| --- | --- |
| R1 | A submit holds one or more records. It is the atomic unit: the log commits all of its records or none of them. ([ADR 0009](decisions/0009-caller-submit-is-the-atomic-unit.md)) |
| R2 | The writer can pack many submits into one group and one `fsync`. Two submits in the same group are not atomic with each other. |
| R3 | The writer never splits a submit across a linger boundary, a byte cap, or a segment roll. |
| R4 | The writer sends an acknowledgment, and readers see a group, only after the group's `fsync` returns success and the watermark moves past it. ([ADR 0002](decisions/0002-group-commit-and-the-watermark.md)) |
| R5 | When the accept queue is full, `append` returns `Full` at once. It does not block. ([ADR 0010](decisions/0010-full-accept-queue-refuses.md)) |
| R6 | After any I/O error on the write path, the writer is poisoned. Submits that were in flight get `Indeterminate`. Later submits get `Poisoned`. The caller must reopen the log. ([ADR 0004](decisions/0004-io-errors-poison-the-writer.md)) |

### Identity, revisions and retries

| ID | Requirement |
| --- | --- |
| R7 | Each record has a log offset, a file position, and a stream revision. ([ADR 0007](decisions/0007-record-identity.md)) |
| R8 | The log offset is one monotonic index across all segments. A durable offset is never used again. Offsets from a torn tail can be used again. |
| R9 | The first record in a stream has revision 0. |
| R10 | The writer checks the idempotency key before the expected revision. If the key matches with the same bytes, the log returns the original offset and revision. If the key matches with different bytes, the log returns an error. ([ADR 0008](decisions/0008-idempotency-key-before-expected-revision.md)) |
| R11 | A wrong expected revision fails the whole submit, returns the stream's current revision, and uses no log offset. |
| R12 | Expected-revision checks see earlier records in the same submit and in earlier submits that are not yet durable, in the order of admission. Whether key checks see pending submits is open item 8. ([ADR 0009](decisions/0009-caller-submit-is-the-atomic-unit.md)) |
| R13 | An idempotency key lives as long as the segment that holds its record. When that segment is deleted, the key expires. |

### Recovery

| ID | Requirement |
| --- | --- |
| R14 | On open, the log keeps the contiguous valid prefix of the active segment and truncates the rest. That prefix sets the new watermark. Before it publishes the watermark, the log makes the kept prefix durable, including bytes that a crash or a failed `fsync` left readable but not on the device. ([ADR 0003](decisions/0003-recovery-keeps-the-checksum-valid-prefix.md), [ADR 0015](decisions/0015-open-makes-the-recovered-tail-durable.md)) |
| R15 | Recovery never truncates a sealed segment. A bad frame in a sealed segment is a corruption error. A gap between segments is an error. |
| R16 | An OS lock on the directory's `LOCK` file blocks a second live writer. A writer epoch, stored in each frame inside the span that the CRC covers, lets recovery reject frames from an older writer. ([ADR 0018](decisions/0018-writer-epoch-from-the-segments.md)) |
| R17 | Each segment header holds a log id, so that segments from two logs that both start at offset 0 do not collide. |

### Read

| ID | Requirement |
| --- | --- |
| R18 | A reader can read all records in order from a stored log offset, up to the watermark. |
| R19 | A reader can get the latest revision and log offset of one stream. |
| R23 | A reader can open a sealed log without a lock and without writing to the volume. (M4, [ADR 0016](decisions/0016-read-only-reader-without-a-lock.md)) |

### Seal and retention

| ID | Requirement |
| --- | --- |
| R20 | `seal` stops admission, commits the last submits, writes the seal frame, makes it durable, and closes the log. A sealed log opens read-only. ([ADR 0011](decisions/0011-sealed-footer-and-no-shutdown-marker.md)) |
| R21 | Each sealed segment has a footer with one entry for each stream in it: stream id, last revision, last log offset, last file position. A locator at the end of the segment points to the footer. Open reads locators and footers, and scans only the active segment. (M4, [ADR 0017](decisions/0017-footer-locator-offset-and-size.md)) |
| R22 | Deferred. Retention is open item 11. |

## Footprint and performance targets

These numbers are proposals for the owner to confirm. The source design left them unset. The M1 benchmark checks them. A target that the benchmark misses is a finding, not a reason to quietly change the number.

| Metric | Proposed target | Conditions |
| --- | --- | --- |
| Append throughput | ≥ 50,000 records/s | 256-byte payloads, 1,000 caller threads, 1 ms linger, local NVMe, Linux |
| Append latency, p99 | ≤ linger + 2 × the device's `fsync` p99 | Same |
| Open time | ≤ 50 ms | 4 MiB active segment, 100 sealed segments with footers (M4 onward) |
| Seal-to-exit | ≤ 20 ms from the `seal` call to process exit | Empty accept queue, footer for 10,000 streams |
| Idle resident set | ≤ 8 MiB | Server binary, empty log, after open (M6) |
| Binary size | ≤ 4 MiB stripped | Server binary, musl, release (M6) |

Measure from the defaults in [api.md](architecture/api.md).

### Reference points

No published number matches this contract (an `fsync` before each acknowledgment, many streams, expected revision). See [performance reference points](architecture/prior-art.md#performance-reference-points).

- **Throughput.** Systems that `fsync` on every write report hundreds to about 10,000 writes/s. Systems that do not `fsync` on acknowledgment report 30,000 to 300,000+ records/s. The 50,000 records/s target sits between the two groups. It is about 12.8 MB/s, which is small in bytes. Group commit is what makes it reachable.
- **Latency.** The bound is relative to the device, so no other system's number applies.
- **Open time, seal-to-exit, idle resident set, binary size.** No comparable system publishes these. The M1 and M6 benchmarks are the only evidence.
- Do not read a Kafka or default NATS number as a target. Neither `fsync`s on acknowledgment.

## Milestones

Each milestone ends with a "done when" check that someone can run. A later milestone does not start until the gate for the earlier one passes.

| # | Scope | Done when |
| --- | --- | --- |
| M0 | Repository ready: edition 2024, pinned toolchain, CI, docs. | CI is green on Linux and macOS. |
| M1 | `open`, `append` with group commit, `read`, reopen with recovery, poison on I/O error. Submits with one or more records (R1). Expected revision. A minimal key map: a submit whose keys all match durable records with the same bytes returns the originals. Simulated storage and a crash harness. Provisional v0 format. One segment, no roll. | The [crash tests](architecture/testing.md) pass: simulated power loss, simulated `kill -9`, a real-process `SIGKILL` test, and `fsync` fault injection. Reopen keeps every acknowledged record. Retrying the unacknowledged keys appends each record once. |
| M2 | The full key rules: `KeyConflict`, `PartialRetry`, and a decision on open items 8 and 9. | Property tests for R10–R12 pass against the model. A rejected revision uses no offset. |
| M3 | Segment roll. | No submit crosses a roll in the crash tests. A crash at each step of a roll recovers. |
| M4 | Sealed footer and locator, `seal`, read-only open, `LogReader`. Resolve the [key-lifetime gap](#open-items). | Open time for M4 conditions meets the target. From each sealed segment except the newest, open reads only the header, the locator and the footer, counted through the storage trait. |
| M5 | Freeze the byte layout. Fuzz the recovery parser. | A format ADR is Accepted. Fuzzing runs for 1 hour in CI with no crash. |
| M6 | `segment-log-server` over a Unix socket and vsock. Benchmark against the targets. | A Go test client appends and reads over the socket. The benchmark report is in `docs/`. |

`segment-log-server` stays a placeholder until M6 ([ADR 0001](decisions/0001-rust-library-first-socket-boundary-later.md)).

## Open items

These items are not decided. Each one names the milestone that it blocks.

1. **Key lifetime after footers (blocks M4).** Three rules conflict. A key lives as long as its segment (R13). Footers do not copy keys. Open scans only the active segment (R21). After M4, nothing finds a key that is in a sealed segment, so a retry after a roll can append again. One option: a per-segment key index file. It is a cache that the log can rebuild from the segment, and open loads it for each retained segment. The other option: accept that a key lives only while its segment is active, and change R13. Whichever option wins must also bound the key map's memory. At 256-byte payloads, a segment of the default roll size holds about 200,000 records, so keeping every key of every retained segment can cost tens of MB of RAM for each segment (an estimate). The PRD has no target for that memory yet.
2. **Expected-revision vocabulary (provisional now; settle by M2).** The source design adopted no name for "this stream has no records". A first append needs one. [ADR 0008](decisions/0008-idempotency-key-before-expected-revision.md) proposes `Empty` and `At(n)`, with no `Any`.
3. **Does the guest's storage meet the [storage contract](architecture/overview.md#storage-contract)?** The device must honor the flush and write each sector atomically, and the filesystem must stop accepting writes after a metadata write error. The library cannot check any of this. The deployment must check all three for each hypervisor, disk and filesystem setup, before the M6 benchmark.
4. **Where the surviving volume is mounted, and who reads it after the guest is gone (before M6).**
5. **Unix socket or vsock first (M6).** This depends on whether the caller is on the same host or across the microVM boundary.
6. **A waiting cursor.** A primitive that waits for the watermark to move. It is not decided whether the library or the layer above owns it.
7. **Linger, byte cap and roll size.** The defaults change when a measurement says so.
8. **A retry while the original is still pending (blocks M2).** A retry can arrive while the first attempt is in a group that is not yet durable. Its keys do not match a durable record, so it gets the normal revision check, which usually fails. Option: make it wait for the earlier group and then answer as a retry.
9. **Key scope, and the same key twice (blocks M2).** Nothing defines whether a key is scoped to one stream or to the whole log. Nothing defines the same key twice in one submit, or in two submits that are pending at the same time. If keys are scoped to the log, two pending submits can commit the same key on two streams, because key checks see only durable records. Options: reject a repeat inside one submit with `Invalid`, reject a key that a pending submit already holds, and pick the scope in [ADR 0008](decisions/0008-idempotency-key-before-expected-revision.md).
10. **Queued bytes (settle before the M1 benchmark).** The accept queue limits submits, not bytes. At the default queue capacity and byte cap, the queue can hold 4 GiB. Options: a byte limit on the queue, or a smaller default capacity.
11. **Retention (after M4).** The rule from the source design deletes a segment only after a later footer covers every stream in it, so that revisions do not restart at 0. A stream that ends never appears in a later footer. With one stream for each agent run, almost no segment becomes deletable. Options: footers that carry every live stream, or no retention, because a sealed log is kept whole as history.

## Risks

| Risk | Mitigation |
| --- | --- |
| The device, the hypervisor or the filesystem breaks the storage contract (drops flushes, tears a sector, or keeps writing after a metadata error), so an acknowledgment is not durable. | Open item 3. The [storage contract](architecture/overview.md#storage-contract) states the limit. Test on the real guest before M6. |
| The disk, not the CPU, limits the agent count, and group commit does not change that. | The M1 benchmark measures `fsync` p99 and group size together. |
| `fsync` error handling differs between kernels and filesystems. A failed `fsync` can leave bytes readable that are not on the device. | Poison on error ([ADR 0004](decisions/0004-io-errors-poison-the-writer.md)). Never retry `fsync` to get a success. Rewrite and sync the tail at open ([ADR 0015](decisions/0015-open-makes-the-recovered-tail-durable.md)). |
| The format is frozen too early. | The format stays provisional until M5, and the version field allows a clean break. |
| A crash test that passes but does not model the real fault. | The simulated storage drops and tears unsynced bytes. A real-process `SIGKILL` test and an optional LazyFS job check it on Linux. |

## Documents

- [Architecture overview](architecture/overview.md): placement, components, invariants, commit protocol, recovery.
- [Byte layout v0](architecture/format.md).
- [API sketch](architecture/api.md).
- [Testing and benchmarks](architecture/testing.md).
- [Prior art](architecture/prior-art.md): Kafka, the crate survey, and the evidence limits.
- [Glossary](glossary.md).
- [Decisions](decisions/README.md).
