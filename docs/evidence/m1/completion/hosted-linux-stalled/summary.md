# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 131731.965 | 3.837 | 82.856 | 501.624 | 1.494 | 109.911 | 220.822 | 794.867 | 296064448 |
| 2 | 169138.544 | 4.062 | 29.158 | 204.011 | 1.420 | 20.407 | 41.814 | 692.730 | 296065152 |
| 3 | 132490.211 | 4.471 | 88.711 | 198.396 | 1.470 | 23.975 | 48.950 | 719.763 | 296065088 |
| 4 | 172319.115 | 4.519 | 30.761 | 88.482 | 1.400 | 57.369 | 115.738 | 934.110 | 296065088 |
| 5 | 126388.192 | 4.340 | 88.013 | 174.782 | 1.427 | 63.017 | 127.035 | 709.139 | 296064608 |

Throughput records/s: median 132490.211, range 126388.192–172319.115.

Append p50 ms: median 4.340, range 3.837–4.519.

Append p99 ms: median 82.856, range 29.158–88.711.

Append p999 ms: median 198.396, range 88.482–501.624.

Device p50 ms: median 1.427, range 1.400–1.494.

Device p99 ms: median 57.369, range 20.407–109.911.

Reopen ms: median 719.763, range 692.730–934.110.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 7.497 | 7.591 | 2012 | 2237 |
| 2 | 5.875 | 5.912 | 2034 | 1211 |
| 3 | 7.504 | 7.548 | 2032 | 1805 |
| 4 | 5.738 | 5.803 | 2032 | 950 |
| 5 | 7.845 | 7.912 | 2017 | 2170 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49627.747 | 29.442 | 1.562 | 219.221 | 330.503 | 388.639 | 56.336 | 113.672 | +105.549 | 11433 | 927 |
| 2 | 49907.612 | 44.093 | 3.291 | 452.273 | 581.556 | 632.183 | 12.186 | 25.371 | +426.902 | 10710 | 1023 |
| 3 | 49946.292 | 31.596 | 2.625 | 276.882 | 373.421 | 407.805 | 6.791 | 14.581 | +262.301 | 10968 | 982 |
| 4 | 49948.992 | 97.337 | 24.692 | 656.042 | 749.975 | 829.518 | 7.688 | 16.375 | +639.667 | 8656 | 1434 |
| 5 | 49908.157 | 40.123 | 1.409 | 346.644 | 408.056 | 463.706 | 4.060 | 9.120 | +337.523 | 11739 | 879 |

Paced p50 ms: median 2.625, range 1.409–24.692.

Paced p99 ms: median 346.644, range 219.221–656.042.

Paced bound ms: median 16.375, range 9.120–113.672.

Paced runs with p99 within the bound of the same run: 0 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.261 | 16.007 | 33.013 | +186.207 |
| 2 | 0.264 | 15.620 | 32.239 | +420.034 |
| 3 | 0.259 | 18.013 | 37.026 | +239.856 |
| 4 | 0.315 | 21.605 | 44.210 | +611.831 |
| 5 | 0.256 | 9.740 | 20.480 | +326.163 |

Paced runs with p99 within the bound from their own group flushes: 0 of 5.
