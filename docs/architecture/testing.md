# Testing and benchmarks

M1 requires the completion gate below, including the crash strategy in [ADR 0014](../decisions/0014-crash-testing-strategy.md) and the evidence policy in [ADR 0029](../decisions/0029-m1-completion-requires-correctness-and-measurement-evidence.md). Later milestones do not start until it passes.

## Acceptance testing across milestones

The [acceptance-test workflow](../acceptance-tests.md) ([ADR 0039](../decisions/0039-scenario-first-acceptance-tests-in-native-rust.md)) says how to write and run scenarios. This document remains the source for the crash model, the oracle and the required cases. Each required case has a stable case ID, such as `ADM-PRECEDENCE`, `ORACLE-7` for oracle check 7, or `RETRY-UNKNOWN-3` for row 3 of the retry table. A test names the case IDs it covers. Case IDs carry no milestone, so a scenario keeps its ID when a later milestone extends it.

The tables below track the status of each coverage group. They do not list test paths; the tests and the coverage check described in the workflow hold that mapping. Change a status only when a group moves between `Planned`, `Partial` and `Implemented`. A group is `Implemented` only when every applicable part of its listed cases has a test.

### Milestone acceptance coverage

The [PRD milestone gates](../prd.md#milestones) remain the source for completion criteria. Later rows are planned groups, not claims that a harness or an unresolved design already exists. Expand each group into concrete scenarios before its implementation. Scenario cases and gate evidence are separate columns: passing scenarios never replaces a measurement, a record or a decision.

| Group | Scenario cases | Gate evidence | Status |
| --- | --- | --- | --- |
| M0-READINESS | None | Green CI run linked in the [PRD](../prd.md#milestones) | Implemented |
| M1 | [M1 groups below](#m1-acceptance-coverage) | [M1 completion gate](#m1-completion-gate) | Partial |
| M2-MODEL | `MODEL`, `REV-NO-OFFSET` | None beyond the tests | Planned |
| M3-ROLL | `ORACLE-6`, M3 parts of `OPEN-LAYOUT`, `RECOVERY-FIXTURES`, `KEY-LIFETIME` and `HISTORY`; M1 scenarios rerun with a small `roll_bytes` | Linux syscall trace of the roll directory sync | Planned |
| M4-SEAL-READ | `READER`, `ROLL-FOOTER`, M4 parts of `CODEC-GROUP`, `ADM-PRECEDENCE`, `RECOVERY-FIXTURES` and `HISTORY` | None beyond the tests | Planned |
| M4-LOOKUP-OPEN | `ORACLE-13`, M4 parts of `ORACLE-12` and `KEY-LIFETIME` | Open-time target result; lookup reads, rebuild cost and key-cache memory under the [M4 design](../prd.md#open-items) | Planned |
| M5-FORMAT-FUZZ | `FUZZ-DECODER`, `FUZZ-RECOVERY` | Accepted format ADR; one-hour fuzz run in CI | Planned |
| M6-TRANSPORT | `TRANSPORT-CONFORMANCE`, `TRANSPORT-UNKNOWN`, `TRANSPORT-RESTART` | Protocol version, commands, topology and results for each transport | Planned |
| M6-DEPLOYMENT | `HANDOFF` | [Storage qualification](#deployment-storage-qualification-before-m6) with every [volume requirement](volume-requirements.md) case, [handoff record](#surviving-volume-handoff-before-m6), [guest benchmark report](#benchmarks-m1-for-the-core-m6-for-the-server) | Planned |

Record an unresolved later-milestone decision as a dependency. Do not guess its expected outcome in a test.

### M1 acceptance coverage

Storage-model, conformance, codec and first engine scenarios are partial. The engine scenarios cover the flush visibility boundary, basic queued grouping, bounded reads, initial rejection checks and directed crash/fault variants. Counter scenarios cover append boundaries, first-failure ordering, refusal without mutations and epoch recovery including crash variants. Durable-key scenarios cover whole-log scope, conflicts, partial retries, regrouping, frame lookup faults, immediate answers and counter-free retries. Directed crash and reopen scenarios rebuild keys from the kept prefix and preserve full history. Pending-key scenarios now cover immediate classification, matched waiters, mixed and regrouped retries, retained waiter charges, cap deferral, exhausted counters and write, flush and lookup failures. Directed crash cases cover pending rejections and atomic originals before write, before flush and after acknowledgment. The unknown-outcome table has directed examples for each row. These cases do not cover the full randomized oracle. No completion evidence exists yet. A group lists only the M1 part of each requirement and case. Rolling, sealed-segment handling, footers, locators and read-only open keep their later gates. Seeded crash-oracle coverage starts in M1; the full model test remains M2.

| Group | Requirements or invariants | Scenario cases | Status |
| --- | --- | --- | --- |
| M1-SIMULATOR | [ADR 0014](../decisions/0014-crash-testing-strategy.md) | `SIM-MODEL`, `SIM-CONFORMANCE` | Partial |
| M1-CODEC | R7, R16–R17 | `CODEC-GROUP` | Partial |
| M1-DURABILITY | R2–R4, R18–R19; I1–I3 | `ORACLE-1`, `ORACLE-8`, `ORACLE-9`, `GROUP-FORMATION`, `READ-BOUNDED` | Partial |
| M1-ATOMICITY | R1, R3, R7–R8; I4–I6 | `ORACLE-2`, `ORACLE-3`, `ORACLE-10` | Partial |
| M1-RECOVERY | R8, R14, R16–R17; I2–I6 | `ORACLE-1`–`ORACLE-5`, `RECOVERY-FIXTURES`, `OPEN-FAULTS` | Partial |
| M1-COUNTERS | [ADR 0041](../decisions/0041-counter-exhaustion-refuses-without-poisoning.md) | `COUNTER-EXHAUSTION` | Partial |
| M1-REVISIONS | R9, R11–R12 | `REV-EXPECTED`, `REV-NO-OFFSET` | Partial |
| M1-RETRY-CLASSIFICATION | R1, R10, R12 | `KEY-REPEATED`, `KEY-SCOPE`, `KEY-CONFLICT`, `KEY-PARTIAL`, `RETRY-REGROUPED` | Partial |
| M1-PENDING-RETRIES | R4, R10, R12; I1 | `RETRY-PENDING`, `RETRY-PENDING-FAIL`, `RETRY-REGROUPED-FAIL`, `ANSWER-TIMING` | Implemented |
| M1-UNKNOWN-OUTCOMES | R1, R10–R12 | `ORACLE-4`, `RETRY-UNKNOWN-1`–`RETRY-UNKNOWN-8`, `KEY-CONFLICT-CRASH`, `KEY-PARTIAL-CRASH` | Partial |
| M1-ADMISSION | R5 | `OPT-DEFAULTS`, `ADM-PRECEDENCE`, `ADM-BUDGETS`, `ADM-SIZE`, `ADM-RESERVATION`, `ADM-CHARGE` | Implemented |
| M1-POISON | R6; I10 | `ORACLE-7`, `POISON-LOOKUP` | Implemented |
| M1-OPEN-BOUNDARIES | R14, R16–R17; I8 | `OPT-VALIDATION`, `OPEN-DIR`, `OPEN-LAYOUT`, `ORACLE-11` | Implemented |
| M1-REOPEN-HISTORY | R13, R18–R19, R22; I9 | `ORACLE-12`, `KEY-LIFETIME`, `HISTORY` | Partial |
| M1-DROP | R4, R16; I8 | `DROP` | Implemented |
| M1-PROCESS-CRASH | R4, R8, R14, R16 | `PROC-SIGKILL` | Implemented |

Passing these scenarios does not replace the completion artifacts below.

## M1 completion gate

All of the following evidence is required:

1. Repository format, lint and correctness checks pass on Linux and macOS. M1 tests cover simulated `Kill`, `PowerLoss` and `PowerLossTorn`, real-process `SIGKILL`, I/O fault injection, durable recovery, one-writer locking, expected revisions, retry rules and admission limits. The oracle and M1 cases below define the required behavior. Later-milestone cases are required only from their stated milestone. Full model-based verification remains M2.
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

The trait and both implementations stay private to the crate. `SegmentLog` holds its storage as a trait object, so the public type has no storage type parameter. `SegmentLog::open` passes real storage. A private constructor takes any storage, and the simulated tests use it. Both paths run the same writer and recovery code. The simulated storage is compiled only for tests and, from M5, for fuzzing (`cfg(any(test, fuzzing))`). It is never part of a release build. Keep the frame decoder and the recovery scan callable on plain bytes, so that the M5 fuzz targets can reuse them without a public testing API.

## Crash model of the simulated storage

For each file, the simulated storage keeps two states:

- **durable**: the bytes and the length at the last successful `sync_data` or `sync_all`;
- **volatile**: everything written since then.

For each modeled directory, it keeps durable and volatile entry lists in the same way. `sync_dir` makes that directory's current entry list durable. Model the log directory and its parent separately. Test setup provisions durable ancestors, as required by the [directory contract](api.md#opening-a-directory).

A test calls `crash(mode, seed)` at any step:

| Mode | Models | Result for each file |
| --- | --- | --- |
| `Kill` | `kill -9` | All written bytes and entry changes stay readable (the kernel has them). They stay volatile, so a later `PowerLoss` can still drop them. |
| `PowerLoss` | Power cut, device honors flush | Durable bytes stay. Each 4 KiB page of volatile bytes is kept or lost on its own, not only as a prefix. A lost page reads as its durable contents, or as zeros past the durable length. Directory changes since the last `sync_dir` can be lost. |
| `PowerLossTorn` | Power cut with a torn write | Like `PowerLoss`, and a kept volatile page can end in random garbage bytes. A sector whose volatile contents equal its durable contents never tears, because the device writes a sector atomically ([ADR 0015](../decisions/0015-open-makes-the-recovered-tail-durable.md)). |

A test can chain crashes, for example `Kill`, then a reopen, then `PowerLoss`.

The storage can also fail on purpose: `sync_data`, `sync_all` or `sync_dir` returns an error, `write` writes only part of the buffer and then fails, or the disk is full (`ENOSPC`). A durable-key lookup can return a read error, unexpected EOF or a frame with a bad CRC or malformed contents. These lookup faults poison the writer, rather than treating the key as absent or conflicting. After a failed `sync_data` or `sync_all`, the volatile bytes stay readable, but a later successful file sync does not make them durable. Only a new write to those bytes does. This models how Linux can mark pages clean after a writeback error ([ADR 0015](../decisions/0015-open-makes-the-recovered-tail-durable.md)). A call can also panic on purpose, so that a test can stop the writer thread without an answer.

A test can hold a chosen call at its start and later release it with success or an injected error. The simulated storage reports when the held call has started. This lets a scenario stop the writer at a flush, a write or a key lookup without a sleep and without a hook in the writer. The storage also counts calls by file and purpose, for observations such as one data flush for each group.

The simulated storage records every call and its result, in order, as a trace. `crash(mode, seed)` derives the crash state from that trace and the seed alone. The thread interleaving that produced the trace cannot be replayed, but the crash state and the recovery that follows can. Recovery runs on one thread, so it is deterministic for a given crash state.

### Checking the simulated storage

The crash tests are only as good as the crash model ([ADR 0014](../decisions/0014-crash-testing-strategy.md)). Test the model itself, from M1:

- `SIM-MODEL`. Each rule above has a direct test. Unsynced pages can be lost on `PowerLoss`, and each page is kept or lost on its own. A sector with unchanged contents never tears. Entries can be lost until `sync_dir`, and the log directory and its parent are separate. After a failed `sync_data` or `sync_all`, a later successful `sync_data` without a new write leaves those bytes volatile. The same trace and seed always give the same crash state.
- `SIM-CONFORMANCE`. Run the same sequence of calls, without a crash, against real storage in a temporary directory and against the simulated storage. Every read, length, listing and lock result matches. Cover create, append, write at a position, truncate, remove and a second lock on the same file from the same process.

## The oracle

A test runs many caller threads against the simulated storage. Each caller writes a log of what it sent and what it got back. Distinguish success, an observed terminal rejection, and an unknown outcome. `Indeterminate` or a lost reply is unknown. A received `Invalid`, `TooLarge`, `Full`, `Poisoned`, `KeyConflict`, `PartialRetry`, `WrongRevision` or `Exhausted` confirms that this request appended no new record. The absence of a success reply alone does not put a request into a successful-retry test.

Checks apply from M1 unless marked with a later milestone. After a crash and a reopen, the test checks:

1. **Acknowledged means kept (I3).** Every acknowledged record is present with the same log offset, revision, stream, key and payload. This holds in every crash mode and every chain of crashes, because an ack comes only after a successful `sync_data` that made the record durable. A `deduplicated: true` answer counts as an ack.
2. **Prefix only (I4).** The recovered records are a prefix of the order in which the writer wrote groups. There are no holes and no reordering.
3. **All or nothing (I5).** Each submit of new records is present in full or absent in full. Fully deduplicated retries add no records and do not imply that their matches originally committed atomically together.
4. **Keyed survivors resolve.** Replay fully keyed submits with unknown outcomes using the same keys, stream ids, payloads and expectations. When all records survived, the retry returns their original positions with `deduplicated: true` and adds no records. If no key survived, ordinary expected-revision checks apply; they can reject the replay. Other matching or conflicting keys follow the `PartialRetry` and `KeyConflict` rules. Use the model and recovered key state to predict rejection or a new append. A known terminal rejection has no promise of success on replay. Apply the mixed and unkeyed cases below separately.
5. **Recovery is durable.** A `PowerLoss` right after open, before any append, gives the same log. Run this after every crash mode and after an injected `sync_data` error.
6. **Sealed segments do not change (I7, from M3).** Their bytes are the same before and after recovery. M3 uses group-only sealed segments; M4 adds closing frames.
7. **Poison (I10).** After an injected write, flush or durable-key lookup failure, every unfinished request in the current group and its dependent retry waiters gets `Indeterminate`, even before the group is written. Queued or deferred requests get `Poisoned`. Finalized rejections and all-durable retry successes stay fixed. No subsequent write or flush uses the failed handle. A reopen in the same simulated boot, then a `PowerLoss`, then another reopen, satisfies checks 1–4 when the injected fault is disabled. Persistent corruption may instead make open fail; it must not produce a false key miss.
8. **Readers see only durable records (I1).** Reader threads run during the test and record every record they read. After a crash and a reopen, every record a reader saw is present with the same log offset and payload.
9. **The watermark never moves back (I2).** The watermark after a reopen is at least the highest watermark that any caller or reader saw before the crash.
10. **Durable offsets are never used again (I6).** After a reopen, no new append gets a log offset that a caller or reader saw before the crash.
11. **One writer (I8).** A second `SegmentLog::open` on the same directory fails with `Locked`, from another process and from the same process.
12. **Caches can be rebuilt (I9).** In M1, discard in-memory caches by closing and reopening. Delete cache files when their feature introduces them. A reopen gives the same watermark, the same `stream_head` for every stream, and the same `read` results. M4 also covers its chosen retained-key lookup metadata and memory budget.
13. **Open reads only the segment summary (from M4).** The storage trait counts reads separately by file and purpose. From each sealed segment except the newest, open reads only its header, its locator and its footer. This restriction concerns segment files. Any separate lookup-metadata reads, their memory use and rebuild cost must be reported under the M4 design; they are not free or forbidden by this segment-file gate.

Cite oracle check N as case `ORACLE-N`. Run each test with many seeds. When a test fails, print its case IDs, crash mode and seed, and save its trace to a file whose path the failure message names. The trace and seed rebuild the crash state, so that the failure can be replayed. Keep a reduced form of each failing trace as a directed regression case.

### Retry cases required from M1

Use these cases to check the scope of oracle check 4. No case changes expected revisions unless the case explicitly says so.

| Case | Original request and observed outcome | Recovered state and replay | Required result |
| --- | --- | --- | --- |
| `RETRY-UNKNOWN-1` | Fully keyed new submit, unknown outcome | Every key survives with the original stream and payload | Original positions, `deduplicated: true`, no append |
| `RETRY-UNKNOWN-2` | Fully keyed new submit, unknown outcome | No key survives | Ordinary revision checks decide a new append or `WrongRevision` |
| `RETRY-UNKNOWN-3` | Fully keyed submit, unknown outcome | Only a subset of matching keys exists, or a key identifies different bytes | `PartialRetry` or `KeyConflict` under the normal precedence rules, no append |
| `RETRY-UNKNOWN-4` | Fully keyed submit, received `WrongRevision` | No intervening state change, exact replay | `WrongRevision` again, no append |
| `RETRY-UNKNOWN-5` | New submit `[K1, unkeyed]`, unknown outcome | Both original records survive, exact replay | `PartialRetry`, no append |
| `RETRY-UNKNOWN-6` | No original mixed submit | Only the same keyed record K1 exists, submit `[K1, unkeyed]` | Also `PartialRetry`; no evidence that the unkeyed record ever existed |
| `RETRY-UNKNOWN-7` | Fully unkeyed submit, unknown outcome | Original survives, exact replay | `WrongRevision`, no append |
| `RETRY-UNKNOWN-8` | Fully unkeyed submit, unknown outcome | Original survives, caller changes expectations to the new heads | A new append can repeat the payloads; no deduplication guarantee |

For a validated fully keyed original whose records were new, recovery's atomicity and prefix rules leave either all or none of its records. Do not use that fact to infer original-submit membership from an arbitrary `PartialRetry`. Regrouped retries confirm individual records, not a historical transaction.

## Real-process tests

- `PROC-SIGKILL`. **`SIGKILL` (Linux and macOS, from M1).** A child process appends from many threads and writes requests, results and observed watermarks to a pipe. The parent kills it at a random time with `SIGKILL`, reopens the directory, and runs oracle checks 1–4, 9 and 10 against that record. Lost replies are unknown outcomes, not known rejections. Repeat many times.
- `PROC-LAZYFS`. **Power loss on a real filesystem (Linux, optional job).** Run the same test on [LazyFS](https://github.com/dsrhaslab/lazyfs). LazyFS is a FUSE filesystem that can drop data that was not flushed. If LazyFS is not practical in CI, run it by hand before M5 and record the result in `docs/`.

## Property and unit tests

Cases apply from M1 unless they name another milestone. Mixed bullets identify the later parts explicitly.

- `OPT-DEFAULTS`. Default options, from M1 ([ADR 0027](../decisions/0027-default-admission-budget-is-an-accounting-ceiling.md)). Check that `Options::default` matches [the API defaults](api.md). Changing `pending_byte_budget` must not change queue-count capacity or the group-byte cap.
- `OPT-VALIDATION`. Option validation, from M1. Cover zero queue capacity and byte budget, a group cap just below and exactly at the smallest valid group body, each upper limit, zero roll size, and zero linger. Invalid options return `InvalidOptions` without filesystem mutation. A positive budget below a request's charge opens successfully and gives that request `TooLarge`.
- `ADM-PRECEDENCE`. Admission precedence, from M1. Combine invalid records, impossible size, writer poisoning and exhausted capacity. Check the [specified order](api.md#admission-and-answer-ordering), including after a writer panic or disconnected queue. Outstanding `Pending` handles whose replies are lost return `Indeterminate`; new size-eligible requests get `Poisoned`. No request waits forever, and abandoned result handles do not leak reservations. From M4, include sealing and sealed states.
- `ADM-BUDGETS`. Admission budgets, from M1 ([ADR 0025](../decisions/0025-admission-byte-reservations-live-until-the-answer.md)). Fill queue-count capacity while byte budget remains, then exhaust byte budget while queue capacity remains. Both cases give immediate `Full` and accept no request. Concurrent producers cannot reserve more than the byte budget. A failed queue insertion releases its temporary reservation.
- `ADM-SIZE`. Permanent size refusal and temporary exhaustion, from M1 ([ADR 0028](../decisions/0028-too-large-is-distinct-from-temporary-full.md)). A group-sized submit whose retained-allocation charge exceeds the entire budget gets `TooLarge` even on an empty log. Charge overflow and exceeding the encoded group cap also get `TooLarge`. Check exact-limit eligibility, no write, no offset or revision consumed, and no retained reservation. A size-eligible submit gets `Full` when outstanding charges or a full queue prevent admission, then succeeds after temporary pressure clears. Impossible size fits remain `TooLarge` even while other requests occupy capacity. Cover retries as well as new submits.
- `ADM-RESERVATION`. Admission reservation lifetime, from M1. Pause the writer after dequeue and verify that the request's byte reservation remains held. Match pending retries and verify that their reservations also remain held. Dropping a `Pending` handle must not free an unfinished request's reservation. Check release on successful answers, `KeyConflict`, `PartialRetry`, `WrongRevision`, poison and writer shutdown. Validation and permanent-size rejections retain no reservation. A completed answer releases the reservation even if the caller has not consumed it.
- `ADM-CHARGE`. Retained-allocation charges, from M1 ([ADR 0026](../decisions/0026-admission-charges-retained-allocations.md)). Build equal-content submits with different record-vector and stream, key and payload capacities. They have equal encoded sizes but different admission charges under [the formula](api.md#admission-charge). Cover unused record slots, many small records, result-position storage and actual request-bookkeeping sizes. Retrying or moving a request's buffers must not grow its reserved charge or release it early. Checked arithmetic must refuse an unrepresentable charge without wrapping.
- `CODEC-GROUP`. Group codec round trips and the CRC check value, from M1 ([format.md](format.md)). Footer, seal and locator codec cases start in M4.
- `MODEL`. A full model test, from M2: a plain in-memory model of streams, revisions and keys. Random submits go to the model and to the log, and the results must be equal. This covers R9–R12. The directed revision and retry cases below remain required in M1.
- `KEY-LIFETIME`. Retained-key lifetime ([ADR 0031](../decisions/0031-keys-live-with-retained-segments.md)). In M1, commit keyed records, drop the log, reopen it, and verify that retries return the original positions without another append. In M3, repeat after rolling the original records into an older retained segment, both before and after reopen. M4 must test its chosen lookup mechanism and cache rebuilds against the same lifetime guarantee, and enforce its key-cache memory budget before replacing full scans with summary-based open.
- `HISTORY`. Complete-history retention ([ADR 0037](../decisions/0037-retain-complete-logs-through-current-milestones.md)). In M1, retain committed records and keys across reopen. From M3, repeated rolls preserve every valid history segment, including streams with later records and streams that never appear again. From M4, later footer coverage still does not permit deletion, and sealing and read-only open preserve the complete readable history. Keep these cases distinct from permitted torn-tail and incomplete-new-segment recovery cleanup. No valid history is deleted through a retention policy in M1–M6.
- `REV-EXPECTED`. Expected revisions, from M1 ([ADR 0030](../decisions/0030-expected-revisions-use-empty-and-at.md)). On separate streams with no records, `Empty` succeeds at revision zero and `At(0)` fails with actual revision `None`. On separate streams at revision zero, `Empty` fails with actual revision `Some(0)` and `At(0)` succeeds at revision one. Check successive records in one submit and valid earlier pending submits. A later failing record rejects the entire submit and leaves no offset, revision or key reservation consumed. Fully matching pending and durable retries succeed despite stale expectations.
- `KEY-SCOPE`. Whole-log key scope, from M1 ([ADR 0019](../decisions/0019-idempotency-keys-are-scoped-to-the-whole-log.md), [ADR 0022](../decisions/0022-reject-conflicting-key-reuse-from-m1.md)). Commit a key on stream A, then use the same key and payload on stream B. The second submit gets `KeyConflict` and appends nothing. Check before and after reopen.
- `KEY-CONFLICT`. Conflicting key reuse, from M1. Check a reused key with a different stream or payload against both a durable original and a valid pending original in the current group. Include a multi-record submit with a fresh record before the conflict and a submit with several conflicts. `Pending::wait` returns `KeyConflict` with the first conflicting record's index, even when an expected revision is also wrong. The rejected submit writes no bytes, reserves no fresh key, and changes no watermark or stream head. The next valid append uses the next unused offset and revision. Pending originals and their matched retry waiters still complete normally. Reusing a key with only a different expected revision follows the retry rule.
- `KEY-CONFLICT-CRASH`. Key conflicts across crashes, from M1. Reject a conflicting submit while its original is pending, then crash before or after the group flush. After reopen, only the original can have survived. Retry its keys under oracle checks 1–5 and verify that none of the rejected submit's fresh records appear.
- `KEY-PARTIAL`. Mixed reused and new records, from M1 ([ADR 0023](../decisions/0023-reject-mixed-reused-and-new-records-from-m1.md)). Mix a matching durable or pending record with a fresh keyed record or an unkeyed record, in either order. The whole submit gets `PartialRetry` before expected-revision checks and writes nothing. Check unchanged watermark, stream heads, key reservations, offsets and revisions, and that original submits and their retry waiters still complete. Include several matching records spread across pending and durable state alongside a fresh record. A conflicting key later in a mixed submit must produce `KeyConflict` at its first conflicting index instead of `PartialRetry`. A submit containing only fresh keyed and unkeyed records passes this key classification.
- `KEY-PARTIAL-CRASH`. Partial-retry rejection across crashes, from M1. Reject a mixed submit while a matching original is pending, then crash before or after the original group's flush. Reopen and retry original keys under oracle checks 1–5. No fresh record from the rejected submit can appear.
- `KEY-REPEATED`. Repeated keys within a submit, from M1 ([ADR 0020](../decisions/0020-reject-repeated-keys-within-a-submit.md)). Two records with the same key get `AppendError::Invalid` before admission. Cover identical streams and payloads, different streams, and different payloads. Check that rejection leaves queue capacity, watermark and stream heads unchanged and writes no bytes. The next successful append gets the next unused offset and revision. Multiple unkeyed records pass this validation.
- `RETRY-PENDING`. Exact pending retries, from M1 ([ADR 0021](../decisions/0021-exact-pending-retries-wait-for-the-original.md)). Stage an original multi-record submit and multiple exact retries in one group, including retries with stale expected revisions. Block the flush and verify that neither the original nor its retry waiters return success, and readers cannot see the group's records. After a successful flush and watermark publication, every retry returns the original positions with `deduplicated: true`. Only the original records appear in the group frame, and retries consume no offset or revision.
- `RETRY-REGROUPED`. Regrouped retries, from M1 ([ADR 0024](../decisions/0024-fully-matching-retries-ignore-original-grouping.md)). Write originals with keys `[K1, K2]` and `[K3]`. Retry a subset, reorder records, and combine originals as `[K3, K1]`. Cover all-pending, all-durable, and mixed pending and durable matches, including multiple originals on the same stream. Use stale expected revisions. Success waits for every referenced record to be durable and returns original positions in request order with `deduplicated: true`, even when offsets decrease. Retries add no records, consume no offset or revision, and trigger no extra flush. Repeat with retained records after reopen.
- `RETRY-REGROUPED-FAIL`. Regrouped pending-retry failures, from M1. Match a retry against durable records and several pending originals in the current group. An injected write or flush failure gives that matched retry `Indeterminate`; an unprocessed queued retry gets `Poisoned`. Run the crash oracle against the original new submits, and verify that the regrouped retry added no record.
- `POISON-LOOKUP`. Durable-key lookup failures, from M1. Inject a read error, unexpected EOF, bad CRC and malformed frame while a retry is checked. The lookup request, unfinished new submits in the current group and their retry waiters get `Indeterminate`; queued requests get `Poisoned`. Check both before any group write and with valid new records already buffered. The failing lookup never gives `KeyConflict`, a key miss, or a new append. No later write or flush uses the failed handle.
- `ANSWER-TIMING`. Terminal answer timing, from M1. Buffer a valid new submit and process a `KeyConflict`, `PartialRetry`, `WrongRevision` and all-durable retry before flushing that group. Each result finalizes and releases its reservation without waiting for the flush. Then fail the group and verify that those results stay fixed. The rejected revision's `actual` can include the buffered submit, while `stream_head` still reports the durable head. A matched pending retry continues to wait and gets the group's failure result.
- `RETRY-PENDING-FAIL`. Pending-retry failures and crashes, from M1. Inject a short write, `ENOSPC`, or flush failure into the original group. The original and its matched retry waiters get `Indeterminate`. A retry still queued when poisoning happens gets `Poisoned`. At crash points before and after the flush, reopen and retry the original keys under oracle checks 1–5. Each original keyed record appears exactly once.
- `GROUP-FORMATION`. Group formation, from M1 ([overview](overview.md#the-write-path)). Each group with new records is one frame and one data flush. A submit is never split across groups (R3). A new submit that would pass the byte cap waits for the next group, and no later submit overtakes it. A group with only rejections and fully matched retries causes no frame write and no flush. Check the flushes through the storage call counts.
- `READ-BOUNDED`. Bounded reads and stream heads, from M1. `read` returns records in log order from `from`, at most `max` of them, and never past the watermark, including while a group is held at its flush. `stream_head` returns `None` for a stream with no durable record and reports only the durable view below the watermark. Cover the empty boundary results defined in [api.md](api.md#target-signatures-and-defaults).
- `DROP`. Writer-draining `Drop`, from M1 ([api.md](api.md#target-signatures-and-defaults)). Dropping the last handle commits the submits already queued. `Pending` handles that outlive the log handle get those answers. The lock is released, so a reopen in the same process succeeds and recovers those records. Drop on a poisoned writer, and after the writer thread stopped without answering, still returns and releases the lock. A write or flush failure during the drain gives the unfinished current group `Indeterminate` and queued requests `Poisoned`, under the poison rule.
- `OPEN-FAULTS`. I/O faults during open, from M1. Inject a failure into the directory sync during creation, the parent directory sync, the tail rewrite and its `sync_all`, including `ENOSPC`. Open fails with `Io` and returns no handle. A later open with the fault disabled, then a `PowerLoss`, then another open satisfies oracle checks 1–5.
- `REV-NO-OFFSET`. A rejected revision uses no offset, from M1: the next successful append gets the offset that the rejected one would have had.
- `COUNTER-EXHAUSTION`. Counter boundaries, from M1, under [ADR 0041](../decisions/0041-counter-exhaustion-refuses-without-poisoning.md). Use private fixtures for otherwise impractical counter states. Cover the last assignable offset and revision, whole-submit refusal, expected-revision precedence, unchanged public state and continued writer use. An exhausted epoch refuses open before recovery mutations. Cover a valid maximum epoch followed by an invalid tail.
- `OPEN-DIR`. Directory creation, from M1. Create only the final component under a durable parent. A missing ancestor fails without creating it. Crash after directory creation, segment creation and each directory sync. Before any acknowledgment, both the segment entry in the log directory and the log entry in its parent must be durable. A power loss immediately after successful open preserves the reachable log.
- `OPEN-LAYOUT`. Supported layouts, from M1. A directory with multiple segment files fails with `Io` of kind `Unsupported` before cleanup, truncation or rewrite. From M3, multi-segment fixtures instead exercise full-scan recovery, gaps and crashes during roll. M4 adds footer-based recovery.
- `RECOVERY-FIXTURES`. Recovery on hand-made files, from M1: a zero-filled tail, a length past the end of the file, a bad CRC, a lower epoch, a wrong base offset, and a file name that differs from its header's base offset. Cross-segment log-id mismatches start in M3 under [ADR 0042](../decisions/0042-log-id-comparison-fixtures-start-with-multiple-segments.md). Missing middle segments and crashes during roll also start in M3. Expected outcomes for the cases where a wrong answer loses data:
  - A newest segment longer than a header with a bad header fails open. Open does not delete it. From M3, a segment whose log id differs from the first segment also fails open without deletion.
  - A footer or seal frame with no locator after it (M4) is truncated, because its `fsync` never returned.
  - A record whose payload ends with bytes that form a valid footer, seal frame and locator (M4) leaves the segment active.
  - A locator that survives a power loss while its footer's page is lost (M4) leaves the segment active, and the closing frames are truncated.
  - A clean reopen, then `PowerLossTorn` during open's rewrite, keeps every acknowledged record.
  - A bad frame in a sealed log's final segment (M4) fails with `Corrupt`. Open does not truncate it.
  - A seal on a segment with no group frame (M4), then a failed `sync_data` or a `Kill`, then a reopen and a `PowerLoss`, still gives a sealed log.
- `ROLL-FOOTER`. Early roll for footer size, from M4. The writer rolls when a group's new streams would make the footer pass the frame limit. Ordinary size-based roll starts in M3.
- `READER`. `LogReader`, from M4, on storage that refuses every write, and with two readers open at once. On an unsealed log, it fails with `NotSealed`.

`proptest` is a suitable dev-dependency for deterministic, single-threaded properties, such as codec round trips, charge arithmetic and the M2 model driven one submit at a time. Commit its regression files. Use the seeded trace harness above, not `proptest` shrinking, for runs with several threads.

## Fuzzing (M5)

- `FUZZ-DECODER`. A `cargo fuzz` target for the frame decoder.
- `FUZZ-RECOVERY`. A `cargo fuzz` target for recovery on an arbitrary active segment.

A target must never panic. It must either return the valid prefix or return an error. `cargo fuzz` needs a nightly toolchain beside the pinned one, and its targets are a separate crate. They reach the decoder and recovery scan through items compiled under `cfg(fuzzing)`, as the [storage trait](#storage-trait) section describes.

## Benchmarks (M1 for the core, M6 for the server)

The benchmark measures the [PRD targets](../prd.md#footprint-and-performance-targets). For each run, report:

- the hardware, the filesystem, the kernel, and whether the device has a volatile write cache;
- the `fsync` latency of the device alone (p50, p99), measured first, with the log's pattern: append to a growing file, then `fdatasync`. Each append grows the file, so `fdatasync` must also commit the new size, which a rewrite in place does not measure;
- append throughput and p50/p99/p999 latency;
- the configured admission byte budget, admission refusals and peak outstanding charges;
- the group size distribution (submits and bytes for each `fsync`);
- record count, segment count and segment sizes at the start and end of each run, including reopen measurements;
- open time, seal-to-exit time and idle resident set, where they apply.

[ADR 0036](../decisions/0036-measure-the-existing-defaults-before-tuning.md) requires a run under the applicable [API defaults](api.md) before tuning experiments. Record all active options. Report changed settings as separate named variants with their exact configuration and the same correctness and durability contract. Preserve the baseline results. A proposed default change includes its measurement evidence, rationale and corresponding code and documentation update. M1 has no roll, so it does not measure or tune roll behavior; that setting becomes effective from M3.

An M1 segment grows for the duration of the run. Record its actual size and scan cost instead of assuming that the later roll setting limits it. Benchmark code creates no sparse index in M1. The segment roll and sparse-index position bounds become relevant with their later features.

A benchmark on macOS is for development only. The targets are for Linux.

For M1, the Linux report is required by the completion gate even if the provisional performance targets are missed. Include each applicable target comparison and explain any miss or condition that prevents a comparable result. Keep missing evidence separate from measured misses.

## Deployment storage qualification (before M6)

[ADR 0032](../decisions/0032-qualify-deployment-storage-before-m6.md) requires a record in `docs/` for the actual guest stack before its M6 benchmark and production durability claims. This is separate from M1's mandatory tests, syscall checks and Linux benchmark.

[volume-requirements.md](volume-requirements.md) ([ADR 0040](../decisions/0040-volumes-meet-testable-minimum-requirements.md)) defines the requirements V1–V11, their verification cases, the testability rules, the power-cut harness and the [record format](volume-requirements.md#qualification-record). The record identifies every layer of the stack and gives one row for each requirement, with its evidence class, commands, known-bad control result, result and limitations.

Evidence must apply to the identified setup. A syscall trace alone does not establish a requirement. An unverified or contradicted MUST requirement blocks qualification. M6 evidence names the qualified stack used for its guest benchmark. Changes to that stack rerun the cases in the [requalification table](volume-requirements.md#requalification-triggers). No qualification is claimed by this checklist itself.

## Surviving-volume handoff (before M6)

[ADR 0033](../decisions/0033-deployment-owns-volumes-and-reader-handoff.md) assigns the volume lifecycle and reader handoff to the deployment layer. Before M6 deployment validation, record the surviving volume, its writer and reader mount paths, the concrete reader service or tool, and the handoff procedure. The procedure must respect the single-writer contract, including one writing kernel ([V5](volume-requirements.md#v5-one-writing-kernel)), and identify access for writable recovery when the guest exits with an unsealed log. `HANDOFF` is also the case for [V6](volume-requirements.md#v6-the-volume-outlives-the-guest), so it destroys the guest through the platform's real teardown path.

`HANDOFF`. Integration evidence must show that acknowledged records remain available on the surviving volume after guest exit and can be read by the selected component. Cover sealed history through `LogReader` and an unsealed crash log through writable `SegmentLog::open` recovery. Using `LogReader` afterward requires explicitly sealing the recovered log; the test must state that policy rather than silently finalizing a resumable log.

The integration record references the actual-stack qualification in ADR 0032. These checks do not select a provider, concrete path or reader implementation for M1, and do not claim that a deployment handoff has been exercised yet.

## M6 transport order and conformance

[ADR 0034](../decisions/0034-m6-builds-unix-socket-before-vsock.md) requires Unix-socket integration first, followed by vsock using the same protocol. Record Go-client append and read results over both transports before marking M6 complete. Reuse protocol conformance cases for retries, expected revisions and admission outcomes, with the same core contract for both adapters.

Keep the conformance cases as data that both transport runs share, not as code copied for each transport. The Go client runs that data. The core's crash and recovery behavior stays in the Rust scenarios; transport cases check how the server maps it onto the protocol:

- `TRANSPORT-CONFORMANCE`. Append and read, expected revisions, retries and admission outcomes give the core's results, mapped to protocol errors without losing the difference between a known rejection and an unknown outcome.
- `TRANSPORT-UNKNOWN`. A connection that closes after a request is sent and before its reply is an unknown outcome for the client, never a rejection.
- `TRANSPORT-RESTART`. After the server process is killed with `SIGKILL` and restarted on the same directory, the Go client resolves its fully keyed unknown outcomes by retry under oracle check 4.

The vsock evidence must exercise the selected guest topology. Passing Unix-socket cases does not establish vsock connectivity. A host vsock loopback can run the shared cases in CI, but it does not replace evidence from the guest topology. Record the protocol version, client and server commands, topology and results needed to reproduce each path. Protocol bytes and framing will be selected in their own M6 ADR; this checklist does not select them or claim either transport is implemented.
