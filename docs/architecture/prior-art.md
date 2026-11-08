# Prior art

This page records what the design took from Kafka, what it left, and why no existing crate is a dependency. The survey date is 5 November 2026.

## Kafka

Sources: the Kafka 4.3 message-format page and the 4.0.0 index sources. Kafka 4.4.0 (22 October 2026) was the current release, and it is KRaft-only. Nobody fetched the 4.4 message-format page. The 4.4 announcement does not mention a change to the record format. The 4.3 log page still describes an older framing in places. Where it disagrees with the 4.3 message-format page, this design trusted the message-format page. Magic is still 2 there. `flush.messages` comes from the 4.1 topic-config page. `linger.ms` and `max.block.ms` come from the 4.4 producer-config page.

Kafka gets its durability from replication. `acks=all` waits for the in-sync replica set. `acks=1` returns when the leader has the bytes in its page cache. `flush.messages` defaults to never, so an acknowledgment is not an `fsync`. `linger.ms` (default 5 ms since 4.0) only delays the batch. On one machine with no follower, `acks=all` and `acks=1` name the same process, and neither one means that the bytes survived an `fsync`. So this log's watermark is not Kafka's recovery point.

On the pages that were read, Kafka truncates a torn or checksum-invalid tail. It does not ignore a checksum-valid suffix. Nobody checked whether a segment roll flushes.

Kafka's idempotence is a different mechanism. A batch carries a base sequence for a producer id and an epoch. The sequence advances for each record in that producer and partition. A new producer session without a transaction gets a new producer id and starts at sequence 0. A `transactional.id` can carry the identity across sessions. Neither one is an application dedupe token, and neither one stops a second process from writing the same logical event. This log uses the caller's key, so that a retry, which usually carries a stale expected revision, still resolves.

| Keep from Kafka | Borrow, and change | Leave |
| --- | --- | --- |
| An active segment, and sealed segments named by base offset | Linger and a byte cap as the wait before one `fsync`. Start near 1 ms and a roll of tens of megabytes, then measure. | High watermark, `acks=all`, `min.insync.replicas` |
| Length-prefixed batches with CRC-32C | A 4-byte relative offset and position as a limit, with a roll much earlier than 1 GiB or 2^31 records | Treating `flush.messages` left at never as a durability policy |
| A monotonic log offset as the tail cursor, never used again once durable | One header over many records, so one checksum covers a group | Transactions, control batches, last stable offset |
| A sparse index without checksums, rebuilt by a scan | A session sequence, only if more than one batch can be unacknowledged | Compaction, tombstones, and holes left so a producer sequence survives |
| A writer epoch checked at recovery, and an OS lock against a second live writer | A CRC over the whole batch, epoch included | Leader-epoch checkpoint, follower truncation |
| Delete whole segments only after a later footer covers their streams | A time index, only if callers seek by timestamp | `sendfile` reads, tiered storage, consumer groups, the Kafka protocol |
| | | 1 GiB segments, 7-day retention, index pre-allocation sized for a broker |

Kafka's relative offset is a signed 32-bit integer, which limits how far a segment's offsets can run from its base. The position field has its own limit. Kafka's default segment is 1 GiB. A guest rolls much earlier. Kafka's warm-window index search exists because a large index caused page faults. A guest index that fits in a few pages does not need it.

The clean-shutdown marker was considered and not kept ([ADR 0011](../decisions/0011-sealed-footer-and-no-shutdown-marker.md)). Nobody confirmed its filename from Kafka's `CleanShutdownFileHandler`, and nobody opened `LogSegment.recover`.

Kafka transaction markers and EventStore's retired prepare/commit transactions are not copied. A submit is the atomic unit ([ADR 0009](../decisions/0009-caller-submit-is-the-atomic-unit.md)).

## Crates surveyed

No crate showed the whole contract: one writer, group commit, a durable commit boundary, many streams in one file, expected revision, idempotency keys, a queue that refuses when full, and seal. No candidate published its idle resident set, startup time or exit time. A rejection means that the surveyed version did not show that contract. It does not mean that an unread source file in that repository lacks a watermark.

| Closest piece | Crate | Why it is not a dependency |
| --- | --- | --- |
| `commit` returns a durable watermark behind the last appended LSN | [`open-wal` 0.2.1](https://crates.io/crates/open-wal) | One LSN per directory. Pre-1.0, about 60 downloads. No streams, no expected revision. Its book's durability model is the physical contract to read. Spec section 5, which the book calls normative, was not opened, so this row claims no byte layout for that watermark. |
| Many logical logs in one append-only file | [`raft-engine` 0.4.2](https://crates.io/crates/raft-engine) | Raft protobuf entries. Reads copy out. Last publish April 2024. Expiry by hole-punching, not sealed segments that stay. TiKV and Greptime embed it. Pull request 400 describes `Engine::sync()` returning without `fdatasync` on an empty batch. Nobody checked whether it merged, so 0.4.2 is neither convicted nor cleared. |
| CRC-32C and a coalesced `fsync` | [`wal-db` 1.1.0](https://crates.io/crates/wal-db) | One byte-offset LSN, many writers, no expected revision. Open keeps a checksum-valid record that reached disk even when `sync` had not returned. That matches this log's recovery rule. It is not a failed durability proof. |
| Group commit, an mmap reader, a seal flag | [`nodedb-wal` 0.6.2](https://docs.rs/crate/nodedb-wal/latest) | One LSN. Encryption crates always linked. Pre-1.0. |

Other systems:

- Apache Iggy keeps its segment files and sparse index inside its server, with Viewstamped Replication on the write path.
- Fluvio does not publish its storage crate as a library.
- Tansu was renamed Nisshi, and it stores data in PostgreSQL, SQLite or S3.
- Materialize `persist` and Restate's log use the Business Source License and sit behind a consensus log.
- `krabka-log` is a real library, byte-compatible with one Kafka partition directory. It has no expected revision.
- Chronicle Queue's Rust build needs a commercial license key.

`open-wal` is the shape to read for the physical half: append stays in memory, `commit` is `write` plus `fdatasync`, the handle is `Send` and not `Sync`, and the book says there are no background threads. Its recovery, as read, is torn-tail truncation, which matches [ADR 0003](../decisions/0003-recovery-keeps-the-checksum-valid-prefix.md). Linking `open-wal` or `raft-engine` would still leave the stream half, which is the product, to write.

## Performance reference points

These numbers come from third-party tests and forum posts, found on 7 November 2026. No source was checked against its primary data. The [PRD targets](../prd.md#footprint-and-performance-targets) link here.

| System | Reported | Same contract? |
| --- | --- | --- |
| Kafka, single node, `acks=all` | 326K records/s, 573 ms average latency ([test](https://computingforgeeks.com/kafka-vs-redpanda-benchmarks/)) | No. `flush.messages` defaults to never, so there is no `fsync` per acknowledgment. |
| Redpanda, `acks=all` | 39K records/s in the same test. Redpanda's own benchmarks claim an `fsync` per batch and lower tail latency. | Partly. [Vanlightly](https://jack-vanlightly.com/blog/2023/5/15/kafka-vs-redpanda-performance-do-the-claims-add-up) found the vendor's Kafka setup skewed. |
| NATS JetStream, `sync_interval: always` | "A few hundred msg/s" ([docs PR](https://github.com/nats-io/nats.docs/pull/896/files)). 158 msg/s in a [Dart client test](https://pub.dev/documentation/rk_nats/0.2.2/). | In kind, with no group commit. |
| NATS JetStream, default | About 32K msg/s sync publish, about 370K async ([2.14 RC post](https://qaze.pages.dev/blog/nats-fast-ingest-batch-publish)) | No. The default `fsync` interval is 2 minutes. |
| SQLite, WAL, `synchronous=FULL` | About 9K transactions/s on NVMe, 6 inserts each ([forum](https://sqlite.org/forum/forumpost/c872a3333f)). About 270/s on old Android storage. | Embedded, with no group commit across callers. |
| EventStoreDB | About 65 appends/s for one sequential client. About 25K events/s with 1,000-event batches. Both are derived from an [old forum post](https://discuss.eventstore.com/t/eventstoredb-performance-comparison/5068). | Similar contract. Old, with no version stated. |

No source gave open time, exit time, idle resident set or binary size for a comparable system.

## Evidence limits

Treat these as unread, not as proof of absence:

- The performance table above. Every row is a secondary source, and the hardware differs from row to row.
- The Kafka 4.4 message-format page. `TimeIndex.java` on a 4.x tag. `LogSegment.recover` and `LogLoader`. The clean-shutdown filename in `CleanShutdownFileHandler`. The numeric defaults for `flush.ms` and `segment.index.bytes` on the 4.4 config page. Whether Kafka flushes on a segment roll.
- `open-wal` spec section 5.
- `raft-engine` pull request 400, merge state unknown.
- Kurrent (EventStoreDB) current gRPC names for expected version, and the rest of its event-id rules. The TCP client page that was read is deprecated. No EventStore chunk-file layout was read.
- No resident set, startup or exit measurement for any crate, and none for this design.

Primary pages read for the survey:

- [Kafka 4.3 log](https://kafka.apache.org/43/implementation/log/)
- [Kafka 4.3 message format](https://kafka.apache.org/43/implementation/message-format/)
- [Kafka 4.3 design](https://kafka.apache.org/43/design/design/)
- [KIP-98](https://cwiki.apache.org/confluence/spaces/KAFKA/pages/66854913/KIP-98+-+Exactly+Once+Delivery+and+Transactional+Messaging)
- [Kafka 4.0.0 `OffsetIndex.java`](https://raw.githubusercontent.com/apache/kafka/4.0.0/storage/src/main/java/org/apache/kafka/storage/internals/log/OffsetIndex.java)
- [Kurrent TCP appending page](https://docs.kurrent.io/clients/tcp/dotnet/21.2/appending.md) (deprecated client)
