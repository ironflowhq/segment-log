# Byte layout v0

Status: provisional ([ADR 0013](../decisions/0013-provisional-v0-byte-layout.md)). The format can change in any way until M5 freezes it. A change before M5 increments `format_version`. It needs no migration and no reader for older versions.

This file is the only source for field widths, offsets and size limits. Other docs link here and do not copy them.

## Conventions

- All integers are little-endian and unsigned.
- There is no alignment and no padding, except reserved fields.
- Reserved fields are written as zero. In v0, a reader rejects a nonzero reserved field.
- CRC is CRC-32C (Castagnoli): reflected polynomial `0x82F63B78`, initial value `0xFFFFFFFF`, final XOR `0xFFFFFFFF`. Check value: `crc32c(b"123456789") == 0xE3069283`.

## Directory

| Name | Content |
| --- | --- |
| `LOCK` | Empty. The writer holds an exclusive OS lock on it ([ADR 0018](../decisions/0018-writer-epoch-from-the-segments.md)). |
| `NNNNNNNNNNNNNNNNNNNN.seg` | A segment. The name is its base offset as 20 decimal digits, with leading zeros. |
| `NNNNNNNNNNNNNNNNNNNN.idx` | The sparse index for that segment (not in M1). A cache. |

Open ignores any other file.

## Limits

| Limit | Value | Why |
| --- | --- | --- |
| Frame body (`body_len`) | at most 16 MiB (`16 * 1024 * 1024`) | Bounds the memory that one frame needs. |
| Group body | at most 8 MiB | A group's footer entries are less than twice its size, so they always fit in one footer ([ADR 0017](../decisions/0017-footer-locator-offset-and-size.md)). |
| Segment roll size | at most 4,000 MiB | Sparse-index positions are 4 bytes. A segment can pass the roll size by one group frame and its closing frames. |
| Stream id | 1 to 1,024 bytes | |
| Idempotency key | 1 to 256 bytes | |

## Segment header (64 bytes)

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | `magic` — the bytes `S G L G` (`0x53 0x47 0x4C 0x47`) |
| 4 | 2 | `format_version` — `0` |
| 6 | 2 | `header_len` — `64` |
| 8 | 16 | `log_id` — 128 random bits, chosen when the log is created. The same in every segment of a log. |
| 24 | 8 | `base_offset` — the log offset of the first record in this segment. Equal to the file name. |
| 32 | 8 | `epoch` — the writer epoch that created this segment |
| 40 | 20 | reserved |
| 60 | 4 | `crc` — CRC-32C of bytes 0..60 |

The width of `log_id` is provisional. M5 reviews it ([ADR 0013](../decisions/0013-provisional-v0-byte-layout.md)).

## Frame

Frames follow the header with no gap. Every frame has the same envelope:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | `body_len` — the size of the body in bytes. The whole frame is `8 + body_len` bytes. |
| 4 | 4 | `crc` — CRC-32C of the 4 `body_len` bytes followed by the body |
| 8 | `body_len` | body |

`body_len` must be at least 24 and at most the frame body limit. A value outside that range ends the valid prefix. A zero-filled tail therefore ends the scan.

### Body header (24 bytes)

| Offset in body | Size | Field |
| --- | --- | --- |
| 0 | 1 | `kind` — `1` group, `2` footer, `3` seal, `4` locator |
| 1 | 1 | `flags` — `0` in v0 |
| 2 | 2 | reserved |
| 4 | 8 | `epoch` — the writer epoch that wrote this frame |
| 12 | 8 | `base_offset` — for a group, the log offset of its first record. For any other kind, the next unused log offset. |
| 20 | 4 | `count` — group: number of records (at least 1). Footer: number of streams. Seal and locator: `0`. |

The CRC covers the epoch. Kafka keeps its leader epoch outside the CRC, so that a broker can stamp it when it receives a batch. A local writer has no receive path, so it has no reason to leave a field unprotected.

### Group record entry

A group body has `count` entries after the body header, one after another.

| Offset in entry | Size | Field |
| --- | --- | --- |
| 0 | 2 | `stream_len` — within the stream id limit |
| 2 | 2 | `key_len` — 0 means no key; otherwise within the idempotency key limit |
| 4 | 4 | `payload_len` |
| 8 | 8 | `revision` — the stream revision assigned to this record |
| 16 | `stream_len` | stream id |
| … | `key_len` | idempotency key |
| … | `payload_len` | payload |

The log offset of entry `i` (from 0) is `base_offset + i`. It is not stored. A record's file position is the frame's file position plus the entry's offset inside the frame. The entries must fill the body exactly. Extra or missing bytes make the frame invalid.

### Footer entry (M4)

A footer body has `count` entries, one for each stream that has a record in this segment.

| Offset in entry | Size | Field |
| --- | --- | --- |
| 0 | 2 | `stream_len` |
| 2 | 2 | reserved |
| 4 | 8 | `last_revision` — the stream's last revision in this segment |
| 12 | 8 | `last_offset` — the log offset of that record |
| 20 | 8 | `last_position` — the file position of the frame that holds that record |
| 28 | `stream_len` | stream id |

The footer does not hold idempotency keys (PRD open item 1). The writer rolls before a group whose new streams would make the footer body pass the frame body limit ([ADR 0017](../decisions/0017-footer-locator-offset-and-size.md)).

### Seal frame

A body header with `kind = 3` and `count = 0`, and nothing after it. It uses no log offset. It marks a sealed log.

### Locator frame (M4)

A body header with `kind = 4` and `count = 0`, followed by:

| Offset in body | Size | Field |
| --- | --- | --- |
| 24 | 8 | `footer_position` — the file position of this segment's footer frame |

The whole frame is 40 bytes. It uses no log offset. Open reads the last 40 bytes of a sealed segment that has a successor to find its footer. For the newest segment, only a forward scan can show that a locator is real (see the rules for readers).

## Sealed segment layout (M4)

A sealed segment holds, in order: the header, its group frames, the footer, the seal frame if it is the final segment of a sealed log, and the locator. Before M4, a sealed segment holds only the header and group frames. Seal ships in M4, so a seal frame always has a locator after it.

## Sparse index (after M1)

A file of fixed 8-byte entries with no header and no CRC. The log rebuilds it from the segment if it is missing, short or wrong.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | `relative_offset` — log offset minus the segment's base offset |
| 4 | 4 | `position` — file position of the frame that holds that offset |

The writer adds one entry about every 4 KiB of frames. The 4-byte fields limit a segment to 2^32 bytes and 2^32 records. The segment roll size limit keeps every segment below that. A guest's index fits in a few pages, so a binary search over the whole file is enough.

## Rules for readers

- A header is valid only if its magic, version, length and CRC match, its reserved bytes are zero, its `log_id` equals the first segment's `log_id`, and its `base_offset` equals its file name.
- Each segment's `base_offset` equals the next unused log offset of the segment before it. A gap is an error, not a hole to skip.
- A frame is valid only if its length is in range, its CRC matches, its reserved fields are zero, its `kind` is known, its `base_offset` is the next expected offset, its `epoch` is not lower than the epoch of the frame before it, and its entries fill the body exactly.
- In the active segment, the first frame that is not valid ends the log. Recovery truncates there. From M4, footer, seal and locator frames are valid only as the closing run: a footer, then a seal frame if there is one, then a locator that points to that footer and ends at the end of the file. A forward scan that ends through that run shows a sealed segment. A closing frame anywhere else ends the log. The last bytes of a file never decide this alone, because a payload can end with bytes that look like a locator.
- In a sealed segment, a frame that is not valid is corruption. Recovery stops with an error.
- Compression: none. A compressed group cannot be sliced to one record. Add it only when a measurement shows that groups are large enough to gain from it.
