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
| **Submit** | One `append` call from a caller. It holds one or more records, and it is the atomic unit. |
| **Group** | The records of one or more submits that the writer writes as one frame and makes durable with one `fsync`. A group is not a transaction. |
| **Frame** | The on-disk form of a group, a footer, a seal, or a locator: length, CRC (cyclic redundancy check), body. See [format.md](architecture/format.md). |
| **Linger** | How long the writer waits for more submits after the first one, before it writes the group. |
| **Byte cap** | The largest group, in bytes. The writer writes the group when the next submit would go over it. |
| **Log offset** | The index of a record across the whole log, starting at 0. Assigned by the writer. Monotonic. The tail cursor. |
| **Base offset** | The log offset of the first record in a segment or a group. |
| **File position** | The byte position of a frame or record inside one segment. Not an identity that callers store. |
| **Stream** | A named sequence of records inside the log, for example one entity or one run. |
| **Stream id** | The bytes that name a stream. |
| **Stream revision** | The index of a record inside its stream, starting at 0. |
| **Expected revision** | The precondition in a request: the caller's view of the stream's current revision. It is not stored. |
| **Idempotency key** | The caller's dedupe token for a record. Whether a key is scoped to one stream or to the whole log is PRD open item 9. |
| **Watermark** | The log offset just past the last durable record. The writer moves it only after an `fsync` returns success. Readers and acknowledgments never go past it. |
| **Acknowledgment (ack)** | The success result of a submit. It means the group is durable, if the storage meets its [contract](architecture/overview.md#storage-contract). |
| **Indeterminate** | The result when the writer cannot tell whether a submit is durable, for example after an `fsync` error. The caller must reopen and retry with the same keys. |
| **Poisoned** | The writer's state after an I/O error. It refuses every submit until the log is reopened. |
| **Torn tail** | Bytes at the end of the active segment that do not form a complete, CRC-valid frame. Recovery removes them. |
| **Writer epoch** | A number that each opening writer gets: one more than the highest epoch in the newest segment. Every frame stores it, inside the span that the CRC covers. |
| **Log id** | A random id that the log gets at creation. Every segment header stores it. |
| **Footer** | A frame near the end of a sealed segment. It lists each stream in that segment: stream id, last revision, last log offset, last file position. |
| **Locator** | The last frame of a sealed segment (from M4). It holds the footer's file position, so that open can find the footer from the end of the file. |
| **Log reader** | A read-only handle (`LogReader`) on a sealed log. It takes no lock and writes nothing. |
| **Accept queue** | The bounded queue between callers and the writer thread. When it is full, `append` returns `Full`. |
| **Tail map** | The in-memory map from stream id to the stream's latest revision and log offset. A cache. |
| **Sparse index** | A file that maps some log offsets to file positions in one segment. A cache. Not needed in M1. |
| **Cache** | Any state that the log can rebuild by reading segments. A cache is never the only copy of a fact. |

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
┌────────┬─────────┬─────────┬─────────┬─────────┬────────┬─────────┬─────────┐
│ header │ frame   │ frame   │ frame   │ ...     │ footer │ seal    │ locator │
│        │ (group) │ (group) │ (group) │         │        │ (log    │         │
│log id  │ epoch   │ epoch   │ epoch   │         │streams │ only)   │→ footer │
└────────┴─────────┴─────────┴─────────┴─────────┴────────┴─────────┴─────────┘
                                                 └──── only in a sealed segment ┘
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
caller after Indeterminate: reopen, retry with the same idempotency key
```

### Caches and sealed-log reading

```text
cache = anything the log can rebuild by reading segments
┌───────────────────────────────────────────────────────────┐
│ tail map: stream id → latest revision, log offset         │
│ sparse index: some log offsets → file positions           │
└───────────────────────────────────────────────────────────┘

sealed segment, read from the end:
  ... [ footer ][ seal ][ locator ] ◄── start here
          ▲                   │
          └───────────────────┘   locator holds the footer's file position
  footer lists every stream: last revision, last offset, last position

log reader = read-only handle on a sealed log (no lock, writes nothing)
```
