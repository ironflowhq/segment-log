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
| Segment roll size | at most 4,000 MiB | Sparse-index and key-index positions are 4 bytes. A segment can pass the roll size by one group frame and its closing frames. |
| Keyed records in one segment | at most 1,044,225 | The key index frame body must fit the frame body limit: 4,095 pages of 255 entries. The writer rolls before a group whose keys would pass it ([ADR 0048](../decisions/0048-sealed-segments-carry-a-key-filter-and-index.md)). |
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
| 0 | 1 | `kind` — `1` group, `2` footer, `3` seal, `4` locator, `5` key filter, `6` key index |
| 1 | 1 | `flags` — `0` in v0 |
| 2 | 2 | reserved |
| 4 | 8 | `epoch` — the writer epoch that wrote this frame |
| 12 | 8 | `base_offset` — for a group, the log offset of its first record. For any other kind, the next unused log offset. |
| 20 | 4 | `count` — group: number of records (at least 1). Footer: number of streams. Key filter and key index: number of keyed records in the segment. Seal and locator: `0`. |

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

For admission, a submit's standalone group-body size is:

```text
body_len = 24 + sum(16 + stream.len() + key_len + payload.len())
```

Here `key_len` is zero for an unkeyed record. The formula includes one body header and every record entry, and excludes the frame envelope and segment header. A submit fits alone when this size is at most `group_byte_cap`. Use checked arithmetic. This encoded size uses initialized lengths, separately from the capacity-based admission charge in [api.md](api.md#admission-charge).

### Footer frame (M4)

A body header with `kind = 2`, followed by:

| Offset in body | Size | Field |
| --- | --- | --- |
| 24 | 8 | `key_filter_position` — the file position of this segment's key filter frame, or `0` in the final segment of a sealed log, which has no key frames |
| 32 | … | `count` footer entries, one after another |

The key index frame starts where the key filter frame ends, so the footer needs no second position.

### Footer entry (M4)

There is one entry for each stream that has a record in this segment.

| Offset in entry | Size | Field |
| --- | --- | --- |
| 0 | 2 | `stream_len` |
| 2 | 2 | reserved |
| 4 | 8 | `last_revision` — the stream's last revision in this segment |
| 12 | 8 | `last_offset` — the log offset of that record |
| 20 | 8 | `last_position` — the file position of the frame that holds that record |
| 28 | `stream_len` | stream id |

The footer does not hold idempotency keys. The key filter and key index frames before it do ([ADR 0048](../decisions/0048-sealed-segments-carry-a-key-filter-and-index.md)). The writer rolls before a group whose new streams would make the footer body pass the frame body limit ([ADR 0017](../decisions/0017-footer-locator-offset-and-size.md)).

### Key hash (M4)

The key frames name a key by a 64-bit hash: SipHash-2-4 of the key bytes, with nothing added before or after them. The 128-bit SipHash key is the segment header's `log_id`: `k0` is bytes 0..8 of it read as a little-endian integer, and `k1` is bytes 8..16. The hash is the 64-bit result. Do not use Rust's `Hash` trait or `DefaultHasher`: the first adds a length prefix, and the second can change between Rust versions.

Check values, with `log_id` bytes `00 01 02 … 0f`:

- input empty: `0x726fdb47dd0e0e31`
- input `00 01 02 … 0e` (15 bytes): `0xa129ca6149be45e5`

Two keys can have the same hash. The key bytes in the record decide a match, so a shared hash is a false hit, never a match ([ADR 0048](../decisions/0048-sealed-segments-carry-a-key-filter-and-index.md)).

### Key filter frame (M4)

A body header with `kind = 5` and `count = n`, the number of keyed records in the segment. Let `P = ceil(n / 255)`, the number of key index pages, and `B = ceil(n / 32)`, the number of filter blocks. The body continues:

| Offset in body | Size | Field |
| --- | --- | --- |
| 24 | `8 * P` | `fences` — the hash of the first entry of each key index page, in page order. They never decrease. |
| `24 + 8 * P` | `64 * B` | `blocks` — the filter |

`body_len` is exactly `24 + 8 * P + 64 * B`. With `n = 0`, the body is the body header alone, and the filter holds no key.

The filter is a blocked Bloom filter with 16 bits for each key. To add a hash `h` or test for it:

1. The block is `((h >> 32) * B) >> 32`, in 64-bit arithmetic.
2. Let `lo` be the low 32 bits of `h`. For each salt `s` in the table below, the bit is `p = (lo * s mod 2^32) >> 23`, a number from 0 to 511.
3. Bit `p` of a block is bit `p mod 8` of byte `p / 8`, counting bits from the least significant.

Adding a hash sets its 11 bits. A test answers "maybe" when all 11 bits are set. Otherwise the key is not in this segment. When `B = 0`, a test answers that the key is not in this segment without reading a block. The salts, in order:

```text
0x47b6137b 0x44974d91 0x8824ad5b 0xa2b7289d 0x705495c7 0x2df1424b
0x9efc4947 0x5c6bfb31 0x1d0f3e9b 0x6ac1e2f5 0xb3a8c64d
```

Eleven probes is `round(16 * ln 2)`. The [prototype](../evidence/m4/key-lookup/README.md) measured this filter. Check value: for `h = 0xa129ca6149be45e5` and `B = 3`, the block is 1 and the bits are 27, 32, 93, 97, 144, 171, 364, 386, 393, 426 and 434.

### Key index frame (M4)

A body header with `kind = 6` and the same `count = n` as the key filter frame, followed by `P` pages of 4,096 bytes. `body_len` is exactly `24 + 4096 * P`.

Each page:

| Offset in page | Size | Field |
| --- | --- | --- |
| 0 | 4,080 | 255 entry slots |
| 4,080 | 4 | `entry_count` — 255 in every page except the last, which holds the rest (1 to 255) |
| 4,084 | 8 | reserved |
| 4,092 | 4 | `crc` — CRC-32C of page bytes 0..4092 |

Each entry:

| Offset in entry | Size | Field |
| --- | --- | --- |
| 0 | 8 | `hash` — the key hash |
| 8 | 4 | `position` — the file position of the group frame that holds the record |
| 12 | 4 | `record` — the record's index in that frame, from 0 |

The entries of all pages together are sorted by `(hash, position, record)`, with no repeats. Slots after `entry_count` are zero. The frame CRC covers the whole body, but a lookup reads one page and checks that page's CRC, so that it never reads a whole frame of up to 16 MiB.

The entries for a hash `h` start in the last page whose fence is less than `h`, or in page 0 if no fence is, and continue into the following pages whose fence equals `h`.

### Seal frame

A body header with `kind = 3` and `count = 0`, and nothing after it. It uses no log offset. It marks a sealed log.

### Locator frame (M4)

A body header with `kind = 4` and `count = 0`, followed by:

| Offset in body | Size | Field |
| --- | --- | --- |
| 24 | 8 | `footer_position` — the file position of this segment's footer frame |

The whole frame is 40 bytes. It uses no log offset. Open reads the last 40 bytes of a sealed segment that has a successor to find its footer. For the newest segment, only a forward scan can show that a locator is real (see the rules for readers).

## Sealed segment layout (M4)

A segment that a roll sealed holds, in order: the header, its group frames, the key filter, the key index, the footer and the locator. A roll writes both key frames even when the segment has no keyed record. The final segment of a sealed log holds the header, its group frames, the footer, the seal frame and the locator. It has no key frames, because a sealed log never checks a key again ([ADR 0048](../decisions/0048-sealed-segments-carry-a-key-filter-and-index.md)). Before M4, a sealed segment holds only the header and group frames. Seal ships in M4, so a seal frame always has a locator after it.

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
- From M4, key filter, key index, footer, seal and locator frames are closing frames. They are valid only as a closing run, which has one of two shapes:
  - A roll's run: a key filter, a key index with the same `count`, a footer whose `key_filter_position` is that key filter's position, and a locator.
  - A seal's run: a footer whose `key_filter_position` is `0`, a seal frame and a locator.

  In both, the locator points to the footer and ends at the end of the file. A forward scan that ends through a complete run shows a sealed segment. If a run is not complete and valid, the scan stops where the run starts. A closing frame anywhere else ends the log. The last bytes of a file never decide this alone, because a payload can end with bytes that look like a locator.
- In the active segment, the first frame that is not valid ends the log. Recovery truncates there.
- In a sealed segment, a frame that is not valid is corruption. Recovery stops with an error.
- A key index page that a lookup reads is valid only if its CRC matches, its `entry_count` is right for its place in the frame, its reserved bytes and unused slots are zero, its entries ascend, and its first entry's hash equals its fence. An entry that a lookup follows is valid only if its `position` names a valid group frame before the key filter frame and its `record` is less than that frame's `count`. A page or an entry that is not valid is a lookup failure.
- Compression: none. A compressed group cannot be sliced to one record. Add it only when a measurement shows that groups are large enough to gain from it.
