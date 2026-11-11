# Glossary

Use each term with this meaning only. If you need another meaning, use another word.

| Term | Meaning |
| --- | --- |
| **Segment log** | The whole component: one directory that holds one active segment and zero or more sealed segments. Do not call it "the WAL" or "the log file". |
| **Segment** | One append-only file in the directory. Its name is its base log offset. |
| **Active segment** | The segment the writer appends to: the newest segment, unless that segment is sealed. It is the only segment recovery can truncate. |
| **Sealed segment** | A segment that the writer no longer appends to. It is immutable. Recovery never truncates it. |
| **Roll** | Stop appending to the active segment and create a new active segment. The old one becomes sealed. |
| **Seal (the log)** | The dispose path: stop admission, commit the last submits, write the seal frame, make it durable, close. A sealed log opens read-only. Different from a roll, which seals one segment and keeps the log open. |
| **Record** | One event: stream id, stream revision, optional idempotency key, payload. |
| **Submit** | One `append` call from a caller, holding one or more records. It is the atomic unit for new writes. A fully deduplicated retry confirms existing durable records and appends nothing ([ADR 0024](decisions/0024-fully-matching-retries-ignore-original-grouping.md)). |
| **Group** | The records of one or more submits that the writer writes as one frame and makes durable with one `fsync`. A group is not a transaction. |
| **Frame** | The on-disk form of a group, a key filter, a key index, a footer, a seal, or a locator: length, CRC (cyclic redundancy check), body. See [format.md](architecture/format.md). |
| **Linger** | How long the writer waits for more submits before it writes a group. It runs from the admission of the group's first submit ([ADR 0046](decisions/0046-linger-runs-from-the-first-submits-admission.md)). |
| **Byte cap** | The largest encoded group body, in bytes, including its body header and record entries. It excludes the frame envelope. The writer writes the group when the next new submit would go over it. |
| **Admission byte budget** | The limit on byte charges reserved for accepted, unanswered requests, including queued submits, writer groups and matched retry waiters. A reservation lasts until the request is answered ([ADR 0025](decisions/0025-admission-byte-reservations-live-until-the-answer.md)). |
| **Admission charge** | A request's retained-allocation capacities plus explicit bookkeeping and result allowances under [the API formula](architecture/api.md#admission-charge). It includes unused input capacity and stays fixed until the answer. It is separate from encoded size and total resident memory ([ADR 0026](decisions/0026-admission-charges-retained-allocations.md)). |
| **Log offset** | The index of a record across the whole log, starting at 0. Assigned by the writer. Monotonic. The tail cursor. |
| **Base offset** | The log offset of the first record in a segment or a group. |
| **File position** | The byte position of a frame or record inside one segment. Not an identity that callers store. |
| **Stream** | A named sequence of records inside the log, for example one entity or one run. |
| **Stream id** | The bytes that name a stream. |
| **Stream revision** | The index of a record inside its stream, starting at 0. |
| **Expected revision** | A new record's stream-state precondition. M1 uses `Empty` for a stream with no records and `At(n)` for a stream whose last revision is `n`. There is no unconditional `Any`. The precondition is not stored ([ADR 0030](decisions/0030-expected-revisions-use-empty-and-at.md)). |
| **Idempotency key** | The caller's dedupe token for a record, scoped to the whole log ([ADR 0019](decisions/0019-idempotency-keys-are-scoped-to-the-whole-log.md)). |
| **Watermark** | The log offset just past the last durable record. The writer moves it only after an `fsync` returns success. Readers and acknowledgments never go past it. |
| **Waiting cursor** | A consumer-owned position and policy for waiting until new durable records can be read. It lives above the core and is distinct from waiting for a submit's result through `Pending::wait` ([ADR 0035](decisions/0035-waiting-cursors-belong-above-the-core.md)). |
| **Acknowledgment (ack)** | The success result of a submit. Its records are durable, if the storage meets its [contract](architecture/overview.md#storage-contract). A deduplicated retry can confirm records from several original groups. |
| **Indeterminate** | The result when the writer cannot tell whether a submit is durable, for example after an `fsync` error or a lost writer reply. Reopen before resolving the outcome. Fully keyed requests can confirm survivors by retry; [mixed and unkeyed requests](architecture/api.md#retry-after-an-unknown-outcome) have narrower guarantees. |
| **Poisoned** | The writer's state after a write-path I/O or integrity failure, or after the writer stops unexpectedly. It accepts no more submits until the log is reopened. |
| **Torn tail** | Bytes at the end of the active segment that do not form a complete, CRC-valid frame. Recovery removes them. |
| **Writer epoch** | A number that each opening writer gets: one more than the highest epoch in any segment ([ADR 0047](decisions/0047-writer-epoch-from-every-segment.md)). Every frame stores it, inside the span that the CRC covers. |
| **Log id** | A random id that the log gets at creation. Every segment header stores it. |
| **Footer** | A frame near the end of a sealed segment. It lists each stream in that segment: stream id, last revision, last log offset, last file position. |
| **Key filter** | A frame that a roll writes before the footer (from M4). It answers "this key is certainly not in this segment" or "maybe" for an idempotency key's hash. A writable open of a log that is not sealed keeps every sealed segment's key filter in memory ([ADR 0048](decisions/0048-sealed-segments-carry-a-key-filter-and-index.md)). |
| **Key index** | A frame that a roll writes after the key filter (from M4). It lists the segment's key hashes, sorted, each with the file position of the frame that holds its record. The writer reads one page of it after the key filter answers "maybe". |
| **False hit** | A key filter "maybe", or a key index entry with the same hash, for a key that the record does not hold. The writer compares the key bytes, so a false hit is never a match. |
| **Locator** | The last frame of a sealed segment (from M4). It holds the footer's file position, so that open can find the footer from the end of the file. |
| **Log reader** | A read-only handle (`LogReader`) on a sealed log. It takes no lock and writes nothing. |
| **Accept queue** | The bounded queue between callers and the writer thread. Its count limit and the admission byte budget can each cause a capacity refusal with `Full`. |
| **Full** | An immediate refusal when a size-eligible request lacks current queue capacity or admission byte budget. It retains no reservation and appends nothing. |
| **Tail map** | The in-memory map from stream id to the stream's latest revision and log offset. A cache. |
| **Sparse index** | A file that maps some log offsets to file positions in one segment. A cache. Not needed in M1. |
| **Cache** | Any state that the log can rebuild by reading segments. A cache is never the only copy of a fact. |
| **Volume** | Every layer that holds one log directory: the guest filesystem, the guest block device, the VMM, the host storage and the durable media. Its requirements are in [volume-requirements.md](architecture/volume-requirements.md). |
| **Failure domain** | The failure events after which a volume still holds every acknowledged record. [ADR 0040](decisions/0040-volumes-meet-testable-minimum-requirements.md) sets the minimum, and each deployment declares the rest. |
| **Qualification record** | A document in `docs/` that identifies one stack and gives the evidence and result for each volume requirement ([ADR 0032](decisions/0032-qualify-deployment-storage-before-m6.md), [ADR 0040](decisions/0040-volumes-meet-testable-minimum-requirements.md)). |

## Diagrams

Each diagram groups related terms from the table. Terms are in the same words as the table.

### What is on disk

```text
segment log = one directory
├─ LOCK
├─ 00000000000000000000.seg      sealed segment   (name = base offset)
├─ 00000000000000000412.seg      sealed segment
└─ 00000000000000000977.seg      active segment   (newest, writer appends here)

one segment:
┌────────┬─────────┬─────────┬─────┬────────┬────────┬────────┬─────────┐
│ header │ frame   │ frame   │ ... │ key    │ key    │ footer │ locator │
│        │ (group) │ (group) │     │ filter │ index  │        │         │
│log id  │ epoch   │ epoch   │     │        │        │streams │→ footer │
└────────┴─────────┴─────────┴─────┴────────┴────────┴────────┴─────────┘
                                   └──── only in a sealed segment ───────┘
the final segment of a sealed log ends: ... [ footer ][ seal ][ locator ]
                                            (no key frames)
frame = length + CRC + body.  File position = a byte spot inside this file.
```

### What is stored

```text
stream "order-7":    rev 0     rev 1            rev 2
stream "order-9":         rev 0         rev 1
                     │    │    │    │    │    │
log, in order:       0    1    2    3    4    5     ← log offset (whole log, from 0)
                     ▲
                     base offset (first record of a segment or group)

record = stream id + stream revision + optional idempotency key + payload
expected revision = what the caller thinks the stream's revision is (not stored)
```

### Write path

```text
caller ─ submit ─┐
caller ─ submit ─┼─► accept queue ─► writer thread
caller ─ submit ─┘   (full → Full)      │ waits up to linger, stops at byte cap
                                        ▼
                               group (many submits, one frame)
                                        │
                                  write + fsync
                                        │
                  ┌─────────────────────┴───────────────────────┐
              success                                          error
                │                                                │
    watermark moves past the group                   writer is poisoned
    submits get an ack                               group → Indeterminate

   offsets:  0 ─────── 40 │ 41 ─── 55
                          ▲ watermark (readers and acks stop here)
                 durable  │  written, not yet fsynced
```

### Crash and recovery

```text
before the crash:  [ok frame][ok frame][ok frame][ half a frame
after open:        [ok frame][ok frame][ok frame]
                                                 └ torn tail (bad length or CRC): cut off

each open:  writer epoch = highest epoch seen + 1
caller after Indeterminate: reopen, resolve keyed records with the same keys
mixed or unkeyed original: follow the API's limited retry guarantees
```

### Caches and sealed-log reading

```text
cache = anything the log can rebuild by reading segments
┌───────────────────────────────────────────────────────────┐
│ tail map: stream id → latest revision, log offset         │
│ sparse index: some log offsets → file positions           │
│ key filters: one per sealed segment, loaded at open       │
└───────────────────────────────────────────────────────────┘

writer looking for a key (from M4):
  current group → active key map → key filter of each sealed segment,
  newest first → on "maybe": one key index page → the record's frame
  → compare the key bytes (a different key is a false hit)

sealed segment, read from the end:
  ... [ footer ][ seal ][ locator ] ◄── start here (final segment)
          ▲                   │
          └───────────────────┘   locator holds the footer's file position
  footer lists every stream: last revision, last offset, last position

log reader = read-only handle on a sealed log (no lock, writes nothing)
```
