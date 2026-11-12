# M4 evidence summary

Conditions are in environment.txt. A hosted runner is a shared virtual machine: these are measurements under those conditions, not a target pass.

## Fixtures

| Fixture | Records | Sealed segments | Records per sealed segment | Record entry bytes | Active bytes | Active records | Directory bytes | Expected filter bytes | Build s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| after-keyed | 23015000 | 1000 | 23000 | 296 | 4440544 | 15000 | 7268864544 | 46744000 | 388.4 |
| after-unkeyed | 24015000 | 1000 | 24000 | 280 | 4200544 | 15000 | 6761176544 | 0 | 62.9 |
| before-keyed | 23015000 | 1000 | 23000 | 296 | 4440544 | 15000 | 6813240544 | none in M3 | 105.6 |

## Open time

Each run opens the after-side fixture once with `SegmentLog::open`, end to end. Target: 50 ms (docs/prd.md). Cold runs drop the page cache before each open; `page_cache_before_kib` shows the drop.

| Fixture | Cache | Runs | Min ms | p50 ms | Max ms | Page cache before, KiB |
| --- | --- | --- | --- | --- | --- | --- |
| keyed | cold | 10 | 697.619 | 750.858 | 796.586 | 279904 to 283056 |
| keyed | warm | 10 | 132.528 | 134.093 | 142.606 | 393216 to 393428 |
| unkeyed | cold | 10 | 620.394 | 646.767 | 668.382 | 282768 to 283136 |
| unkeyed | warm | 10 | 111.721 | 114.122 | 126.007 | 345088 to 345220 |

Keyed minus unkeyed p50, cold: 104.091 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

Keyed minus unkeyed p50, warm: 19.972 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

## Key-cache memory

- ADR 0048 rule 5, the filter part: 46744000 bytes for 23000000 sealed keys (2.032 bytes per key), from the key filter frame layout in format.md.
- Heap the open log holds (allocations live after open, first run): keyed 48499170 bytes, unkeyed 621542 bytes. The difference, 47877628 bytes, is the measured key-cache memory: the filters, the index-page fences and the active segment's exact map of 15000 keys.
- Measured minus the filter formula: 1133628 bytes, 75.6 bytes per active-segment key.
- Peak heap during open: keyed 48662294 bytes, unkeyed 951864 bytes.
- Resident set before and after the first open, KiB: keyed 2428 to 51360, unkeyed 2448 to 4380.

## Appends, M3 (before) and M4 (after)

### closed: keyed appends, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 6710886, start 23015000 records in 1001 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 205285 | 4.558 | 8.045 | 41.376 | 44.521 | 0 | 1001 to 1045 | 31667.3 | 2595808 |
| before | 2 | 206270 | 4.558 | 8.548 | 44.581 | 49.692 | 0 | 1001 to 1045 | 31572.5 | 2595832 |
| before | 3 | 209465 | 4.392 | 10.305 | 46.025 | 51.597 | 0 | 1001 to 1045 | 31184.5 | 2595592 |
| before | 4 | 211119 | 4.438 | 8.336 | 43.911 | 48.437 | 0 | 1001 to 1045 | 31330.7 | 2595528 |
| before | 5 | 207194 | 4.482 | 9.221 | 44.779 | 49.680 | 0 | 1001 to 1045 | 31091.1 | 2595904 |
| after | 1 | 26602 | 34.945 | 105.099 | 136.021 | 141.292 | 0 | 1001 to 1045 | 149.8 | 96756 |
| after | 2 | 29048 | 34.281 | 44.399 | 68.865 | 73.595 | 0 | 1001 to 1045 | 149.0 | 96872 |
| after | 3 | 29150 | 34.165 | 42.683 | 68.555 | 74.997 | 0 | 1001 to 1045 | 149.4 | 97620 |
| after | 4 | 29105 | 34.259 | 41.541 | 68.102 | 73.764 | 0 | 1001 to 1045 | 151.7 | 97124 |
| after | 5 | 29164 | 34.161 | 41.944 | 68.519 | 77.585 | 0 | 1001 to 1045 | 150.6 | 96692 |

Records/s: before median 207194.311, range 205285.000 to 211119.293; after median 29104.665, range 26602.431 to 29164.146; after/before medians 0.140; ranges overlap: no.

p50 ms: before median 4.482, range 4.392 to 4.558; after median 34.259, range 34.161 to 34.945; after/before medians 7.645; ranges overlap: no.

p99 ms: before median 8.548, range 8.045 to 10.305; after median 42.683, range 41.541 to 105.099; after/before medians 4.994; ranges overlap: no.

### closed-unkeyed: unkeyed appends, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 6710886, start 23015000 records in 1001 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 304581 | 3.009 | 6.597 | 23.031 | 47.695 | 0 | 1001 to 1043 | 31212.7 | 2489144 |
| before | 2 | 311260 | 2.954 | 6.460 | 22.530 | 47.383 | 0 | 1001 to 1042 | 31134.2 | 2489440 |
| before | 3 | 305404 | 2.979 | 7.316 | 16.401 | 49.121 | 0 | 1001 to 1042 | 31243.9 | 2489072 |
| before | 4 | 307248 | 2.954 | 7.143 | 30.254 | 42.010 | 0 | 1001 to 1042 | 31276.9 | 2489212 |
| before | 5 | 306003 | 2.997 | 6.801 | 22.783 | 42.754 | 0 | 1001 to 1042 | 31314.7 | 2489280 |
| after | 1 | 285911 | 3.299 | 6.661 | 11.877 | 14.825 | 0 | 1001 to 1042 | 149.0 | 91352 |
| after | 2 | 280571 | 3.346 | 6.985 | 13.601 | 15.435 | 0 | 1001 to 1042 | 150.9 | 91528 |
| after | 3 | 278975 | 3.365 | 6.936 | 12.520 | 15.846 | 0 | 1001 to 1042 | 149.3 | 91508 |
| after | 4 | 288487 | 3.283 | 6.480 | 8.755 | 15.087 | 0 | 1001 to 1042 | 142.7 | 91608 |
| after | 5 | 285541 | 3.256 | 7.101 | 13.329 | 24.291 | 0 | 1001 to 1043 | 150.4 | 91596 |

Records/s: before median 306002.829, range 304581.195 to 311259.539; after median 285541.231, range 278975.417 to 288487.392; after/before medians 0.933; ranges overlap: no.

p50 ms: before median 2.979, range 2.954 to 3.009; after median 3.299, range 3.256 to 3.365; after/before medians 1.107; ranges overlap: no.

p99 ms: before median 6.801, range 6.460 to 7.316; after median 6.936, range 6.480 to 7.101; after/before medians 1.020; ranges overlap: yes.

### paced: keyed appends, 1000 callers, 1000 rounds, interval 20000 µs, offered 50000 records/s, roll_bytes 6710886, start 23015000 records in 1001 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 49940 | 1.190 | 45.568 | 55.696 | 62.556 | 0 | 1001 to 1045 | 31238.3 | 2602408 |
| before | 2 | 49938 | 1.189 | 45.792 | 55.520 | 63.726 | 0 | 1001 to 1045 | 31443.2 | 2603260 |
| before | 3 | 49937 | 1.184 | 45.494 | 55.523 | 63.738 | 0 | 1001 to 1045 | 31147.8 | 2602620 |
| before | 4 | 49939 | 1.181 | 46.016 | 55.767 | 61.937 | 0 | 1001 to 1045 | 31761.9 | 2603264 |
| before | 5 | 49935 | 1.193 | 46.132 | 55.029 | 64.853 | 0 | 1001 to 1045 | 31670.4 | 2602592 |
| after | 1 | 29146 | 6913.865 | 14143.466 | 14305.585 | 14415.765 | 0 | 1001 to 1045 | 148.6 | 96908 |
| after | 2 | 28984 | 7059.450 | 14338.730 | 14496.163 | 14602.024 | 0 | 1001 to 1045 | 150.3 | 96808 |
| after | 3 | 29293 | 6878.971 | 13974.957 | 14132.141 | 14209.751 | 0 | 1001 to 1045 | 149.0 | 96656 |
| after | 4 | 29111 | 7009.622 | 14193.118 | 14344.777 | 14441.106 | 0 | 1001 to 1045 | 150.3 | 96968 |
| after | 5 | 28675 | 7107.498 | 14705.949 | 14869.748 | 14968.785 | 0 | 1001 to 1045 | 149.3 | 96120 |

Records/s: before median 49937.872, range 49935.007 to 49940.255; after median 29111.143, range 28675.250 to 29292.543; after/before medians 0.583; ranges overlap: no.

p50 ms: before median 1.189, range 1.181 to 1.193; after median 7009.622, range 6878.971 to 7107.498; after/before medians 5893.153; ranges overlap: no.

p99 ms: before median 45.792, range 45.494 to 46.132; after median 14193.118, range 13974.957 to 14705.949; after/before medians 309.950; ranges overlap: no.

### Key lookup cost on each side

Both modes append to the same keyed fixture. Unkeyed appends skip the key lookup, so the gap is what the lookup costs on that side (ADR 0051).

- before: keyed/unkeyed medians 0.677; keyed minus unkeyed time per record 1.56 µs.
- after: keyed/unkeyed medians 0.102; keyed minus unkeyed time per record 30.86 µs.

