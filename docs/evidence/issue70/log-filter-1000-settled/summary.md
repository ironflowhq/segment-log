# M4 evidence summary

Conditions are in environment.txt. A hosted runner is a shared virtual machine: these are measurements under those conditions, not a target pass.

## Fixtures

| Fixture | Records | Sealed segments | Records per sealed segment | Record entry bytes | Active bytes | Active records | Directory bytes | Expected filter bytes | Build s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| after-keyed | 23015000 | 1000 | 23000 | 296 | 4440544 | 15000 | 7268864544 | 46744000 | 73.5 |
| after-unkeyed | 24015000 | 1000 | 24000 | 280 | 4200544 | 15000 | 6761176544 | 0 | 46.5 |
| before-keyed | 23015000 | 1000 | 23000 | 296 | 4440544 | 15000 | 6813240544 | none in M3 | 92.7 |

## Open time

Each run opens the after-side fixture once with `SegmentLog::open`, end to end. Target: 50 ms (docs/prd.md). Cold runs drop the page cache before each open; `page_cache_before_kib` shows the drop.

| Fixture | Cache | Runs | Min ms | p50 ms | Max ms | Page cache before, KiB |
| --- | --- | --- | --- | --- | --- | --- |
| keyed | cold | 10 | 567.584 | 582.001 | 606.965 | 282868 to 283448 |
| keyed | warm | 10 | 124.067 | 124.596 | 126.755 | 393564 to 393644 |
| unkeyed | cold | 10 | 470.506 | 482.826 | 499.337 | 282900 to 283380 |
| unkeyed | warm | 10 | 103.654 | 104.211 | 105.172 | 345432 to 345580 |

Keyed minus unkeyed p50, cold: 99.175 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

Keyed minus unkeyed p50, warm: 20.385 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

## Key-cache memory

- ADR 0048 rule 5, the filter part: 46744000 bytes for 23000000 sealed keys (2.032 bytes per key), from the key filter frame layout in format.md.
- Heap the open log holds (allocations live after open, first run): keyed 48612612 bytes, unkeyed 735594 bytes. The difference, 47877018 bytes, is the measured key-cache memory: the filters, the index-page fences and the active segment's exact map of 15000 keys.
- Measured minus the filter formula: 1133018 bytes, 75.5 bytes per active-segment key.
- Peak heap during open: keyed 48662294 bytes, unkeyed 951864 bytes.
- Resident set before and after the first open, KiB: keyed 2636 to 51564, unkeyed 2632 to 4532.

## Appends, M3 (before) and M4 (after)

### closed: keyed appends, settle 15000 ms, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 6710886, start 23015000 records in 1001 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 229949 | 4.184 | 6.895 | 21.264 | 23.959 | 0 | 1001 to 1045 | 30651.1 | 2596000 |
| before | 2 | 246949 | 3.906 | 6.366 | 19.727 | 21.520 | 0 | 1001 to 1045 | 30348.9 | 2595896 |
| before | 3 | 247467 | 3.896 | 6.347 | 19.873 | 22.058 | 0 | 1001 to 1045 | 30435.8 | 2596008 |
| before | 4 | 243277 | 3.966 | 6.313 | 20.154 | 22.429 | 0 | 1001 to 1045 | 30590.1 | 2596032 |
| before | 5 | 250663 | 3.848 | 6.152 | 20.300 | 22.438 | 0 | 1001 to 1045 | 30632.5 | 2595760 |
| after | 1 | 225976 | 4.168 | 10.769 | 12.708 | 14.808 | 0 | 1001 to 1045 | 131.6 | 142100 |
| after | 2 | 227348 | 4.110 | 10.951 | 13.871 | 20.174 | 0 | 1001 to 1045 | 132.7 | 142448 |
| after | 3 | 231266 | 4.056 | 10.711 | 12.665 | 15.238 | 0 | 1001 to 1045 | 130.7 | 141880 |
| after | 4 | 230447 | 4.070 | 10.651 | 12.676 | 15.714 | 0 | 1001 to 1045 | 131.1 | 142340 |
| after | 5 | 234194 | 4.010 | 10.416 | 12.595 | 16.155 | 0 | 1001 to 1045 | 129.7 | 142572 |

Records/s: before median 246948.616, range 229948.546 to 250662.999; after median 230446.642, range 225976.279 to 234194.108; after/before medians 0.933; ranges overlap: yes.

p50 ms: before median 3.906, range 3.848 to 4.184; after median 4.070, range 4.010 to 4.168; after/before medians 1.042; ranges overlap: yes.

p99 ms: before median 6.347, range 6.152 to 6.895; after median 10.711, range 10.416 to 10.951; after/before medians 1.688; ranges overlap: no.

### closed-unkeyed: unkeyed appends, settle 15000 ms, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 6710886, start 23015000 records in 1001 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 372191 | 2.553 | 4.690 | 6.193 | 10.010 | 0 | 1001 to 1042 | 30574.3 | 2489136 |
| before | 2 | 362641 | 2.593 | 5.125 | 17.473 | 22.133 | 0 | 1001 to 1043 | 30523.8 | 2489260 |
| before | 3 | 367703 | 2.562 | 4.886 | 17.385 | 22.394 | 0 | 1001 to 1042 | 30336.8 | 2489476 |
| before | 4 | 364214 | 2.584 | 4.858 | 11.555 | 18.110 | 0 | 1001 to 1042 | 30521.7 | 2489244 |
| before | 5 | 365564 | 2.581 | 4.779 | 11.349 | 21.792 | 0 | 1001 to 1042 | 30661.0 | 2489212 |
| after | 1 | 316075 | 3.059 | 5.146 | 7.245 | 11.792 | 0 | 1001 to 1042 | 128.4 | 136668 |
| after | 2 | 320865 | 2.998 | 5.222 | 7.350 | 10.794 | 0 | 1001 to 1042 | 130.1 | 136616 |
| after | 3 | 326707 | 2.943 | 5.091 | 6.698 | 9.580 | 0 | 1001 to 1042 | 129.8 | 136496 |
| after | 4 | 316680 | 3.037 | 5.298 | 6.883 | 10.538 | 0 | 1001 to 1042 | 130.3 | 136540 |
| after | 5 | 315061 | 3.073 | 5.155 | 6.647 | 9.632 | 0 | 1001 to 1042 | 130.2 | 136376 |

Records/s: before median 365564.266, range 362640.777 to 372191.493; after median 316680.156, range 315061.468 to 326707.242; after/before medians 0.866; ranges overlap: no.

p50 ms: before median 2.581, range 2.553 to 2.593; after median 3.037, range 2.943 to 3.073; after/before medians 1.176; ranges overlap: no.

p99 ms: before median 4.858, range 4.690 to 5.125; after median 5.155, range 5.091 to 5.298; after/before medians 1.061; ranges overlap: yes.

### paced: keyed appends, settle 15000 ms, 1000 callers, 1000 rounds, interval 20000 µs, offered 50000 records/s, roll_bytes 6710886, start 23015000 records in 1001 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 49964 | 0.998 | 17.539 | 25.943 | 32.188 | 0 | 1001 to 1045 | 30326.3 | 2601700 |
| before | 2 | 49961 | 0.997 | 18.589 | 26.800 | 34.604 | 0 | 1001 to 1045 | 30240.7 | 2601656 |
| before | 3 | 49957 | 1.002 | 18.866 | 28.135 | 36.990 | 0 | 1001 to 1045 | 30529.8 | 2601628 |
| before | 4 | 49898 | 1.015 | 19.104 | 29.041 | 36.530 | 0 | 1001 to 1045 | 30226.7 | 2601648 |
| before | 5 | 49965 | 0.998 | 19.078 | 26.510 | 37.487 | 0 | 1001 to 1045 | 30523.1 | 2601504 |
| after | 1 | 49965 | 1.012 | 5.037 | 8.780 | 12.694 | 0 | 1001 to 1045 | 130.3 | 147028 |
| after | 2 | 49965 | 0.996 | 4.778 | 8.435 | 11.345 | 0 | 1001 to 1045 | 129.3 | 146608 |
| after | 3 | 49963 | 1.018 | 5.235 | 8.854 | 17.516 | 0 | 1001 to 1045 | 129.4 | 146848 |
| after | 4 | 49964 | 1.018 | 5.157 | 8.709 | 11.436 | 0 | 1001 to 1045 | 129.5 | 146640 |
| after | 5 | 49966 | 1.006 | 4.619 | 8.272 | 10.189 | 0 | 1001 to 1045 | 129.4 | 146668 |

Records/s: before median 49960.790, range 49898.349 to 49964.809; after median 49964.645, range 49962.583 to 49965.761; after/before medians 1.000; ranges overlap: yes.

p50 ms: before median 0.998, range 0.997 to 1.015; after median 1.012, range 0.996 to 1.018; after/before medians 1.014; ranges overlap: yes.

p99 ms: before median 18.866, range 17.539 to 19.104; after median 5.037, range 4.619 to 5.235; after/before medians 0.267; ranges overlap: no.

### Key lookup cost on each side

Both modes append to the same keyed fixture. Unkeyed appends skip the key lookup, so the gap is what the lookup costs on that side (ADR 0051).

- before: keyed/unkeyed medians 0.676; keyed minus unkeyed time per record 1.31 µs.
- after: keyed/unkeyed medians 0.728; keyed minus unkeyed time per record 1.18 µs.

