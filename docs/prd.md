---
date: 2026-11-08
status: Draft. M1 clarification decisions accepted. Later-milestone implementation details remain open.
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
- Compaction, tombstones, segment-level retention, or key-based retention in M1–M6. Logs keep complete valid histories; deployment owns whole-log archival and expiry ([ADR 0037](decisions/0037-retain-complete-logs-through-current-milestones.md)).
- Compression, until a measurement shows that batches are large enough to gain from it.
- Subscriber state, consumer groups, leases, wait matching, or waiting cursors inside the core. A layer above the log owns these ([ADR 0035](decisions/0035-waiting-cursors-belong-above-the-core.md)).
- An in-process C ABI. The boundary for other languages is a socket.
- Volume provisioning and mount orchestration inside the core. The deployment layer owns the volume lifecycle and reader handoff ([ADR 0033](decisions/0033-deployment-owns-volumes-and-reader-handoff.md)).
- Tiered storage, `sendfile` reads, or the Kafka wire protocol.
- An async runtime, SQL, or protobuf on the write path.
- A stable on-disk format before milestone M5.
- Reading one stream from an arbitrary revision through an index. A filtered scan is enough for now.

## Requirements

Each requirement has an ID. Tests and issues refer to these IDs. The architecture docs give the detail.

### Append and durability

| ID | Requirement |
| --- | --- |
| R1 | A submit holds one or more records. For new records, it is the atomic unit: the log commits all of them or none. A fully deduplicated retry appends nothing and confirms that its referenced records are durable. It does not assert that they originally committed atomically together. ([ADR 0009](decisions/0009-caller-submit-is-the-atomic-unit.md), [ADR 0024](decisions/0024-fully-matching-retries-ignore-original-grouping.md)) |
| R2 | The writer can pack many submits into one group and one `fsync`. Two submits in the same group are not atomic with each other. |
| R3 | The writer never splits a submit across a linger boundary, a byte cap, or a segment roll. |
| R4 | The writer sends an acknowledgment, and readers see a group, only after the group's `fsync` returns success and the watermark moves past it. ([ADR 0002](decisions/0002-group-commit-and-the-watermark.md)) |
| R5 | A size-eligible submit gets `Full` immediately if current queue capacity or outstanding byte charges prevent acceptance. A submit exceeding the group cap or total byte budget, or whose charge calculation overflows, gets `TooLarge` before acceptance. Neither refusal retains a reservation. Each accepted request holds its byte reservation until it is answered, including time in the queue, writer group or as a matched retry waiter. ([ADR 0010](decisions/0010-full-accept-queue-refuses.md), [ADR 0025](decisions/0025-admission-byte-reservations-live-until-the-answer.md), [ADR 0028](decisions/0028-too-large-is-distinct-from-temporary-full.md)) |
| R6 | After any I/O error on the write path, the writer is poisoned. Submits that were in flight get `Indeterminate`. Later submits get `Poisoned`. The caller must reopen the log. ([ADR 0004](decisions/0004-io-errors-poison-the-writer.md)) |

### Identity, revisions and retries

| ID | Requirement |
| --- | --- |
| R7 | Each record has a log offset, a file position, and a stream revision. ([ADR 0007](decisions/0007-record-identity.md)) |
| R8 | The log offset is one monotonic index across all segments. A durable offset is never used again. Offsets from a torn tail can be used again. |
| R9 | The first record in a stream has revision 0. |
| R10 | An idempotency key is scoped to the whole log. A submit cannot repeat a key; `append` rejects it with `Invalid` before admission. The writer checks keys before expected revisions. If every record has a key and matches an existing record with the same stream and payload, the retry returns original positions in request order after all referenced records are durable, regardless of original grouping. If a pending or durable key matches with a different stream or payload, the whole submit fails with `KeyConflict` at its first conflicting record. Otherwise, mixing matching existing records with new or unkeyed records fails the whole submit with `PartialRetry`. Neither rejection consumes an offset or revision. ([ADR 0008](decisions/0008-idempotency-key-before-expected-revision.md), [ADR 0019](decisions/0019-idempotency-keys-are-scoped-to-the-whole-log.md), [ADR 0020](decisions/0020-reject-repeated-keys-within-a-submit.md), [ADR 0022](decisions/0022-reject-conflicting-key-reuse-from-m1.md), [ADR 0023](decisions/0023-reject-mixed-reused-and-new-records-from-m1.md), [ADR 0024](decisions/0024-fully-matching-retries-ignore-original-grouping.md)) |
| R11 | Each new record uses `Empty` or `At(n)` as its expected revision, without unconditional `Any`. A mismatch rejects the whole submit with `WrongRevision`, the failing record's index, and the current revision or `None` for a stream with no records. It consumes no offset or revision and leaves no pending changes. ([ADR 0030](decisions/0030-expected-revisions-use-empty-and-at.md)) |
| R12 | Expected-revision checks see earlier records in the same submit and in earlier submits that are not yet durable, in order of admission. Key checks also see earlier valid pending submits. Fully matching retries skip expected revisions, wait for all referenced records to become durable, and append nothing. ([ADR 0009](decisions/0009-caller-submit-is-the-atomic-unit.md), [ADR 0021](decisions/0021-exact-pending-retries-wait-for-the-original.md), [ADR 0024](decisions/0024-fully-matching-retries-ignore-original-grouping.md)) |
| R13 | An idempotency key remains valid while the segment holding its record is retained, including after roll and reopen. Rolling or sealing does not expire it. Segment deletion expires the key. M4 must settle optimized key lookup and its key-cache memory budget without shortening this lifetime. ([ADR 0031](decisions/0031-keys-live-with-retained-segments.md)) |

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
| R22 | M1–M6 retain complete valid history without segment-level retention or automatic deletion of valid history segments. Deployment owns whole-log archival and expiry. Existing recovery cleanup is unchanged. Any future bounded-history retention needs a separate decision. ([ADR 0037](decisions/0037-retain-complete-logs-through-current-milestones.md)) |

## Footprint and performance targets

These numbers remain provisional. M1 measures the applicable core targets as diagnostics under [ADR 0029](decisions/0029-m1-completion-requires-correctness-and-measurement-evidence.md). A measured miss is a reported finding, not an M1 completion blocker or a reason to quietly change the target. Missing required evidence still blocks M1. Targets for later features wait for their milestone, and existing later-milestone gates remain unchanged.

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

## M1 implementation readiness

The M1 product decisions are settled in the accepted clarification ADRs linked from the requirements and open-item dispositions below. They cover revisions, retries, admission charges and limits, initial defaults, completion evidence, and boundaries around later features. Plan implementation against the [M1 API subset](architecture/api.md#milestone-availability) and the single-segment provisional format.

The follow-up specification review is reflected in the [retry oracle cases](architecture/testing.md#retry-cases-required-from-m1), writer lookup-failure outcomes, answer timing and milestone labels. [ADR 0038](decisions/0038-m1-admission-and-recovery-boundaries.md) proposes boundary choices for admission order, option validation, supported layouts and directory creation. It is the working direction under the repository's ADR rules.

M1 completion requires the [evidence gate](architecture/testing.md#m1-completion-gate); accepted design decisions do not establish test or benchmark results. The remaining M4 lookup and key-cache memory design, M6 wire protocol, and concrete deployment qualification, paths and reader integration are explicit later-milestone dependencies. They do not block M1 implementation.

## Milestones

Each milestone ends with a "done when" check that someone can run. A later milestone does not start until the gate for the earlier one passes.

Implementation follows the [acceptance-test workflow](acceptance-tests.md) from M1. Its scenarios add to each gate below and replace none of its evidence.

M0 passed on 2026-11-08. [CI run 42447941878](https://github.com/ironflowhq/segment-log/actions/runs/42447941878) is green on Ubuntu and macOS at `eb5af03`.

| # | Scope | Done when |
| --- | --- | --- |
| ✅ M0 | Repository ready: edition 2024, pinned toolchain, CI, docs. | CI is green on Linux and macOS. |
| M1 | `open`, `append` with group commit, `read`, reopen with recovery, poison on I/O error. Submits with one or more records (R1). Expected revision. Reject repeated keys within a submit before admission. Fully matching retries against pending or durable records wait for durability and return original positions without another append, regardless of original grouping. Reject conflicting key reuse with `KeyConflict` and mixed reused-and-new submits with `PartialRetry`, against pending and durable records. Simulated storage and a crash harness. Provisional v0 format. An admission byte budget uses retained-allocation charges and covers queued, grouped and matched-retry requests until they are answered. One segment, no roll. | The [M1 completion gate](architecture/testing.md#m1-completion-gate) passes. Correctness tests pass on Linux and macOS, including crash, recovery, revision, retry and admission behavior. Required artifacts include Linux group-flush syscall evidence, directory-sync results on both platforms, and a reproducible Linux core benchmark report in `docs/`. Correctness or durability failures and missing evidence block completion. Provisional M1 performance misses are reported findings. |
| M2 | Full model-based verification of revisions and keys. | Property tests for R10–R12 pass against the model. A rejected revision uses no offset. |
| M3 | Segment roll. | No submit crosses a roll in the crash tests. A crash at each step of a roll recovers. A Linux syscall trace verifies directory sync during roll ([ADR 0029](decisions/0029-m1-completion-requires-correctness-and-measurement-evidence.md)). |
| M4 | Sealed footer and locator, `seal`, read-only open, `LogReader`. Settle and implement [retained-key lookup and its key-cache memory budget](#open-items) under [ADR 0031](decisions/0031-keys-live-with-retained-segments.md). | Open time for M4 conditions meets the target. From each sealed segment except the newest, open reads only the header, the locator and the footer, counted through the storage trait. Retained-key retry and cache-rebuild tests pass, and the chosen key-cache memory budget is enforced. |
| M5 | Freeze the byte layout. Fuzz the recovery parser. | A format ADR is Accepted. Fuzzing runs for 1 hour in CI with no crash. |
| M6 | `segment-log-server` over Unix socket first, then vsock using the same protocol ([ADR 0034](decisions/0034-m6-builds-unix-socket-before-vsock.md)). Benchmark against the targets. | A Go test client appends and reads over both transports, with shared protocol conformance cases for retries, revisions and admission outcomes. The benchmark report is in `docs/` and identifies the actual guest stack with its required storage-qualification record ([ADR 0032](decisions/0032-qualify-deployment-storage-before-m6.md)). Qualification is required before the guest benchmark and production durability claims. The deployment record identifies mount paths, the reader component and verified post-guest handoff under [ADR 0033](decisions/0033-deployment-owns-volumes-and-reader-handoff.md). |

`segment-log-server` stays a placeholder until M6 ([ADR 0001](decisions/0001-rust-library-first-socket-boundary-later.md)).

## Open items

Unresolved items name the milestone that they block. Resolved items keep their numbers for existing references and link their accepted decisions.

1. **M4 key lookup and key-cache memory (lifetime resolved).** [ADR 0031](decisions/0031-keys-live-with-retained-segments.md) preserves keys while their segments are retained, including after roll and reopen. M1 scans one segment and M3 can scan all retained segments. M4 footers do not contain keys, so M4 must choose a lookup mechanism and key-cache memory budget before replacing old record scans with summary-based open. A rebuildable per-segment key index is one candidate, not an approved format. Any index must remain rebuildable from segment records under I9. The M4 read gate limits reads from sealed segment files; separate lookup metadata is allowed but its reads, rebuild cost, memory use and effect on append performance must be measured. The M4 decision must cover lookup I/O failures under the writer poison rule as well. Lookup design and memory policy block the M4 optimization, not M1; expiring keys on roll is not the chosen solution.
2. **Resolved for M1. Expected revisions use Empty and At.** [ADR 0030](decisions/0030-expected-revisions-use-empty-and-at.md) adopts the preconditions in [api.md](architecture/api.md). `Empty` requires a stream with no records; its first successful record gets revision zero. `At(n)` requires current last revision `n`; its next successful record gets revision `n + 1`. M1 has no unconditional `Any`. Whole-submit rejection, pending-revision visibility and key-first retry matching retain their existing rules.
3. **Deployment storage qualification (scope settled; evidence required before M6).** [ADR 0032](decisions/0032-qualify-deployment-storage-before-m6.md) keeps actual-stack qualification separate from M1 implementation and its mandatory completion evidence. Before the M6 guest benchmark or production durability claims, identify the hypervisor, virtual block device, backing storage, filesystem and mount options, and record evidence and limitations for all three [storage-contract assumptions](architecture/overview.md#storage-contract). M1 tests the explicit conditional contract; its tests, syscall checks and Linux benchmark do not certify the eventual guest stack. The qualification result remains open until that actual setup is identified and checked.
4. **Surviving volume and reader handoff (ownership settled; concrete integration before M6).** [ADR 0033](decisions/0033-deployment-owns-volumes-and-reader-handoff.md) assigns provisioning, mounts, volume preservation and post-guest reader handoff to the deployment or runner layer. The core operates on the supplied directory and owns file validation, recovery and reading. Before M6, identify writer and reader mount paths, the concrete reader component and a handoff procedure that respects single-writer ownership. Sealed history uses `LogReader` from M4; unsealed crash logs need writable recovery through `SegmentLog::open`. This decision chooses no automatic sealing policy. Concrete paths, components and integration evidence remain open without blocking M1.
5. **Resolved sequencing for M6. Unix socket before vsock.** [ADR 0034](decisions/0034-m6-builds-unix-socket-before-vsock.md) builds the Unix-socket path and local Go-client integration first, then vsock using the same protocol. Both transports remain required for M6. Actual caller placement determines the deployed transport. The wire format remains a separate M6 ADR, and server implementation remains outside M1–M5.
6. **Resolved for M1. Waiting cursors belong above the core.** [ADR 0035](decisions/0035-waiting-cursors-belong-above-the-core.md) assigns cursor positions, polling or scheduling, cancellation and timeouts to the consuming layer. M1 exposes `watermark()` and bounded `read()` without a native watermark-wait or subscription API. `Pending::wait` remains a per-submit result wait. A future efficient notification primitive needs a separate decision based on a concrete consumer need; this decision adds no wrapper or endpoint to M1 or M6.
7. **Resolved initial policy. Measure defaults before tuning.** [ADR 0036](decisions/0036-measure-the-existing-defaults-before-tuning.md) retains the existing defaults in [api.md](architecture/api.md), including the already agreed queue-count and admission-budget settings. Benchmark defaults first and report changed settings as separate reproducible variants. Default changes require measurement evidence, a recorded reason and corresponding code and documentation updates. M1 has no roll; the roll-size setting becomes effective from M3. Tuning remains empirical work rather than an assumption that the initial settings meet the targets.
8. **Resolved for M1. Pending retries wait for durability.** [ADR 0021](decisions/0021-exact-pending-retries-wait-for-the-original.md) establishes pending retry waiting, generalized to all fully matching requests by [ADR 0024](decisions/0024-fully-matching-retries-ignore-original-grouping.md). They append nothing and return original positions in request order with `deduplicated: true` after all referenced records are durable. Matched retries get `Indeterminate` if a referenced in-flight group fails. Requests still queued when the writer is poisoned get `Poisoned`.
9. **Resolved for M1. Key scope and retry classification.** Keys are scoped to the whole log ([ADR 0019](decisions/0019-idempotency-keys-are-scoped-to-the-whole-log.md)). Repeated keys within a submit are rejected before admission ([ADR 0020](decisions/0020-reject-repeated-keys-within-a-submit.md)). Pending retries wait for durability ([ADR 0021](decisions/0021-exact-pending-retries-wait-for-the-original.md)). Conflicting key reuse gets `KeyConflict` ([ADR 0022](decisions/0022-reject-conflicting-key-reuse-from-m1.md)). Mixed reused-and-new records get `PartialRetry` ([ADR 0023](decisions/0023-reject-mixed-reused-and-new-records-from-m1.md)). Fully matching retries may select subsets, reorder records, combine original submits, and mix pending and durable matches. They append nothing and return original positions in request order after all referenced records are durable ([ADR 0024](decisions/0024-fully-matching-retries-ignore-original-grouping.md)). Success confirms durability without asserting that the records originally committed atomically together.
10. **Resolved for M1. Admission bytes and size refusals.** [ADR 0025](decisions/0025-admission-byte-reservations-live-until-the-answer.md) adds a configurable budget alongside queue-count capacity, held until each request is answered. [ADR 0026](decisions/0026-admission-charges-retained-allocations.md) sets retained-allocation charges, including unused capacities and explicit bookkeeping and result allowances. [ADR 0027](decisions/0027-default-admission-budget-is-an-accounting-ceiling.md) sets the initial default in [api.md](architecture/api.md), without preallocating a budget-sized buffer. [ADR 0028](decisions/0028-too-large-is-distinct-from-temporary-full.md) uses `TooLarge` for requests that cannot fit a group or the entire budget, including charge overflow, and `Full` for temporary exhaustion. The [charge formula](architecture/api.md#admission-charge) remains separate from encoded size and total process memory.
11. **Resolved for current milestones. Retain complete histories.** [ADR 0037](decisions/0037-retain-complete-logs-through-current-milestones.md) excludes segment-level retention from M1–M6. Deployment owns whole-log archival and expiry; the core keeps every valid history segment without a retention option or deletion endpoint. Existing recovery cleanup of invalid tails and incomplete new segments is unchanged. A future bounded-history feature needs durable revision checkpoints, key-expiry rules, reads from deleted offsets and crash-safe deletion semantics. The former footer-coverage deletion proposal is not part of the current milestones.

## Risks

| Risk | Mitigation |
| --- | --- |
| The device, the hypervisor or the filesystem breaks the storage contract (drops flushes, tears a sector, or keeps writing after a metadata error), so an acknowledgment is not durable. | Qualify the actual stack under [ADR 0032](decisions/0032-qualify-deployment-storage-before-m6.md) before M6 guest validation and production durability claims. Open item 3 remains an evidence task; M1 tests do not certify the deployment. |
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
