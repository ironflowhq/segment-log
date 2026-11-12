# M4 evidence summary

Conditions are in environment.txt. A hosted runner is a shared virtual machine: these are measurements under those conditions, not a target pass.

## Fixtures

| Fixture | Records | Sealed segments | Records per sealed segment | Record entry bytes | Active bytes | Active records | Directory bytes | Expected filter bytes | Build s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| after-keyed | 23015000 | 1000 | 23000 | 296 | 4440544 | 15000 | 7268864544 | 46744000 | 91.0 |
| after-unkeyed | 24015000 | 1000 | 24000 | 280 | 4200544 | 15000 | 6761176544 | 0 | 56.5 |
| before-keyed | 23015000 | 1000 | 23000 | 296 | 4440544 | 15000 | 6813240544 | none in M3 | 104.1 |

## Open time

Each run opens the after-side fixture once with `SegmentLog::open`, end to end. Target: 50 ms (docs/prd.md). Cold runs drop the page cache before each open; `page_cache_before_kib` shows the drop.

| Fixture | Cache | Runs | Min ms | p50 ms | Max ms | Page cache before, KiB |
| --- | --- | --- | --- | --- | --- | --- |
| keyed | cold | 10 | 574.182 | 587.744 | 604.616 | 282796 to 283228 |
| keyed | warm | 10 | 98.814 | 99.155 | 101.827 | 393504 to 393668 |
| unkeyed | cold | 10 | 489.041 | 501.834 | 543.760 | 282912 to 283916 |
| unkeyed | warm | 10 | 83.348 | 84.116 | 87.225 | 345208 to 345640 |

Keyed minus unkeyed p50, cold: 85.910 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

Keyed minus unkeyed p50, warm: 15.038 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

## Key-cache memory

- ADR 0048 rule 5, the filter part: 46744000 bytes for 23000000 sealed keys (2.032 bytes per key), from the key filter frame layout in format.md.
- Heap the open log holds (allocations live after open, first run): keyed 48612612 bytes, unkeyed 735602 bytes. The difference, 47877010 bytes, is the measured key-cache memory: the filters, the index-page fences and the active segment's exact map of 15000 keys.
- Measured minus the filter formula: 1133010 bytes, 75.5 bytes per active-segment key.
- Peak heap during open: keyed 48662294 bytes, unkeyed 951864 bytes.
- Resident set before and after the first open, KiB: keyed 2644 to 51572, unkeyed 2548 to 4528.

## Appends, M3 (before) and M4 (after)

### closed: keyed appends, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 6710886, start 23015000 records in 1001 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 192061 | 4.867 | 9.765 | 36.724 | 39.457 | 0 | 1001 to 1045 | 31575.9 | 2595636 |
| before | 2 | 231615 | 4.050 | 8.281 | 36.951 | 43.359 | 0 | 1001 to 1045 | 31596.7 | 2595964 |
| before | 3 | 198745 | 4.704 | 9.800 | 39.550 | 43.509 | 0 | 1001 to 1045 | 32171.8 | 2595832 |
| before | 4 | 209184 | 4.442 | 10.938 | 36.168 | 43.355 | 0 | 1001 to 1045 | 31950.3 | 2596012 |
| before | 5 | 177329 | 5.312 | 10.449 | 37.262 | 43.767 | 0 | 1001 to 1045 | 32108.1 | 2595864 |
| after | 1 | 154828 | 4.481 | 39.684 | 53.110 | 71.607 | 0 | 1001 to 1045 | 121.9 | 142028 |
| after | 2 | 166654 | 4.202 | 29.119 | 40.685 | 55.578 | 0 | 1001 to 1045 | 121.4 | 142404 |
| after | 3 | 160940 | 4.418 | 29.828 | 40.003 | 51.677 | 0 | 1001 to 1045 | 124.0 | 142176 |
| after | 4 | 154013 | 4.580 | 31.796 | 49.024 | 121.266 | 0 | 1001 to 1045 | 125.6 | 142060 |
| after | 5 | 147262 | 4.815 | 31.402 | 58.348 | 133.637 | 0 | 1001 to 1045 | 123.9 | 142420 |

Records/s: before median 198744.664, range 177329.200 to 231615.234; after median 154827.851, range 147261.824 to 166654.075; after/before medians 0.779; ranges overlap: no.

p50 ms: before median 4.704, range 4.050 to 5.312; after median 4.481, range 4.202 to 4.815; after/before medians 0.953; ranges overlap: yes.

p99 ms: before median 9.800, range 8.281 to 10.938; after median 31.402, range 29.119 to 39.684; after/before medians 3.204; ranges overlap: no.

### closed-unkeyed: unkeyed appends, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 6710886, start 23015000 records in 1001 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 288133 | 3.104 | 9.806 | 33.002 | 38.084 | 0 | 1001 to 1043 | 32046.6 | 2489264 |
| before | 2 | 302852 | 3.032 | 7.366 | 32.053 | 34.408 | 0 | 1001 to 1043 | 31950.9 | 2488860 |
| before | 3 | 285545 | 3.183 | 8.239 | 21.775 | 39.098 | 0 | 1001 to 1043 | 32030.8 | 2489344 |
| before | 4 | 296375 | 3.115 | 7.385 | 21.262 | 35.301 | 0 | 1001 to 1043 | 31654.5 | 2489368 |
| before | 5 | 304886 | 3.001 | 7.459 | 30.833 | 37.143 | 0 | 1001 to 1043 | 32219.6 | 2489640 |
| after | 1 | 245351 | 3.433 | 15.438 | 33.178 | 43.404 | 0 | 1001 to 1042 | 125.2 | 137344 |
| after | 2 | 222359 | 3.638 | 17.065 | 78.093 | 84.778 | 0 | 1001 to 1042 | 125.1 | 137160 |
| after | 3 | 229802 | 3.622 | 17.732 | 56.252 | 71.447 | 0 | 1001 to 1042 | 124.5 | 137308 |
| after | 4 | 242096 | 3.599 | 12.805 | 21.828 | 38.705 | 0 | 1001 to 1043 | 123.3 | 136952 |
| after | 5 | 248204 | 3.488 | 11.817 | 19.472 | 59.511 | 0 | 1001 to 1043 | 124.0 | 137364 |

Records/s: before median 296375.472, range 285545.447 to 304885.769; after median 242096.432, range 222358.518 to 248204.162; after/before medians 0.817; ranges overlap: no.

p50 ms: before median 3.104, range 3.001 to 3.183; after median 3.599, range 3.433 to 3.638; after/before medians 1.159; ranges overlap: no.

p99 ms: before median 7.459, range 7.366 to 9.806; after median 15.438, range 11.817 to 17.732; after/before medians 2.070; ranges overlap: no.

### paced: keyed appends, 1000 callers, 1000 rounds, interval 20000 µs, offered 50000 records/s, roll_bytes 6710886, start 23015000 records in 1001 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 49956 | 1.207 | 258.050 | 328.473 | 345.726 | 0 | 1001 to 1045 | 32334.7 | 2601012 |
| before | 2 | 49932 | 1.158 | 269.311 | 336.213 | 346.230 | 0 | 1001 to 1045 | 31877.6 | 2601596 |
| before | 3 | 49953 | 1.151 | 118.943 | 151.020 | 160.444 | 0 | 1001 to 1045 | 32253.6 | 2601644 |
| before | 4 | 49959 | 1.111 | 105.483 | 145.581 | 158.039 | 0 | 1001 to 1045 | 31461.7 | 2602292 |
| before | 5 | 49953 | 1.146 | 76.513 | 98.510 | 111.715 | 0 | 1001 to 1045 | 31877.0 | 2601796 |
| after | 1 | 49959 | 1.133 | 251.174 | 278.071 | 306.375 | 0 | 1001 to 1045 | 122.9 | 147160 |
| after | 2 | 49959 | 1.210 | 328.531 | 366.338 | 410.712 | 0 | 1001 to 1045 | 124.2 | 147008 |
| after | 3 | 49957 | 1.098 | 247.775 | 270.940 | 300.748 | 0 | 1001 to 1045 | 122.9 | 147292 |
| after | 4 | 49958 | 1.132 | 411.170 | 433.494 | 458.224 | 0 | 1001 to 1045 | 123.9 | 147756 |
| after | 5 | 49951 | 1.155 | 281.312 | 306.359 | 330.590 | 0 | 1001 to 1045 | 125.7 | 147376 |

Records/s: before median 49953.327, range 49932.089 to 49959.202; after median 49957.845, range 49950.788 to 49958.719; after/before medians 1.000; ranges overlap: yes.

p50 ms: before median 1.151, range 1.111 to 1.207; after median 1.133, range 1.098 to 1.210; after/before medians 0.984; ranges overlap: yes.

p99 ms: before median 118.943, range 76.513 to 269.311; after median 281.312, range 247.775 to 411.170; after/before medians 2.365; ranges overlap: yes.

### Key lookup cost on each side

Both modes append to the same keyed fixture. Unkeyed appends skip the key lookup, so the gap is what the lookup costs on that side (ADR 0051).

- before: keyed/unkeyed medians 0.671; keyed minus unkeyed time per record 1.66 µs.
- after: keyed/unkeyed medians 0.640; keyed minus unkeyed time per record 2.33 µs.

