# M4 evidence summary

Conditions are in environment.txt. A hosted runner is a shared virtual machine: these are measurements under those conditions, not a target pass.

## Fixtures

| Fixture | Records | Sealed segments | Records per sealed segment | Record entry bytes | Active bytes | Active records | Directory bytes | Expected filter bytes | Build s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| after-keyed | 22715000 | 100 | 227000 | 296 | 4440544 | 15000 | 7139055744 | 46114400 | 102.4 |
| after-unkeyed | 24015000 | 100 | 240000 | 280 | 4200544 | 15000 | 6728589344 | 0 | 43.2 |
| before-keyed | 22715000 | 100 | 227000 | 296 | 4440544 | 15000 | 6724373344 | none in M3 | 90.4 |

## Open time

Each run opens the after-side fixture once with `SegmentLog::open`, end to end. Target: 50 ms (docs/prd.md). Cold runs drop the page cache before each open; `page_cache_before_kib` shows the drop.

| Fixture | Cache | Runs | Min ms | p50 ms | Max ms | Page cache before, KiB |
| --- | --- | --- | --- | --- | --- | --- |
| keyed | cold | 10 | 201.971 | 218.703 | 249.044 | 284788 to 285028 |
| keyed | warm | 10 | 29.287 | 29.617 | 50.946 | 385008 to 385160 |
| unkeyed | cold | 10 | 62.773 | 72.253 | 127.985 | 284668 to 284896 |
| unkeyed | warm | 10 | 12.030 | 12.137 | 21.212 | 310844 to 310972 |

Keyed minus unkeyed p50, cold: 146.450 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

Keyed minus unkeyed p50, warm: 17.479 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

## Key-cache memory

- ADR 0048 rule 5, the filter part: 46114400 bytes for 22700000 sealed keys (2.031 bytes per key), from the key filter frame layout in format.md.
- Heap the open log holds (allocations live after open, first run): keyed 47726530 bytes, unkeyed 477574 bytes. The difference, 47248956 bytes, is the measured key-cache memory: the filters, the index-page fences and the active segment's exact map of 15000 keys.
- Measured minus the filter formula: 1134556 bytes, 75.6 bytes per active-segment key.
- Peak heap during open: keyed 48660421 bytes, unkeyed 801080 bytes.
- Resident set before and after the first open, KiB: keyed 2536 to 53284, unkeyed 2460 to 3884.

## Appends, M3 (before) and M4 (after)

### closed: keyed appends, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 67108864, start 22715000 records in 101 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 167130 | 3.598 | 89.944 | 104.698 | 114.265 | 0 | 101 to 105 | 30923.6 | 2569976 |
| before | 2 | 154727 | 3.624 | 84.600 | 176.564 | 200.873 | 0 | 101 to 105 | 31050.2 | 2570052 |
| before | 3 | 220776 | 3.459 | 36.269 | 109.216 | 116.806 | 0 | 101 to 105 | 31400.5 | 2569828 |
| before | 4 | 266214 | 3.459 | 7.740 | 36.971 | 43.503 | 0 | 101 to 105 | 31410.5 | 2570096 |
| before | 5 | 263653 | 3.528 | 8.179 | 34.881 | 39.306 | 0 | 101 to 105 | 31137.3 | 2569972 |
| after | 1 | 73369 | 10.183 | 100.723 | 118.944 | 134.965 | 0 | 101 to 105 | 42.4 | 128924 |
| after | 2 | 93454 | 7.016 | 141.703 | 320.955 | 347.309 | 0 | 101 to 105 | 79.8 | 127728 |
| after | 3 | 133644 | 7.027 | 16.123 | 76.584 | 83.955 | 0 | 101 to 105 | 42.9 | 127840 |
| after | 4 | 133796 | 6.896 | 20.635 | 79.388 | 92.067 | 0 | 101 to 105 | 38.3 | 127752 |
| after | 5 | 135509 | 6.955 | 13.976 | 81.733 | 90.295 | 0 | 101 to 105 | 38.0 | 127664 |

Records/s: before median 220775.627, range 154727.114 to 266214.219; after median 133644.272, range 73369.498 to 135508.720; after/before medians 0.605; ranges overlap: no.

p50 ms: before median 3.528, range 3.459 to 3.624; after median 7.016, range 6.896 to 10.183; after/before medians 1.989; ranges overlap: no.

p99 ms: before median 36.269, range 7.740 to 89.944; after median 20.635, range 13.976 to 141.703; after/before medians 0.569; ranges overlap: yes.

### closed-unkeyed: unkeyed appends, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 67108864, start 22715000 records in 101 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 382638 | 2.394 | 5.851 | 22.839 | 36.018 | 0 | 101 to 105 | 31197.5 | 2463740 |
| before | 2 | 363049 | 2.500 | 5.563 | 37.038 | 44.965 | 0 | 101 to 105 | 31243.9 | 2463612 |
| before | 3 | 331296 | 2.416 | 7.289 | 105.691 | 111.090 | 0 | 101 to 105 | 31774.9 | 2463592 |
| before | 4 | 365926 | 2.494 | 5.655 | 19.342 | 39.306 | 0 | 101 to 105 | 31205.7 | 2463616 |
| before | 5 | 356235 | 2.499 | 7.694 | 35.566 | 41.066 | 0 | 101 to 105 | 31497.0 | 2463548 |
| after | 1 | 365025 | 2.552 | 5.645 | 12.388 | 24.373 | 0 | 101 to 105 | 41.9 | 90972 |
| after | 2 | 352472 | 2.668 | 5.408 | 12.360 | 24.381 | 0 | 101 to 105 | 36.4 | 90708 |
| after | 3 | 316809 | 2.663 | 10.161 | 90.127 | 104.520 | 0 | 101 to 105 | 39.1 | 90960 |
| after | 4 | 343785 | 2.647 | 7.103 | 21.944 | 44.458 | 0 | 101 to 105 | 41.1 | 90520 |
| after | 5 | 353402 | 2.619 | 5.643 | 18.088 | 22.123 | 0 | 101 to 105 | 44.1 | 90972 |

Records/s: before median 363048.772, range 331296.291 to 382638.306; after median 352471.886, range 316808.607 to 365025.399; after/before medians 0.971; ranges overlap: yes.

p50 ms: before median 2.494, range 2.394 to 2.500; after median 2.647, range 2.552 to 2.668; after/before medians 1.061; ranges overlap: no.

p99 ms: before median 5.851, range 5.563 to 7.694; after median 5.645, range 5.408 to 10.161; after/before medians 0.965; ranges overlap: yes.

### paced: keyed appends, 1000 callers, 1000 rounds, interval 20000 µs, offered 50000 records/s, roll_bytes 67108864, start 22715000 records in 101 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 49950 | 1.071 | 37.869 | 49.218 | 58.940 | 0 | 101 to 105 | 30969.1 | 2575996 |
| before | 2 | 49963 | 1.091 | 62.248 | 91.040 | 97.896 | 0 | 101 to 105 | 30611.7 | 2576288 |
| before | 3 | 49957 | 1.057 | 37.634 | 47.133 | 55.431 | 0 | 101 to 105 | 30626.4 | 2576252 |
| before | 4 | 49960 | 1.043 | 35.894 | 44.570 | 50.438 | 0 | 101 to 105 | 31373.3 | 2576252 |
| before | 5 | 49960 | 1.063 | 35.446 | 45.237 | 53.233 | 0 | 101 to 105 | 31621.5 | 2576264 |
| after | 1 | 49957 | 1.127 | 93.481 | 159.188 | 177.738 | 0 | 101 to 105 | 35.6 | 128960 |
| after | 2 | 49958 | 1.085 | 47.941 | 76.379 | 85.557 | 0 | 101 to 105 | 37.7 | 129216 |
| after | 3 | 49958 | 1.062 | 35.719 | 68.698 | 77.882 | 0 | 101 to 105 | 40.8 | 129364 |
| after | 4 | 49958 | 1.113 | 94.961 | 144.899 | 172.980 | 0 | 101 to 105 | 38.5 | 129220 |
| after | 5 | 49959 | 1.127 | 57.760 | 80.930 | 94.163 | 0 | 101 to 105 | 42.5 | 129204 |

Records/s: before median 49959.524, range 49950.277 to 49962.677; after median 49957.711, range 49956.789 to 49958.776; after/before medians 1.000; ranges overlap: yes.

p50 ms: before median 1.063, range 1.043 to 1.091; after median 1.113, range 1.062 to 1.127; after/before medians 1.047; ranges overlap: yes.

p99 ms: before median 37.634, range 35.446 to 62.248; after median 57.760, range 35.719 to 94.961; after/before medians 1.535; ranges overlap: yes.

### Key lookup cost on each side

Both modes append to the same keyed fixture. Unkeyed appends skip the key lookup, so the gap is what the lookup costs on that side (ADR 0051).

- before: keyed/unkeyed medians 0.608; keyed minus unkeyed time per record 1.78 µs.
- after: keyed/unkeyed medians 0.379; keyed minus unkeyed time per record 4.65 µs.

