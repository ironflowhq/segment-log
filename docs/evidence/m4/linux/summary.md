# M4 evidence summary

Conditions are in environment.txt. A hosted runner is a shared virtual machine: these are measurements under those conditions, not a target pass.

## Fixtures

| Fixture | Records | Sealed segments | Records per sealed segment | Record entry bytes | Active bytes | Active records | Directory bytes | Expected filter bytes | Build s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| after-keyed | 22715000 | 100 | 227000 | 296 | 4440544 | 15000 | 7139055744 | 46114400 | 99.0 |
| after-unkeyed | 24015000 | 100 | 240000 | 280 | 4200544 | 15000 | 6728589344 | 0 | 44.3 |
| before-keyed | 22715000 | 100 | 227000 | 296 | 4440544 | 15000 | 6724373344 | none in M3 | 90.9 |

## Open time

Each run opens the after-side fixture once with `SegmentLog::open`, end to end. Target: 50 ms (docs/prd.md). Cold runs drop the page cache before each open; `page_cache_before_kib` shows the drop.

| Fixture | Cache | Runs | Min ms | p50 ms | Max ms | Page cache before, KiB |
| --- | --- | --- | --- | --- | --- | --- |
| keyed | cold | 10 | 117.714 | 122.780 | 132.710 | 284772 to 285328 |
| keyed | warm | 10 | 38.376 | 38.616 | 41.961 | 356188 to 356248 |
| unkeyed | cold | 10 | 76.411 | 81.425 | 88.995 | 285056 to 285236 |
| unkeyed | warm | 10 | 17.803 | 18.108 | 19.778 | 311172 to 311240 |

Keyed minus unkeyed p50, cold: 41.356 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

Keyed minus unkeyed p50, warm: 20.508 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

## Key-cache memory

- ADR 0048 rule 5, the filter part: 46114400 bytes for 22700000 sealed keys (2.031 bytes per key), from the key filter frame layout in format.md.
- Heap the open log holds (allocations live after open, first run): keyed 47728890 bytes, unkeyed 479934 bytes. The difference, 47248956 bytes, is the measured key-cache memory: the filters, the index-page fences and the active segment's exact map of 15000 keys.
- Measured minus the filter formula: 1134556 bytes, 75.6 bytes per active-segment key.
- Peak heap during open: keyed 48201973 bytes, unkeyed 803480 bytes.
- Resident set before and after the first open, KiB: keyed 2420 to 53172, unkeyed 2416 to 3772.

## Keyed appends, M3 (before) and M4 (after)

### closed: 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 67108864, start 22715000 records in 101 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 201091 | 4.679 | 8.139 | 42.392 | 45.943 | 0 | 101 to 105 | 31916.2 | 2569904 |
| before | 2 | 193321 | 4.811 | 9.420 | 44.662 | 48.434 | 0 | 101 to 105 | 31727.6 | 2569928 |
| before | 3 | 204740 | 4.568 | 9.351 | 42.980 | 47.586 | 0 | 101 to 105 | 31583.5 | 2570124 |
| before | 4 | 202757 | 4.573 | 10.816 | 43.801 | 53.204 | 0 | 101 to 105 | 31486.4 | 2570032 |
| before | 5 | 201843 | 4.649 | 8.572 | 43.368 | 47.127 | 0 | 101 to 105 | 32496.4 | 2569712 |
| after | 1 | 68841 | 12.872 | 28.566 | 86.101 | 94.815 | 0 | 101 to 105 | 44.6 | 127096 |
| after | 2 | 109006 | 8.747 | 13.979 | 82.882 | 91.254 | 0 | 101 to 105 | 44.0 | 125808 |
| after | 3 | 112988 | 8.468 | 12.970 | 76.901 | 84.449 | 0 | 101 to 105 | 43.7 | 131208 |
| after | 4 | 109099 | 8.764 | 17.459 | 72.806 | 81.699 | 0 | 101 to 105 | 44.1 | 126112 |
| after | 5 | 111478 | 8.572 | 14.270 | 73.774 | 78.526 | 0 | 101 to 105 | 43.9 | 127780 |

Records/s: before median 201842.618, range 193321.286 to 204740.031; after median 109099.321, range 68841.118 to 112987.785; after/before medians 0.541; ranges overlap: no.

p50 ms: before median 4.649, range 4.568 to 4.811; after median 8.747, range 8.468 to 12.872; after/before medians 1.881; ranges overlap: no.

p99 ms: before median 9.351, range 8.139 to 10.816; after median 14.270, range 12.970 to 28.566; after/before medians 1.526; ranges overlap: no.

### paced: 1000 callers, 1000 rounds, interval 20000 µs, offered 50000 records/s, roll_bytes 67108864, start 22715000 records in 101 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 49933 | 1.264 | 46.294 | 55.095 | 61.869 | 0 | 101 to 105 | 31579.5 | 2576896 |
| before | 2 | 49815 | 1.199 | 45.005 | 56.738 | 64.195 | 0 | 101 to 105 | 31249.1 | 2576804 |
| before | 3 | 49933 | 1.211 | 44.687 | 53.810 | 59.961 | 0 | 101 to 105 | 31039.5 | 2576804 |
| before | 4 | 49935 | 1.202 | 46.417 | 57.466 | 65.183 | 0 | 101 to 105 | 31464.6 | 2577124 |
| before | 5 | 49939 | 1.216 | 47.577 | 58.495 | 74.143 | 0 | 101 to 105 | 31842.9 | 2576696 |
| after | 1 | 49923 | 1.256 | 38.173 | 68.449 | 76.801 | 0 | 101 to 105 | 44.2 | 130872 |
| after | 2 | 49920 | 1.241 | 37.394 | 69.045 | 82.391 | 0 | 101 to 105 | 43.8 | 130808 |
| after | 3 | 49935 | 1.227 | 36.575 | 66.949 | 72.841 | 0 | 101 to 105 | 43.9 | 130544 |
| after | 4 | 49931 | 1.245 | 37.742 | 69.543 | 80.844 | 0 | 101 to 105 | 45.3 | 130592 |
| after | 5 | 49934 | 1.222 | 35.206 | 66.740 | 74.422 | 0 | 101 to 105 | 44.0 | 132676 |

Records/s: before median 49933.328, range 49815.277 to 49939.136; after median 49931.133, range 49919.899 to 49935.240; after/before medians 1.000; ranges overlap: yes.

p50 ms: before median 1.211, range 1.199 to 1.264; after median 1.241, range 1.222 to 1.256; after/before medians 1.025; ranges overlap: yes.

p99 ms: before median 46.294, range 44.687 to 47.577; after median 37.394, range 35.206 to 38.173; after/before medians 0.808; ranges overlap: no.

