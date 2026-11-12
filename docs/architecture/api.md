# API sketch

Status: provisional. This is the M1–M4 target for `segment-log-core`. Names can change during implementation. A change in behavior needs an ADR. This file is the only source for the public signatures and the default values.

## Milestone availability

The sketch below describes the cumulative target. Implement each behavior in its milestone; later features are not M1 requirements.

| From milestone | Behavior |
| --- | --- |
| M1 | `SegmentLog::open`, `append`, `watermark`, bounded `read`, `stream_head`, `Pending::wait`, and writer-draining `Drop`. Expected revisions, key matching, recovery, poisoning and admission limits follow the accepted rules below. |
| M3 | Segment roll and the effect of `roll_bytes`, multi-segment recovery and cross-segment log-id comparison under [ADR 0042](../decisions/0042-log-id-comparison-fixtures-start-with-multiple-segments.md). M1 remains single-segment without roll. |
| M4 | `seal`, `LogReader`, sealed-log read-only open, footers and locators, and retained-key lookup through each sealed segment's key filter and key index ([ADR 0048](../decisions/0048-sealed-segments-carry-a-key-filter-and-index.md)). |

## Target signatures and defaults

```rust
use std::path::Path;
use std::time::Duration;

pub type LogOffset = u64;
pub type Revision = u64;

pub struct Options {
    /// How long the writer waits for more submits after the first one.
    pub linger: Duration,          // default 1 ms
    /// Largest group body in bytes. A submit that cannot fit alone gets TooLarge.
    pub group_byte_cap: usize,     // default 1 MiB; at most the group body limit in format.md
    /// From M3, roll the active segment when it reaches this size.
    pub roll_bytes: u64,           // default 64 MiB; at most the roll size limit in format.md
    /// Submits the accept queue holds before append returns Full.
    pub queue_capacity: usize,     // default 4096
    /// Accounting ceiling for accepted, unanswered requests, including retry waiters.
    pub pending_byte_budget: usize, // default 16 MiB; retained-allocation charges
}

impl Default for Options { /* the defaults above */ }

/// One log directory, open for writing or read-only after seal.
/// Send + Sync: share it with Arc and call append from many threads.
pub struct SegmentLog { /* … */ }

impl SegmentLog {
    /// Creates the directory or the log if needed, takes the lock, recovers,
    /// makes the kept prefix durable, and starts the writer thread.
    pub fn open(dir: impl AsRef<Path>, options: Options) -> Result<SegmentLog, OpenError>;

    /// Puts the submit in the accept queue and returns at once.
    /// Never waits for queue capacity or admission byte budget.
    /// Rejects repeated keys within the submit with Invalid before queuing it.
    /// Returns TooLarge for an impossible size fit or charge overflow.
    pub fn append(&self, submit: Submit) -> Result<Pending, AppendError>;

    /// The offset just past the last durable record.
    pub fn watermark(&self) -> LogOffset;

    /// Records in log order from `from`, up to `max` records, never past the watermark.
    /// Returns an empty vector if max is zero or from is at or past the watermark.
    pub fn read(&self, from: LogOffset, max: usize) -> Result<Vec<Record>, ReadError>;

    /// The latest durable revision and offset of one stream, from the tail map.
    pub fn stream_head(&self, stream: &[u8]) -> Option<StreamHead>;

    /// Stops admission, commits the queued submits, writes the footer, the
    /// seal frame and the locator, and makes them durable. Takes &self because other threads
    /// can hold the same Arc: from the moment seal starts, their append gets
    /// Sealed. read, watermark and stream_head keep working on the sealed log.
    /// A second call returns Ok(()).
    pub fn seal(&self) -> Result<(), SealError>;
}

/// Dropping the last handle stops admission, lets the writer commit what is
/// already queued, joins the writer thread, and releases the lock. If seal was
/// not called, the log is not sealed.
impl Drop for SegmentLog { /* … */ }

/// A read-only view of a sealed log (ADR 0016). Takes no lock and writes
/// nothing, so it works on a read-only volume and many can open at once.
/// Send + Sync. M4.
pub struct LogReader { /* … */ }

impl LogReader {
    /// Checks the headers and reads the segments without changing any file.
    /// Fails with NotSealed if the log has no seal frame.
    pub fn open(dir: impl AsRef<Path>) -> Result<LogReader, OpenError>;
    pub fn watermark(&self) -> LogOffset;
    pub fn read(&self, from: LogOffset, max: usize) -> Result<Vec<Record>, ReadError>;
    pub fn stream_head(&self, stream: &[u8]) -> Option<StreamHead>;
}

pub struct Submit {
    /// At least one record. New records commit all or none.
    /// A fully deduplicated retry only confirms existing durable records.
    /// Present keys must be distinct within the submit.
    pub records: Vec<NewRecord>,
}

pub struct NewRecord {
    pub stream: Vec<u8>,                 // within the stream id limit in format.md
    pub expected: ExpectedRevision,
    /// Scoped to the whole log. Within the key limit in format.md when present.
    pub key: Option<Vec<u8>>,
    pub payload: Vec<u8>,
}

/// M1 preconditions (ADR 0030). There is no unconditional Any variant.
pub enum ExpectedRevision {
    /// The stream has no records. A successful append gets revision 0.
    Empty,
    /// The stream's current last revision is exactly this value.
    At(Revision),
}

/// The result handle for a submit, including a matched pending retry.
pub struct Pending { /* … */ }

impl Pending {
    /// Blocks until the writer answers this submit. If the writer thread stops
    /// without an answer (a panic), returns Indeterminate.
    pub fn wait(self) -> Result<Committed, CommitError>;
}

pub struct Committed {
    /// One entry for each record, in submit order.
    /// A retry's original positions need not have increasing offsets.
    pub records: Vec<Position>,
    /// True when the submit reused original records and appended nothing.
    /// Those records are durable before this result is returned.
    pub deduplicated: bool,
}

pub struct Position {
    pub offset: LogOffset,
    pub revision: Revision,
}

pub struct Record {
    pub offset: LogOffset,
    pub stream: Vec<u8>,
    pub revision: Revision,
    pub key: Option<Vec<u8>>,
    pub payload: Vec<u8>,
}

pub struct StreamHead {
    pub revision: Revision,
    pub offset: LogOffset,
}

/// Returned at once by append. Nothing was queued.
pub enum AppendError {
    Full,                  // size-eligible submit lacks current queue or byte capacity
    TooLarge,              // exceeds group cap or total byte budget, or charge overflows
    Invalid(&'static str), // empty submit, repeated key, or a field outside its limits
    Sealed,                // seal has started or finished
    Poisoned,              // an earlier I/O error; reopen the log
}

/// Returned by Pending::wait.
pub enum CommitError {
    /// A new record would exhaust the log-offset or stream-revision space.
    /// Nothing was appended and the writer remains usable.
    Exhausted,
    /// Record `index` had the wrong expected revision. `actual` includes earlier
    /// valid pending records and is None when the stream has no records.
    /// Nothing was appended. No offset was used.
    WrongRevision { index: usize, actual: Option<Revision> },
    /// First record in submit order whose key matches a pending or durable
    /// record with a different stream or payload. The whole submit is rejected.
    KeyConflict { index: usize },
    /// A matching pending or durable record is mixed with a new or unkeyed
    /// record. The whole submit is rejected. KeyConflict takes precedence.
    PartialRetry,
    /// An I/O error happened while this submit was in flight. It might be
    /// durable. Reopen the log. Fully keyed requests can resolve survivors
    /// through retry; mixed and unkeyed requests need reconciliation.
    /// Also returned to a retry matched to that in-flight submit.
    Indeterminate,
    /// The writer was poisoned before it wrote this submit. Nothing was
    /// written. Reopen the log and retry.
    Poisoned,
}

pub enum OpenError {
    Exhausted,                                    // no next writer epoch; no recovery mutations
    Locked,                                        // another process holds LOCK
    InvalidOptions(&'static str),                  // an option outside its limits
    Corrupt { segment: LogOffset, position: u64 }, // bad frame in a sealed segment
    LogIdMismatch { segment: LogOffset },
    Gap { segment: LogOffset },                    // does not start where the one before it ends
    NotSealed,                                     // LogReader::open on a log with no seal frame
    UnsupportedVersion(u16),
    Io(std::io::Error),
}

pub enum ReadError { Corrupt { segment: LogOffset, position: u64 }, Io(std::io::Error) }
pub enum SealError { Poisoned, Io(std::io::Error) }
```

## Behavior notes

- **Admission byte budget.** M1 retains the queue-count limit and adds `pending_byte_budget` ([ADR 0025](../decisions/0025-admission-byte-reservations-live-until-the-answer.md)). A size-eligible submit reserves its [retained-allocation charge](#admission-charge) before acceptance; unavailable current queue capacity or byte budget gives `Full` immediately. Every accepted request, including a retry, holds its reservation while queued, grouped or waiting for a pending match. Dequeueing and retry matching do not release it. Release happens when the request is answered, without waiting for the caller to consume the result. Failed admission retains no reservation. Terminal rejection, success, poisoning and writer shutdown release the reservation. Dropping `Pending` does not release a request that is still being processed. The default is an accounting ceiling without a budget-sized preallocation ([ADR 0027](../decisions/0027-default-admission-budget-is-an-accounting-ceiling.md)).
- **Permanent size refusal.** From M1, `append` returns `TooLarge` if the encoded submit cannot fit in one group, its charge exceeds the entire configured admission budget, or charge arithmetic overflows ([ADR 0028](../decisions/0028-too-large-is-distinct-from-temporary-full.md)). Check size eligibility before reserving capacity or queuing. Equality with a limit is allowed if enough capacity is currently available. `Full` is only for temporary capacity exhaustion of a size-eligible request. Both errors are immediate, retain no reservation, consume no offset or revision, and do not poison the writer. Retrying an unchanged `TooLarge` request under unchanged limits cannot succeed merely because other requests finish. The rule also applies to retries.
- **Key scope.** A key is scoped to the whole log ([ADR 0019](../decisions/0019-idempotency-keys-are-scoped-to-the-whole-log.md)). Lookup uses the key bytes alone. Reusing a retained key on a different stream follows the key-conflict rule, even if the payload matches. Callers must choose keys across all streams in the log.
- **Key lifetime.** A key remains valid while the segment holding its record is retained, including after roll and reopen ([ADR 0031](../decisions/0031-keys-live-with-retained-segments.md)). Rolling or sealing does not expire it. Segment deletion expires the key under R13. M1 and M3 rebuild keys by scanning every segment. From M4, open scans only the newest segment, and the writer finds older keys through each sealed segment's key filter and key index ([ADR 0048](../decisions/0048-sealed-segments-carry-a-key-filter-and-index.md)). No option controls this: the filter density is a format constant, and key-cache memory has no fixed upper limit.
- **Repeated keys within a submit.** From M1, `append` rejects the whole submit with `Invalid` before queuing it if two records carry the same key bytes ([ADR 0020](../decisions/0020-reject-repeated-keys-within-a-submit.md)). The rule applies regardless of stream, payload or expected revision. Nothing is written, and no offset or revision is consumed. Multiple records without a key are allowed, subject to the other checks.
- **Same bytes.** For a key match, "same bytes" means the same stream id and the same payload. The expected revision is not compared, because a retry usually carries a stale one. For durable records, the writer reads and validates the original frame before comparing its record. For pending records, it compares against the buffered original submit. It does not trust a hash: a record whose key bytes differ is a false hit, never a match or a `KeyConflict`. A read error, unexpected EOF, bad CRC, malformed frame or malformed key index page during this writer lookup poisons the writer under [ADR 0004](../decisions/0004-io-errors-poison-the-writer.md). It is never a missing key or a `KeyConflict`. The unfinished current group, including the lookup request and its dependent retry waiters, gets `Indeterminate`; queued requests get `Poisoned`.
- **Conflicting key reuse.** From M1, a key that identifies a pending or durable record with a different stream or payload rejects the whole new submit with `CommitError::KeyConflict { index }` ([ADR 0022](../decisions/0022-reject-conflicting-key-reuse-from-m1.md)). The index names the first conflicting record in submit order. This check precedes expected revisions. The rejected submit writes nothing and consumes no offset or revision. It leaves the original submit and its retry waiters unchanged and does not poison the writer. A pending original need not be flushed to reject the conflict.
- **Fully matching retry.** From M1, if every record has a distinct key and every key matches a pending or durable record with the same stream and payload, the submit is a fully deduplicated retry ([ADR 0024](../decisions/0024-fully-matching-retries-ignore-original-grouping.md)). It may select subsets, reorder records, combine original submits, or mix pending and durable matches. Expected revisions are ignored. It appends nothing and consumes no offset or revision. Success waits until every referenced record is durable and covered by the published watermark, then returns original positions in request order with `deduplicated: true`. Offsets need not increase. A record without a key is never deduplicated.
- **Mixed reused and new records.** From M1, a submit with at least one matching pending or durable record and at least one new or unkeyed record is rejected with `CommitError::PartialRetry` ([ADR 0023](../decisions/0023-reject-mixed-reused-and-new-records-from-m1.md)). The whole submit writes nothing and consumes no offset or revision. Existing submits and their retry waiters are unchanged. The rejection neither poisons the writer nor waits for a matching pending original to become durable. Any `KeyConflict` takes precedence; both errors precede expected-revision checks. If no record matches an existing key, the submit follows ordinary expected-revision checks, including when fresh keyed records are mixed with unkeyed records.
- **Pending retry outcome.** Normal admission checks and queue limits apply, and `append` does not wait for durability. A matched retry referencing pending records waits for their existing group's successful flush and watermark publication, without another append or flush solely for the retry ([ADR 0021](../decisions/0021-exact-pending-retries-wait-for-the-original.md), [ADR 0024](../decisions/0024-fully-matching-retries-ignore-original-grouping.md)). If a referenced in-flight group fails, the matched retry gets `Indeterminate`, even if some matches were already durable. A request still queued when the writer is poisoned gets `Poisoned`. All-durable matches need no new flush.
- **Atomic writes and retry confirmation.** New records in a submit commit all or none. A fully deduplicated retry only confirms that its referenced records are durable. It creates no new transaction and does not assert that the records originally committed atomically together ([ADR 0024](../decisions/0024-fully-matching-retries-ignore-original-grouping.md)).
- **Milestones.** M1 includes admission byte budgeting, repeated-key rejection within a submit, fully matching retries against pending and durable records regardless of original grouping, and `KeyConflict` and `PartialRetry`. M2 adds full model-based verification of revisions and keys.
- **Initial defaults and tuning.** Keep the defaults above as the configuration for the first benchmark baseline ([ADR 0036](../decisions/0036-measure-the-existing-defaults-before-tuning.md)). Benchmark defaults first, then report any changed options as separately named experimental configurations. Default changes require measurement evidence, a recorded reason and matching code and documentation updates in the same change. M1 has no roll; the roll-size setting becomes effective from M3. Queue-count and admission-budget defaults remain as already agreed.
- **Waiting for future records.** Waiting cursors live above the core ([ADR 0035](../decisions/0035-waiting-cursors-belong-above-the-core.md)). M1 exposes `watermark()` and bounded `read()` without a native watermark-wait or subscription API. The consumer owns cursor positions, polling or scheduling, cancellation and timeouts. `Pending::wait` still waits for one submit's result and is not a general waiting cursor. A future notification primitive needs a separate decision based on a concrete consumer need.
- **Expected revisions.** M1 uses `Empty` and `At(n)` without `Any` ([ADR 0030](../decisions/0030-expected-revisions-use-empty-and-at.md)). `Empty` requires no records in the stream and assigns the first revision zero. `At(n)` requires the current last revision to equal `n` and assigns the next revision `n + 1`. A mismatch rejects the entire submit with `WrongRevision`, the failing record's index, and the current revision or `None` for a stream with no records. It consumes no offset or revision and leaves no pending changes. A fully matching retry ignores expected revisions after checking keys.
- **Order of checks.** Admission follows the [order below](#admission-and-answer-ordering). The writer checks keys for the whole submit, with `KeyConflict` before `PartialRetry`. For new submits, it then checks expected revisions, record by record. Records in the same submit and earlier pending submits count, in order of admission (R12). For a second record to the same stream in one submit, use `At(revision of the first)`.
- **A sealed log** opens read-only. `append` returns `Sealed`. `SegmentLog::open` takes the lock for the tail rewrite of open step 8, then releases it, as `seal` does, so several handles can open a sealed log at once. A failed seal leaves the writer poisoned: a later `seal` and every `append` return `Poisoned`. Every seal I/O failure is `Poisoned`, so `SealError::Io` has no producer today.
- **Directory and volume ownership.** The deployment or runner provisions, mounts and preserves the volume, supplies the directory to `SegmentLog::open`, and arranges the post-guest reader handoff ([ADR 0033](../decisions/0033-deployment-owns-volumes-and-reader-handoff.md)). The core validates, recovers and reads files in that directory. Concrete mount paths and the reader component are pre-M6 integration choices. Required writable recovery follows the existing API rather than an automatic deployment action inside the core.
- **Complete-history retention.** M1–M6 retain all valid history segments without segment-level retention or automatic expiry ([ADR 0037](../decisions/0037-retain-complete-logs-through-current-milestones.md)). Deployment owns whole-log archival and expiry. The core gains no retention option or deletion endpoint. Existing recovery cleanup remains unchanged and does not authorize deletion of valid history.
- **A reader after the guest is gone** uses `LogReader`, not `SegmentLog::open`, so that it changes nothing on the volume. It opens only a sealed log. A directory with no segment, or whose newest segment does not end in a complete seal run, gives `NotSealed`. A missing directory gives `Io`. To read an unsealed log, open it with `SegmentLog::open` on a writable volume and call `seal`.
- **Corrupt is for a human to resolve.** `OpenError::Corrupt { segment, position }` names the file and the byte where open stopped. Open never repairs it, because it cannot tell a torn tail from a bad frame in a sealed segment, and truncating a sealed segment drops acknowledged records. From M4, frame CRCs cover the log id, so a caller's payload cannot cause it ([ADR 0049](../decisions/0049-frame-crcs-cover-the-log-id.md)). Repair is manual for now. Copy the directory, inspect the segment at that position, and truncate the copy only if the bytes there are a torn tail. A repair tool is future work ([issue #1](https://github.com/ironflowhq/segment-log/issues/1)).
- **Options.** `open` checks the [option limits below](#option-validation) before creating or changing files. An invalid option returns `InvalidOptions`.
- **Thread model.** One writer thread, owned by `SegmentLog`. Callers never write files.

## Admission and answer ordering

[ADR 0038](../decisions/0038-m1-admission-and-recovery-boundaries.md) records the boundary choices. `append` checks in this order:

1. Structural validation, including empty submits, field limits and repeated keys. Failure returns `Invalid`.
2. Standalone [encoded group-body size](format.md#group-record-entry) and [admission charge](#admission-charge). An impossible fit or arithmetic overflow returns `TooLarge`.
3. Writer state. An observed poisoned or stopped writer returns `Poisoned`, including after a writer panic. Otherwise, a sealing or sealed log returns `Sealed` from M4.
4. Current queue and byte capacity. A size-eligible request that lacks either returns `Full`. Successful admission reserves the charge and enqueues the request. Failed admission retains no reservation.

The state check also covers a disconnected queue. Admission concurrent with writer failure can either refuse with `Poisoned` or return a `Pending` whose result reports the failure. It cannot leave a result waiting forever. An existing `Pending` whose writer stops without answering still returns `Indeterminate`, as specified above. Detecting writer failure releases all outstanding admission reservations, including when result handles have been dropped.

The writer finalizes `KeyConflict`, `PartialRetry` and `WrongRevision` when classification finishes. A fully matching retry with only durable matches succeeds after validation against the published watermark. These results do not wait for linger, a new group write or another flush. They release their reservations when finalized, and a later group failure cannot change them. A retry with pending matches still waits for those matches' durability result.

Revision checks use a writer view that includes earlier valid pending records and earlier records in the same submit. `WrongRevision.actual` reports that view at the failing record. Public `stream_head` reports only the durable view below the watermark, so it can temporarily report an older revision. An earlier pending group can fail after the rejection. Neither the reported `actual` nor the rejection acknowledges those pending records.

## Retry after an unknown outcome

Give every record a distinct key when the caller needs to resolve an unknown append outcome through deduplication. Preserve the keys, stream ids, payloads and original expectations on replay. A fully matching retry confirms the original positions even with stale expectations. If no key survived, normal expected-revision checks apply and can return `WrongRevision`. A known terminal rejection did not append new records and carries no promise that an unchanged replay will succeed.

A mixed keyed and unkeyed submit is allowed as a new write. If it survives, an exact replay gets `PartialRetry`. That response alone does not prove the whole original survived: it also occurs when the keyed record exists but the unkeyed record was never written. A fully unkeyed survivor has no deduplication path. An exact replay with its original expectations fails with `WrongRevision`; changing the expectations to the new head can append the payload again. Fresh keys also identify a new operation, not a resolution of the original outcome.

## Option validation

- `queue_capacity` and `pending_byte_budget` must each be at least 1.
- `group_byte_cap` must fit at least the smallest valid group: one record with a minimum-length stream id, no key and an empty payload. Its upper bound is the group-body limit in [format.md](format.md#limits). The [size formula](format.md#group-record-entry) defines the lower bound without a second copy of field widths.
- `roll_bytes` must be nonzero and at most the segment roll size limit in [format.md](format.md#limits). M1 validates the option, but roll behavior starts in M3.
- Zero `linger` is valid. It disables deliberate waiting for more work. The writer drains already queued submits until the queue is empty or the next new submit would exceed the group-body cap. It does not wait for another submit, and a submit admitted after it started to drain starts the next group ([ADR 0046](../decisions/0046-linger-runs-from-the-first-submits-admission.md)). Classification and deferral follow the [write path](overview.md#the-write-path). [ADR 0043](../decisions/0043-zero-linger-and-empty-read-boundaries.md) records this choice.

A positive byte budget smaller than a request's charge is valid configuration. That request gets `TooLarge`; opening a log does not imply that every valid record shape fits its budget.

## Opening a directory

`open` creates only the final directory component. Its parent and ancestor entries must already exist durably. A missing ancestor returns `Io`; the core does not create it recursively. The [recovery sequence](overview.md#open-and-recovery) syncs the log directory and its parent before publishing records.

From M3, `open` recovers a directory with any number of segment files. In M3, it scanned every segment in full. From M4, it scans only the newest segment in full. From each other segment, it reads only the header, the locator, the footer and the key filter ([open step 5](overview.md#open-and-recovery)). The first `read` that needs a sealed segment then reads that segment's frame headers once. M1 and M2 accepted at most one segment file. After taking the writer lock, they rejected a directory with multiple segment files with `Io` whose kind is `Unsupported`, before segment cleanup or recovery writes. M4 adds footer-based open and read-only sealed logs. From M4, `open` also reads each sealed segment's key filter, unless the log is sealed. `LogReader::open` reads no key filter.

## Counter exhaustion

[ADR 0041](../decisions/0041-counter-exhaustion-refuses-without-poisoning.md) defines exhaustion as a known refusal. The writer returns `CommitError::Exhausted` if a new submit would require an unrepresentable stream revision or next unused log offset. It writes nothing for that submit, consumes no offset or revision, releases the reservation, and remains usable. Fully matching retries still return existing positions without advancing counters.

Key classification keeps its existing precedence. For new records, expected-revision and revision-exhaustion checks run in request order. The first failing check decides the result. After those checks and the group-cap decision, the writer checks whether the entire new submit fits the remaining offset space. A refused submit leaves no pending changes. A later smaller submit may still fit.

The last representable stream revision is `u64::MAX`. The watermark must remain representable, so the last assignable record offset is `u64::MAX - 1`. A writer epoch may equal `u64::MAX`, but a later open cannot increment it. If the highest epoch in the valid kept prefix cannot be incremented, `open` returns `OpenError::Exhausted` before truncation, tail rewrite or recovery syncs. A sealed log is the exception: it starts no writer and needs no next epoch, so `open` opens it read-only at any epoch. Lock acquisition and layout validation still precede the recovery scan.

## Admission charge

[ADR 0026](../decisions/0026-admission-charges-retained-allocations.md) defines retained-allocation accounting. Before admission, calculate:

```text
charge = bookkeeping
       + records.capacity() * size_of::<NewRecord>()
       + sum of stream.capacity(), key.capacity() when present, and payload.capacity()
         for every initialized record
       + records.len() * size_of::<Position>()
```

`bookkeeping` is the explicit per-request allowance for the library-owned envelope, reply metadata and possible retry waiter, excluding allocations already counted above. Derive it from the concrete types' sizes. Any additional request-owned copies or buffers must be included in those allowances or added to the charge. The implementation defines the allowance alongside its concrete request types; type changes must update the accounting.

Unused record slots and unused byte-buffer capacity are charged. Result storage and a possible retry waiter are reserved even if a request will eventually be rejected or its result handle is dropped. Use checked multiplication and addition. An unrepresentable charge gets `TooLarge` before acceptance ([ADR 0028](../decisions/0028-too-large-is-distinct-from-temporary-full.md)).

The charge is fixed until the request is answered. It does not decrease when data is moved or freed during processing. It varies with capacity and platform type sizes. It does not include caches, shared group-encoding buffers, allocator overhead, opaque standard-library channel internals, or results retained after the answer. It is not a limit on total process memory.

## Implementation notes

- The core crate uses only `std` in M1. A new dependency needs an ADR ([ADR 0012](../decisions/0012-no-log-crate-dependency.md)). Dev-dependencies for tests and benches do not.
- Accept queue: `std::sync::mpsc::sync_channel` with `try_send` gives "refuse when full" without a dependency. Byte reservation must precede acceptance, and failed queue insertion must release it.
- Lock: `std::fs::File::try_lock` (stable since Rust 1.89, an advisory lock).
- Flush: `File::sync_data` for segments, and `sync_all` on an opened directory handle for directory entries. [ADR 0005](../decisions/0005-flush-calls-and-platforms.md) gives the mapping to system calls and what M1 must still verify.
- All file I/O goes through a small storage trait, so that the tests can use simulated storage ([testing.md](testing.md)).
- Reads on real storage: positioned reads on a shared handle (`std::os::unix::fs::FileExt::read_at`), so a reader never waits for a group flush. This limits the core to Unix targets, which are the supported platforms under [ADR 0005](../decisions/0005-flush-calls-and-platforms.md).
- Log id: random bytes from the storage trait, as many as the `log_id` width in format.md. `std` has no public random source, and the simulated storage must make them deterministic from its seed.
- CRC-32C: software tables that take eight input bytes per step are enough for M1. A version on processor instructions needs `unsafe` or a dependency, so it waits for an ADR and a measurement.
