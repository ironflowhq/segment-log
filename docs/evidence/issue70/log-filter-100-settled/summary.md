# M4 evidence summary

Conditions are in environment.txt. A hosted runner is a shared virtual machine: these are measurements under those conditions, not a target pass.

## Fixtures

| Fixture | Records | Sealed segments | Records per sealed segment | Record entry bytes | Active bytes | Active records | Directory bytes | Expected filter bytes | Build s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| after-keyed | 22715000 | 100 | 227000 | 296 | 4440544 | 15000 | 7139055744 | 46114400 | 73.8 |
| after-unkeyed | 24015000 | 100 | 240000 | 280 | 4200544 | 15000 | 6728589344 | 0 | 41.3 |
| before-keyed | 22715000 | 100 | 227000 | 296 | 4440544 | 15000 | 6724373344 | none in M3 | 102.3 |

## Open time

Each run opens the after-side fixture once with `SegmentLog::open`, end to end. Target: 50 ms (docs/prd.md). Cold runs drop the page cache before each open; `page_cache_before_kib` shows the drop.

| Fixture | Cache | Runs | Min ms | p50 ms | Max ms | Page cache before, KiB |
| --- | --- | --- | --- | --- | --- | --- |
| keyed | cold | 10 | 101.072 | 103.100 | 133.385 | 284360 to 284652 |
| keyed | warm | 10 | 40.030 | 40.352 | 43.240 | 357436 to 357692 |
| unkeyed | cold | 10 | 66.143 | 70.144 | 76.448 | 284300 to 284580 |
| unkeyed | warm | 10 | 18.177 | 18.418 | 18.798 | 310568 to 310632 |

Keyed minus unkeyed p50, cold: 32.956 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

Keyed minus unkeyed p50, warm: 21.934 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

## Key-cache memory

- ADR 0048 rule 5, the filter part: 46114400 bytes for 22700000 sealed keys (2.031 bytes per key), from the key filter frame layout in format.md.
- Heap the open log holds (allocations live after open, first run): keyed 47739172 bytes, unkeyed 490322 bytes. The difference, 47248850 bytes, is the measured key-cache memory: the filters, the index-page fences and the active segment's exact map of 15000 keys.
- Measured minus the filter formula: 1134450 bytes, 75.6 bytes per active-segment key.
- Peak heap during open: keyed 48660647 bytes, unkeyed 801080 bytes.
- Resident set before and after the first open, KiB: keyed 2444 to 53252, unkeyed 2384 to 3812.

## Appends, M3 (before) and M4 (after)

### closed: keyed appends, settle 15000 ms, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 67108864, start 22715000 records in 101 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 147032 | 6.366 | 12.983 | 51.947 | 91.382 | 0 | 101 to 105 | 39651.1 | 2569472 |
| before | 2 | 158386 | 5.930 | 11.623 | 48.228 | 58.397 | 0 | 101 to 105 | 39396.7 | 2569728 |
| before | 3 | 168513 | 5.531 | 10.419 | 50.077 | 68.126 | 0 | 101 to 105 | 39758.8 | 2569668 |
| before | 4 | 166866 | 5.541 | 11.804 | 48.174 | 54.374 | 0 | 101 to 105 | 39798.8 | 2570180 |
| before | 5 | 150673 | 6.207 | 12.155 | 49.710 | 53.837 | 0 | 101 to 105 | 39322.9 | 2570032 |
| after | 1 | 140253 | 6.532 | 10.701 | 120.167 | 132.339 | 0 | 101 to 105 | 45.6 | 172764 |
| after | 2 | 141987 | 6.380 | 11.219 | 114.371 | 125.590 | 0 | 101 to 105 | 44.7 | 172752 |
| after | 3 | 148396 | 6.156 | 11.767 | 115.215 | 127.447 | 0 | 101 to 105 | 48.9 | 172784 |
| after | 4 | 155923 | 5.932 | 9.430 | 107.661 | 122.873 | 0 | 101 to 105 | 45.2 | 173188 |
| after | 5 | 141800 | 6.442 | 12.503 | 116.560 | 128.303 | 0 | 101 to 105 | 46.0 | 172992 |

Records/s: before median 158386.424, range 147032.348 to 168512.723; after median 141986.508, range 140253.289 to 155923.370; after/before medians 0.896; ranges overlap: yes.

p50 ms: before median 5.930, range 5.531 to 6.366; after median 6.380, range 5.932 to 6.532; after/before medians 1.076; ranges overlap: yes.

p99 ms: before median 11.804, range 10.419 to 12.983; after median 11.219, range 9.430 to 12.503; after/before medians 0.950; ranges overlap: yes.

### closed-unkeyed: unkeyed appends, settle 15000 ms, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 67108864, start 22715000 records in 101 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 243443 | 3.796 | 9.098 | 23.980 | 40.350 | 0 | 101 to 105 | 39593.9 | 2463540 |
| before | 2 | 240814 | 3.790 | 9.485 | 37.874 | 44.105 | 0 | 101 to 105 | 39543.0 | 2463284 |
| before | 3 | 238917 | 3.918 | 8.344 | 33.001 | 53.072 | 0 | 101 to 105 | 39551.0 | 2463552 |
| before | 4 | 219093 | 4.189 | 9.097 | 44.477 | 49.881 | 0 | 101 to 105 | 39922.3 | 2463776 |
| before | 5 | 224456 | 4.161 | 9.138 | 27.451 | 37.245 | 0 | 101 to 105 | 40108.1 | 2463488 |
| after | 1 | 221137 | 4.203 | 8.962 | 32.176 | 44.565 | 0 | 101 to 105 | 44.7 | 135276 |
| after | 2 | 231058 | 4.054 | 9.024 | 14.310 | 32.081 | 0 | 101 to 105 | 45.8 | 135052 |
| after | 3 | 227274 | 4.117 | 9.018 | 19.330 | 26.154 | 0 | 101 to 105 | 44.6 | 135464 |
| after | 4 | 210632 | 4.493 | 9.140 | 13.594 | 20.605 | 0 | 101 to 105 | 47.0 | 135324 |
| after | 5 | 219364 | 4.320 | 8.583 | 12.482 | 24.781 | 0 | 101 to 105 | 53.8 | 135448 |

Records/s: before median 238917.180, range 219093.102 to 243443.406; after median 221136.856, range 210632.378 to 231057.511; after/before medians 0.926; ranges overlap: yes.

p50 ms: before median 3.918, range 3.790 to 4.189; after median 4.203, range 4.054 to 4.493; after/before medians 1.073; ranges overlap: yes.

p99 ms: before median 9.098, range 8.344 to 9.485; after median 9.018, range 8.583 to 9.140; after/before medians 0.991; ranges overlap: yes.

### paced: keyed appends, settle 15000 ms, 1000 callers, 1000 rounds, interval 20000 µs, offered 50000 records/s, roll_bytes 67108864, start 22715000 records in 101 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 49929 | 1.136 | 51.382 | 61.424 | 71.680 | 0 | 101 to 105 | 40501.8 | 2576064 |
| before | 2 | 49923 | 1.133 | 53.203 | 67.181 | 83.727 | 0 | 101 to 105 | 39738.5 | 2575936 |
| before | 3 | 49934 | 1.125 | 52.474 | 64.130 | 79.997 | 0 | 101 to 105 | 39240.4 | 2575808 |
| before | 4 | 49922 | 1.229 | 53.478 | 66.541 | 82.048 | 0 | 101 to 105 | 39084.8 | 2576216 |
| before | 5 | 49925 | 1.155 | 51.996 | 61.524 | 69.280 | 0 | 101 to 105 | 40291.1 | 2575680 |
| after | 1 | 49928 | 1.133 | 70.426 | 105.118 | 125.023 | 0 | 101 to 105 | 47.9 | 173508 |
| after | 2 | 49931 | 1.139 | 66.149 | 99.483 | 109.818 | 0 | 101 to 105 | 45.8 | 173688 |
| after | 3 | 49933 | 1.115 | 62.627 | 97.487 | 111.198 | 0 | 101 to 105 | 45.4 | 173576 |
| after | 4 | 49929 | 1.131 | 64.438 | 101.285 | 108.402 | 0 | 101 to 105 | 45.7 | 173444 |
| after | 5 | 49926 | 1.164 | 71.676 | 109.148 | 119.372 | 0 | 101 to 105 | 46.1 | 173780 |

Records/s: before median 49925.160, range 49921.871 to 49934.030; after median 49928.963, range 49925.797 to 49933.344; after/before medians 1.000; ranges overlap: yes.

p50 ms: before median 1.136, range 1.125 to 1.229; after median 1.133, range 1.115 to 1.164; after/before medians 0.997; ranges overlap: yes.

p99 ms: before median 52.474, range 51.382 to 53.478; after median 66.149, range 62.627 to 71.676; after/before medians 1.261; ranges overlap: no.

### Key lookup cost on each side

Both modes append to the same keyed fixture. Unkeyed appends skip the key lookup, so the gap is what the lookup costs on that side (ADR 0051).

- before: keyed/unkeyed medians 0.663; keyed minus unkeyed time per record 2.13 µs.
- after: keyed/unkeyed medians 0.642; keyed minus unkeyed time per record 2.52 µs.

