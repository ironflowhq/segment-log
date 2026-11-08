# Testing and benchmarks

The crash tests are the gate for M1. Nothing above the library starts until they pass ([ADR 0014](../decisions/0014-crash-testing-strategy.md)).

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
3. **All or nothing (I5).** Each submit is present in full or absent in full.
4. **Retries resolve.** The test retries every submit that did not get an ack, with the same keys and bytes. Afterwards, each keyed record is present exactly once. A survivor returns its original offset and revision with `deduplicated: true`.
5. **Recovery is durable.** A `PowerLoss` right after open, before any append, gives the same log. Run this after every crash mode and after an injected `sync_data` error.
6. **Sealed segments do not change (I7).** Their bytes are the same before and after recovery.
7. **Poison (I10).** After an injected I/O error, every in-flight submit gets `Indeterminate`, and every queued or later submit gets `Poisoned`. A reopen in the same simulated boot, then a `PowerLoss`, then another reopen, satisfies checks 1–4.
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

- Codec round trip for every frame kind ([format.md](format.md)), plus the CRC check value.
- A model test: a plain in-memory model of streams, revisions and keys. Random submits go to the model and to the log, and the results must be equal. This covers R9–R12.
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
- the group size distribution (submits and bytes for each `fsync`);
- open time, seal-to-exit time and idle resident set, where they apply.

A benchmark on macOS is for development only. The targets are for Linux.
