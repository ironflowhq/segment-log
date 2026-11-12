# M4 evidence summary

Conditions are in environment.txt. A hosted runner is a shared virtual machine: these are measurements under those conditions, not a target pass.

## Fixtures

| Fixture | Records | Sealed segments | Records per sealed segment | Record entry bytes | Active bytes | Active records | Directory bytes | Expected filter bytes | Build s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| after-keyed | 22715000 | 100 | 227000 | 296 | 4440544 | 15000 | 7139055744 | 46114400 | 70.9 |
| after-unkeyed | 24015000 | 100 | 240000 | 280 | 4200544 | 15000 | 6728589344 | 0 | 43.8 |
| before-keyed | 22715000 | 100 | 227000 | 296 | 4440544 | 15000 | 6724373344 | none in M3 | 88.9 |

## Open time

Each run opens the after-side fixture once with `SegmentLog::open`, end to end. Target: 50 ms (docs/prd.md). Cold runs drop the page cache before each open; `page_cache_before_kib` shows the drop.

| Fixture | Cache | Runs | Min ms | p50 ms | Max ms | Page cache before, KiB |
| --- | --- | --- | --- | --- | --- | --- |
| keyed | cold | 10 | 112.555 | 115.800 | 143.054 | 281712 to 282144 |
| keyed | warm | 10 | 38.025 | 38.552 | 42.308 | 358072 to 358176 |
| unkeyed | cold | 10 | 73.124 | 78.463 | 86.730 | 281912 to 282100 |
| unkeyed | warm | 10 | 17.147 | 17.726 | 18.905 | 307960 to 308104 |

Keyed minus unkeyed p50, cold: 37.337 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

Keyed minus unkeyed p50, warm: 20.825 ms. The fixtures differ in their keys, their key frames and 16 bytes per record.

## Key-cache memory

- ADR 0048 rule 5, the filter part: 46114400 bytes for 22700000 sealed keys (2.031 bytes per key), from the key filter frame layout in format.md.
- Heap the open log holds (allocations live after open, first run): keyed 47739172 bytes, unkeyed 490952 bytes. The difference, 47248220 bytes, is the measured key-cache memory: the filters, the index-page fences and the active segment's exact map of 15000 keys.
- Measured minus the filter formula: 1133820 bytes, 75.6 bytes per active-segment key.
- Peak heap during open: keyed 49121791 bytes, unkeyed 801080 bytes.
- Resident set before and after the first open, KiB: keyed 2364 to 53256, unkeyed 2352 to 3832.

## Appends, M3 (before) and M4 (after)

### closed: keyed appends, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 67108864, start 22715000 records in 101 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 189328 | 4.890 | 9.978 | 43.233 | 50.814 | 0 | 101 to 105 | 32815.2 | 2569892 |
| before | 2 | 198662 | 4.660 | 9.263 | 43.397 | 49.483 | 0 | 101 to 105 | 32890.2 | 2569976 |
| before | 3 | 198792 | 4.691 | 9.379 | 43.148 | 48.159 | 0 | 101 to 105 | 32850.2 | 2570036 |
| before | 4 | 191639 | 4.822 | 9.762 | 49.083 | 53.675 | 0 | 101 to 105 | 32958.1 | 2569888 |
| before | 5 | 184641 | 5.011 | 10.544 | 47.565 | 52.027 | 0 | 101 to 105 | 32960.9 | 2569928 |
| after | 1 | 146906 | 5.408 | 21.543 | 81.984 | 92.910 | 0 | 101 to 105 | 43.7 | 172668 |
| after | 2 | 158249 | 5.433 | 13.022 | 79.900 | 93.959 | 0 | 101 to 105 | 43.7 | 172712 |
| after | 3 | 162768 | 5.316 | 12.026 | 77.790 | 83.570 | 0 | 101 to 105 | 43.4 | 172640 |
| after | 4 | 166993 | 5.151 | 12.291 | 76.250 | 94.482 | 0 | 101 to 105 | 44.3 | 172824 |
| after | 5 | 162564 | 5.293 | 13.078 | 76.170 | 84.025 | 0 | 101 to 105 | 42.9 | 172824 |

Records/s: before median 191638.799, range 184640.802 to 198791.993; after median 162564.064, range 146906.484 to 166993.061; after/before medians 0.848; ranges overlap: no.

p50 ms: before median 4.822, range 4.660 to 5.011; after median 5.316, range 5.151 to 5.433; after/before medians 1.102; ranges overlap: no.

p99 ms: before median 9.762, range 9.263 to 10.544; after median 13.022, range 12.026 to 21.543; after/before medians 1.334; ranges overlap: no.

### closed-unkeyed: unkeyed appends, 1000 callers, 1000 rounds, interval closed_loop µs, offered closed_loop records/s, roll_bytes 67108864, start 22715000 records in 101 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 292718 | 3.131 | 7.505 | 17.781 | 41.508 | 0 | 101 to 105 | 32989.6 | 2463544 |
| before | 2 | 291755 | 3.121 | 7.776 | 29.361 | 33.769 | 0 | 101 to 105 | 32911.8 | 2463364 |
| before | 3 | 291415 | 3.105 | 7.527 | 27.786 | 34.428 | 0 | 101 to 105 | 32784.7 | 2463368 |
| before | 4 | 286745 | 3.211 | 7.620 | 9.478 | 14.774 | 0 | 101 to 105 | 32761.3 | 2463496 |
| before | 5 | 289221 | 3.091 | 7.957 | 41.513 | 48.769 | 0 | 101 to 105 | 33120.3 | 2463400 |
| after | 1 | 243245 | 3.740 | 9.480 | 13.531 | 24.928 | 0 | 101 to 105 | 43.9 | 135376 |
| after | 2 | 243589 | 3.739 | 9.541 | 15.363 | 47.252 | 0 | 101 to 105 | 44.4 | 135248 |
| after | 3 | 247816 | 3.698 | 8.866 | 13.433 | 24.416 | 0 | 101 to 105 | 44.9 | 135588 |
| after | 4 | 246392 | 3.700 | 9.039 | 14.832 | 26.942 | 0 | 101 to 105 | 43.7 | 135724 |
| after | 5 | 245443 | 3.741 | 8.743 | 13.157 | 19.140 | 0 | 101 to 105 | 43.7 | 135848 |

Records/s: before median 291414.848, range 286744.609 to 292718.480; after median 245442.984, range 243244.876 to 247815.504; after/before medians 0.842; ranges overlap: no.

p50 ms: before median 3.121, range 3.091 to 3.211; after median 3.739, range 3.698 to 3.741; after/before medians 1.198; ranges overlap: no.

p99 ms: before median 7.620, range 7.505 to 7.957; after median 9.039, range 8.743 to 9.541; after/before medians 1.186; ranges overlap: no.

### paced: keyed appends, 1000 callers, 1000 rounds, interval 20000 µs, offered 50000 records/s, roll_bytes 67108864, start 22715000 records in 101 segments

| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 49936 | 1.252 | 47.241 | 56.901 | 63.583 | 0 | 101 to 105 | 32815.8 | 2577208 |
| before | 2 | 49934 | 1.247 | 45.815 | 55.902 | 62.700 | 0 | 101 to 105 | 32891.1 | 2577064 |
| before | 3 | 49929 | 1.227 | 46.052 | 57.631 | 63.652 | 0 | 101 to 105 | 33097.4 | 2576840 |
| before | 4 | 49937 | 1.227 | 47.177 | 56.367 | 65.741 | 0 | 101 to 105 | 33181.1 | 2577064 |
| before | 5 | 49928 | 1.220 | 45.652 | 56.883 | 62.551 | 0 | 101 to 105 | 33095.0 | 2576772 |
| after | 1 | 49909 | 1.228 | 41.201 | 75.675 | 85.071 | 0 | 101 to 105 | 43.6 | 174920 |
| after | 2 | 49930 | 1.234 | 40.850 | 78.806 | 88.941 | 0 | 101 to 105 | 43.9 | 174476 |
| after | 3 | 49933 | 1.223 | 37.458 | 72.519 | 85.096 | 0 | 101 to 105 | 43.8 | 177252 |
| after | 4 | 49896 | 1.222 | 41.725 | 77.935 | 87.104 | 0 | 101 to 105 | 46.0 | 174916 |
| after | 5 | 49935 | 1.217 | 35.563 | 70.674 | 83.131 | 0 | 101 to 105 | 44.1 | 174992 |

Records/s: before median 49933.831, range 49928.484 to 49937.200; after median 49930.343, range 49895.852 to 49934.574; after/before medians 1.000; ranges overlap: yes.

p50 ms: before median 1.227, range 1.220 to 1.252; after median 1.223, range 1.217 to 1.234; after/before medians 0.997; ranges overlap: yes.

p99 ms: before median 46.052, range 45.652 to 47.241; after median 40.850, range 35.563 to 41.725; after/before medians 0.887; ranges overlap: no.

### Key lookup cost on each side

Both modes append to the same keyed fixture. Unkeyed appends skip the key lookup, so the gap is what the lookup costs on that side (ADR 0051).

- before: keyed/unkeyed medians 0.658; keyed minus unkeyed time per record 1.79 µs.
- after: keyed/unkeyed medians 0.662; keyed minus unkeyed time per record 2.08 µs.

