# Testing and benchmarks

M1 requires the completion gate below, including the crash strategy in [ADR 0014](../decisions/0014-crash-testing-strategy.md) and the evidence policy in [ADR 0029](../decisions/0029-m1-completion-requires-correctness-and-measurement-evidence.md). Later milestones do not start until it passes.

## M1 completion gate

All of the following evidence is required:

1. Repository format, lint and correctness checks pass on Linux and macOS. M1 tests cover simulated `Kill`, `PowerLoss` and `PowerLossTorn`, real-process `SIGKILL`, I/O fault injection, durable recovery, one-writer locking, expected revisions, retry rules and admission limits. The oracle and M1 cases below define the required behavior. Full model-based verification remains M2.
2. A Linux syscall trace shows exactly one `fdatasync` for each new committed group. Identify initialization and recovery syncs separately. A fully deduplicated retry adds no group or flush solely for that retry. Record the command, toolchain and trace used to verify the calls.
3. Directory-sync checks succeed on Linux and macOS during log creation and recovery, using the actual storage implementation. Record the commands and results. These checks verify the calls used by the library; they do not certify a deployment's storage contract. [ADR 0032](../decisions/0032-qualify-deployment-storage-before-m6.md) keeps actual-stack qualification as a pre-M6 requirement under PRD item 3.
4. A reproducible Linux core benchmark report is saved in `docs/`. It includes the commands, workload parameters, actual hardware and software conditions, and applicable measurements in the benchmark section below. Record deviations from target conditions and do not claim a target passed under different conditions. Report seeds or other parameters needed to reproduce randomized workloads.

Correctness or durability failures block completion. Missing evidence, omitted applicable measurements or an unreproducible report also block completion. Missed provisional M1 performance targets are explicit findings in the report, not completion blockers. The completion record links the test results, syscall and directory-sync evidence, and benchmark report. This gate defines required evidence; it does not claim that any of it has run yet.

Roll-specific syscall verification belongs to M3. Footer-based open-time and seal targets wait for M4, and server footprint and socket measurements wait for M6. An M1 report marks these later-feature metrics as not applicable. Existing later-milestone gates remain unchanged.

Waiting cursors remain above the core under [ADR 0035](../decisions/0035-waiting-cursors-belong-above-the-core.md). M1 verifies watermark and bounded-read durability, plus per-submit `Pending::wait` outcomes. It adds no native watermark-wait API or waiting-cursor implementation to this gate. Any future wrapper is tested at its consumer boundary.

## Storage trait

All file I/O in `segment-log-core` goes through one small trait. It has two implementations:

- **Real storage.** `std::fs`. Used in production and in the real-process tests.
- **Simulated storage.** In memory, deterministic, and seeded. Used for the crash tests.

The trait needs only the calls the log makes: create, open, append, write at a position, read at a position, `sync_data`, `sync_all`, `sync_dir`, truncate, remove, list, lock, and random bytes. Keep it that small. The random bytes are for the log id, so that the simulated storage can derive them from its seed.

## Crash model of the simulated storage

For each file, the simulated storage keeps two states:

- **durable**: the bytes and the length at the last successful `sync_data` or `sync_all`;
- **volatile**: everything written since then.

For the directory, it keeps durable and volatile entry lists in the same way. `sync_dir` makes the current entry list durable.

A test calls `crash(mode, seed)` at any step:

| Mode | Models | Result for each file |
| --- | --- | --- |
| `Kill` | `kill -9` | All written bytes and entry changes stay readable (the kernel has them). They stay volatile, so a later `PowerLoss` can still drop them. |
| `PowerLoss` | Power cut, device honors flush | Durable bytes stay. Each 4 KiB page of volatile bytes is kept or lost on its own, not only as a prefix. A lost page reads as its durable contents, or as zeros past the durable length. Directory changes since the last `sync_dir` can be lost. |
| `PowerLossTorn` | Power cut with a torn write | Like `PowerLoss`, and a kept volatile page can end in random garbage bytes. A sector whose volatile contents equal its durable contents never tears, because the device writes a sector atomically ([ADR 0015](../decisions/0015-open-makes-the-recovered-tail-durable.md)). |

A test can chain crashes, for example `Kill`, then a reopen, then `PowerLoss`.

The storage can also fail on purpose: `sync_data`, `sync_all` or `sync_dir` returns an error, `write` writes only part of the buffer and then fails, or the disk is full (`ENOSPC`). After a failed `sync_data`, the volatile bytes stay readable, but a later successful `sync_data` does not make them durable. Only a new write to those bytes does. This models how Linux can mark pages clean after a writeback error ([ADR 0015](../decisions/0015-open-makes-the-recovered-tail-durable.md)).

## The oracle

A test runs many caller threads against the simulated storage. Each caller writes a log of what it sent and what it got back. After a crash and a reopen, the test checks:

1. **Acknowledged means kept (I3).** Every acknowledged record is present with the same log offset, revision, stream, key and payload. This holds in every crash mode and every chain of crashes, because an ack comes only after a successful `sync_data` that made the record durable. A `deduplicated: true` answer counts as an ack.
2. **Prefix only (I4).** The recovered records are a prefix of the order in which the writer wrote groups. There are no holes and no reordering.
3. **All or nothing (I5).** Each submit of new records is present in full or absent in full. Fully deduplicated retries add no records and do not imply that their matches originally committed atomically together.
4. **Retries resolve.** The test retries every submit that did not get an ack, with the same keys and bytes. Afterwards, each keyed record is present exactly once. A survivor returns its original offset and revision with `deduplicated: true`.
5. **Recovery is durable.** A `PowerLoss` right after open, before any append, gives the same log. Run this after every crash mode and after an injected `sync_data` error.
6. **Sealed segments do not change (I7).** Their bytes are the same before and after recovery.
7. **Poison (I10).** After an injected I/O error, every in-flight submit and its matched retry waiters get `Indeterminate`, and every queued or later submit gets `Poisoned`. A reopen in the same simulated boot, then a `PowerLoss`, then another reopen, satisfies checks 1–4.
8. **Readers see only durable records (I1).** Reader threads run during the test and record every record they read. After a crash and a reopen, every record a reader saw is present with the same log offset and payload.
9. **The watermark never moves back (I2).** The watermark after a reopen is at least the highest watermark that any caller or reader saw before the crash.
10. **Durable offsets are never used again (I6).** After a reopen, no new append gets a log offset that a caller or reader saw before the crash.
11. **One writer (I8).** A second `SegmentLog::open` on the same directory fails with `Locked`, from another process and from the same process.
12. **Caches can be rebuilt (I9).** After the test deletes every cache file, such as the sparse index, a reopen gives the same watermark, the same `stream_head` for every stream, and the same `read` results.
13. **Open reads only the summary (M4).** The storage trait counts the bytes that open reads. From each sealed segment except the newest, open reads only its header, its locator and its footer.

Run each test with many seeds. Print the seed when a test fails, so that the failure can be replayed.

## Real-process tests

- **`SIGKILL` (Linux and macOS).** A child process appends from many threads and writes each ack to a pipe. The parent kills it at a random time with `SIGKILL`, reopens the directory, and runs oracle checks 1–4, 9 and 10 against the acks and watermarks it received. Repeat many times.
- **Power loss on a real filesystem (Linux, optional job).** Run the same test on [LazyFS](https://github.com/dsrhaslab/lazyfs). LazyFS is a FUSE filesystem that can drop data that was not flushed. If LazyFS is not practical in CI, run it by hand before M5 and record the result in `docs/`.

## Property and unit tests

- Default options, from M1 ([ADR 0027](../decisions/0027-default-admission-budget-is-an-accounting-ceiling.md)). Check that `Options::default` matches [the API defaults](api.md). Changing `pending_byte_budget` must not change queue-count capacity or the group-byte cap.
- Admission budgets, from M1 ([ADR 0025](../decisions/0025-admission-byte-reservations-live-until-the-answer.md)). Fill queue-count capacity while byte budget remains, then exhaust byte budget while queue capacity remains. Both cases give immediate `Full` and accept no request. Concurrent producers cannot reserve more than the byte budget. A failed queue insertion releases its temporary reservation.
- Permanent size refusal and temporary exhaustion, from M1 ([ADR 0028](../decisions/0028-too-large-is-distinct-from-temporary-full.md)). A group-sized submit whose retained-allocation charge exceeds the entire budget gets `TooLarge` even on an empty log. Charge overflow and exceeding the encoded group cap also get `TooLarge`. Check exact-limit eligibility, no write, no offset or revision consumed, and no retained reservation. A size-eligible submit gets `Full` when outstanding charges or a full queue prevent admission, then succeeds after temporary pressure clears. Impossible size fits remain `TooLarge` even while other requests occupy capacity. Cover retries as well as new submits.
- Admission reservation lifetime, from M1. Pause the writer after dequeue and verify that the request's byte reservation remains held. Match pending retries and verify that their reservations also remain held. Dropping a `Pending` handle must not free an unfinished request's reservation. Check release on successful answers, validation rejections, `KeyConflict`, `PartialRetry`, `WrongRevision`, poison and writer shutdown. A completed answer releases the reservation even if the caller has not consumed it.
- Retained-allocation charges, from M1 ([ADR 0026](../decisions/0026-admission-charges-retained-allocations.md)). Build equal-content submits with different record-vector and stream, key and payload capacities. They have equal encoded sizes but different admission charges under [the formula](api.md#admission-charge). Cover unused record slots, many small records, result-position storage and actual request-bookkeeping sizes. Retrying or moving a request's buffers must not grow its reserved charge or release it early. Checked arithmetic must refuse an unrepresentable charge without wrapping.
- Codec round trip for every frame kind ([format.md](format.md)), plus the CRC check value.
- A model test: a plain in-memory model of streams, revisions and keys. Random submits go to the model and to the log, and the results must be equal. This covers R9–R12.
- Retained-key lifetime ([ADR 0031](../decisions/0031-keys-live-with-retained-segments.md)). In M1, commit keyed records, drop the log, reopen it, and verify that retries return the original positions without another append. In M3, repeat after rolling the original records into an older retained segment, both before and after reopen. M4 must test its chosen lookup mechanism and cache rebuilds against the same lifetime guarantee, and enforce its key-cache memory budget before replacing full scans with summary-based open.
- Complete-history retention ([ADR 0037](../decisions/0037-retain-complete-logs-through-current-milestones.md)). In M1, retain committed records and keys across reopen. From M3, repeated rolls preserve every valid history segment, including streams with later records and streams that never appear again. From M4, later footer coverage still does not permit deletion, and sealing and read-only open preserve the complete readable history. Keep these cases distinct from permitted torn-tail and incomplete-new-segment recovery cleanup. No valid history is deleted through a retention policy in M1–M6.
- Expected revisions, from M1 ([ADR 0030](../decisions/0030-expected-revisions-use-empty-and-at.md)). On separate streams with no records, `Empty` succeeds at revision zero and `At(0)` fails with actual revision `None`. On separate streams at revision zero, `Empty` fails with actual revision `Some(0)` and `At(0)` succeeds at revision one. Check successive records in one submit and valid earlier pending submits. A later failing record rejects the entire submit and leaves no offset, revision or key reservation consumed. Fully matching pending and durable retries succeed despite stale expectations.
- Whole-log key scope, from M1 ([ADR 0019](../decisions/0019-idempotency-keys-are-scoped-to-the-whole-log.md), [ADR 0022](../decisions/0022-reject-conflicting-key-reuse-from-m1.md)). Commit a key on stream A, then use the same key and payload on stream B. The second submit gets `KeyConflict` and appends nothing. Check before and after reopen.
- Conflicting key reuse, from M1. Check a reused key with a different stream or payload against both a durable original and a valid pending original in the current group. Include a multi-record submit with a fresh record before the conflict and a submit with several conflicts. `Pending::wait` returns `KeyConflict` with the first conflicting record's index, even when an expected revision is also wrong. The rejected submit writes no bytes, reserves no fresh key, and changes no watermark or stream head. The next valid append uses the next unused offset and revision. Pending originals and their matched retry waiters still complete normally. Reusing a key with only a different expected revision follows the retry rule.
- Key conflicts across crashes, from M1. Reject a conflicting submit while its original is pending, then crash before or after the group flush. After reopen, only the original can have survived. Retry its keys under oracle checks 1–5 and verify that none of the rejected submit's fresh records appear.
- Mixed reused and new records, from M1 ([ADR 0023](../decisions/0023-reject-mixed-reused-and-new-records-from-m1.md)). Mix a matching durable or pending record with a fresh keyed record or an unkeyed record, in either order. The whole submit gets `PartialRetry` before expected-revision checks and writes nothing. Check unchanged watermark, stream heads, key reservations, offsets and revisions, and that original submits and their retry waiters still complete. Include several matching records spread across pending and durable state alongside a fresh record. A conflicting key later in a mixed submit must produce `KeyConflict` at its first conflicting index instead of `PartialRetry`. A submit containing only fresh keyed and unkeyed records passes this key classification.
- Partial-retry rejection across crashes, from M1. Reject a mixed submit while a matching original is pending, then crash before or after the original group's flush. Reopen and retry original keys under oracle checks 1–5. No fresh record from the rejected submit can appear.
- Repeated keys within a submit, from M1 ([ADR 0020](../decisions/0020-reject-repeated-keys-within-a-submit.md)). Two records with the same key get `AppendError::Invalid` before admission. Cover identical streams and payloads, different streams, and different payloads. Check that rejection leaves queue capacity, watermark and stream heads unchanged and writes no bytes. The next successful append gets the next unused offset and revision. Multiple unkeyed records pass this validation.
- Exact pending retries, from M1 ([ADR 0021](../decisions/0021-exact-pending-retries-wait-for-the-original.md)). Stage an original multi-record submit and multiple exact retries in one group, including retries with stale expected revisions. Block the flush and verify that neither the original nor its retry waiters return success, and readers cannot see the group's records. After a successful flush and watermark publication, every retry returns the original positions with `deduplicated: true`. Only the original records appear in the group frame, and retries consume no offset or revision.
- Regrouped retries, from M1 ([ADR 0024](../decisions/0024-fully-matching-retries-ignore-original-grouping.md)). Write originals with keys `[K1, K2]` and `[K3]`. Retry a subset, reorder records, and combine originals as `[K3, K1]`. Cover all-pending, all-durable, and mixed pending and durable matches, including multiple originals on the same stream. Use stale expected revisions. Success waits for every referenced record to be durable and returns original positions in request order with `deduplicated: true`, even when offsets decrease. Retries add no records, consume no offset or revision, and trigger no extra flush. Repeat with retained records after reopen.
- Regrouped pending-retry failures, from M1. Match a retry against durable records and several pending originals in the current group. An injected write or flush failure gives that matched retry `Indeterminate`; an unprocessed queued retry gets `Poisoned`. Run the crash oracle against the original new submits, and verify that the regrouped retry added no record.
- Pending-retry failures and crashes, from M1. Inject a short write, `ENOSPC`, or flush failure into the original group. The original and its matched retry waiters get `Indeterminate`. A retry still queued when poisoning happens gets `Poisoned`. At crash points before and after the flush, reopen and retry the original keys under oracle checks 1–5. Each original keyed record appears exactly once.
- A rejected revision uses no offset: the next successful append gets the offset that the rejected one would have had.
- Recovery on hand-made files: a zero-filled tail, a length past the end of the file, a bad CRC, a lower epoch, a wrong base offset, a header with the wrong log id, a file name that differs from its header's base offset, a missing middle segment, and a crash during a roll. Expected outcomes for the cases where a wrong answer loses data:
  - A newest segment longer than a header with a bad header, or with the wrong log id, fails open. Open does not delete it.
  - A footer or seal frame with no locator after it (M4) is truncated, because its `fsync` never returned.
  - A record whose payload ends with bytes that form a valid footer, seal frame and locator (M4) leaves the segment active.
  - A locator that survives a power loss while its footer's page is lost (M4) leaves the segment active, and the closing frames are truncated.
  - A clean reopen, then `PowerLossTorn` during open's rewrite, keeps every acknowledged record.
  - A bad frame in a sealed log's final segment (M4) fails with `Corrupt`. Open does not truncate it.
  - A seal on a segment with no group frame, then a failed `sync_data` or a `Kill`, then a reopen and a `PowerLoss`, still gives a sealed log.
- The writer rolls early when a group's new streams would make the footer pass the frame limit.
- `LogReader` on storage that refuses every write, and with two readers open at once. On an unsealed log, it fails with `NotSealed`.

`proptest` is a suitable dev-dependency for these tests.

## Fuzzing (M5)

`cargo fuzz` targets for the frame decoder and for recovery on an arbitrary active segment. A target must never panic. It must either return the valid prefix or return an error.

## Benchmarks (M1 for the core, M6 for the server)

The benchmark measures the [PRD targets](../prd.md#footprint-and-performance-targets). For each run, report:

- the hardware, the filesystem, the kernel, and whether the device has a volatile write cache;
- the `fsync` latency of the device alone (p50, p99), measured first, with the log's pattern: append to a growing file, then `fdatasync`. Each append grows the file, so `fdatasync` must also commit the new size, which a rewrite in place does not measure;
- append throughput and p50/p99/p999 latency;
- the configured admission byte budget, admission refusals and peak outstanding charges;
- the group size distribution (submits and bytes for each `fsync`);
- open time, seal-to-exit time and idle resident set, where they apply.

[ADR 0036](../decisions/0036-measure-the-existing-defaults-before-tuning.md) requires a run under the applicable [API defaults](api.md) before tuning experiments. Record all active options. Report changed settings as separate named variants with their exact configuration and the same correctness and durability contract. Preserve the baseline results. A proposed default change includes its measurement evidence, rationale and corresponding code and documentation update. M1 has no roll, so it does not measure or tune roll behavior; that setting becomes effective from M3.

A benchmark on macOS is for development only. The targets are for Linux.

For M1, the Linux report is required by the completion gate even if the provisional performance targets are missed. Include each applicable target comparison and explain any miss or condition that prevents a comparable result. Keep missing evidence separate from measured misses.

## Deployment storage qualification (before M6)

[ADR 0032](../decisions/0032-qualify-deployment-storage-before-m6.md) requires a record in `docs/` for the actual guest stack before its M6 benchmark and production durability claims. This is separate from M1's mandatory tests, syscall checks and Linux benchmark.

The record identifies the hypervisor, virtual block device, backing storage, filesystem and mount options, including relevant versions and configuration. For each storage-contract assumption, it states the qualification method, supporting evidence, limitations and unresolved conditions:

- successful flushes make the bytes survive power loss;
- sector writes are atomic as required by recovery's identical-byte rewrite;
- the filesystem stops accepting writes after metadata-write errors as the recovery contract assumes.

Evidence must apply to the identified setup. A syscall trace alone does not establish the assumptions. An unverified or contradicted assumption blocks qualification. M6 evidence names the qualified stack used for its guest benchmark. Changes to that stack require reviewing whether the qualification still applies. No qualification is claimed by this checklist itself.

## Surviving-volume handoff (before M6)

[ADR 0033](../decisions/0033-deployment-owns-volumes-and-reader-handoff.md) assigns the volume lifecycle and reader handoff to the deployment layer. Before M6 deployment validation, record the surviving volume, its writer and reader mount paths, the concrete reader service or tool, and the handoff procedure. The procedure must respect the single-writer contract and identify access for writable recovery when the guest exits with an unsealed log.

Integration evidence must show that acknowledged records remain available on the surviving volume after guest exit and can be read by the selected component. Cover sealed history through `LogReader` and an unsealed crash log through writable `SegmentLog::open` recovery. Using `LogReader` afterward requires explicitly sealing the recovered log; the test must state that policy rather than silently finalizing a resumable log.

The integration record references the actual-stack qualification in ADR 0032. These checks do not select a provider, concrete path or reader implementation for M1, and do not claim that a deployment handoff has been exercised yet.

## M6 transport order and conformance

[ADR 0034](../decisions/0034-m6-builds-unix-socket-before-vsock.md) requires Unix-socket integration first, followed by vsock using the same protocol. Record Go-client append and read results over both transports before marking M6 complete. Reuse protocol conformance cases for retries, expected revisions and admission outcomes, with the same core contract for both adapters.

The vsock evidence must exercise the selected guest topology. Passing Unix-socket cases does not establish vsock connectivity. Record the protocol version, client and server commands, topology and results needed to reproduce each path. Protocol bytes and framing will be selected in their own M6 ADR; this checklist does not select them or claim either transport is implemented.
