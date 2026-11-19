# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 312385.676 | 3.047 | 5.964 | 17.624 | 0.392 | 0.928 | 2.856 | 41.724 | 312648128 |
| 2 | 315146.058 | 3.024 | 5.687 | 17.656 | 0.372 | 0.797 | 2.594 | 41.811 | 312648688 |
| 3 | 316491.030 | 3.007 | 5.792 | 17.596 | 0.368 | 0.459 | 1.919 | 41.242 | 312649784 |
| 4 | 315149.840 | 3.024 | 6.098 | 17.006 | 0.356 | 0.434 | 1.868 | 41.709 | 312643560 |
| 5 | 316383.324 | 2.999 | 5.506 | 17.628 | 0.368 | 0.448 | 1.896 | 41.277 | 312651384 |

Throughput records/s: median 315149.840, range 312385.676–316491.030.

Append p50 ms: median 3.024, range 2.999–3.047.

Append p99 ms: median 5.792, range 5.506–6.098.

Append p999 ms: median 17.624, range 17.006–17.656.

Device p50 ms: median 0.368, range 0.356–0.392.

Device p99 ms: median 0.459, range 0.434–0.928.

Reopen ms: median 41.709, range 41.242–41.811.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 3.170 | 3.201 | 1237 | 438 |
| 2 | 3.151 | 3.173 | 1239 | 429 |
| 3 | 3.123 | 3.160 | 1239 | 412 |
| 4 | 3.141 | 3.173 | 1236 | 417 |
| 5 | 3.124 | 3.161 | 1236 | 401 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49978.846 | 0.896 | 0.853 | 1.365 | 15.637 | 21.186 | 0.461 | 1.922 | -0.557 | 19430 | 192 |
| 2 | 49978.008 | 0.882 | 0.840 | 1.340 | 15.097 | 20.104 | 0.464 | 1.928 | -0.589 | 19429 | 180 |
| 3 | 49969.139 | 0.899 | 0.856 | 1.364 | 15.939 | 21.724 | 0.451 | 1.902 | -0.537 | 19430 | 191 |
| 4 | 49978.771 | 0.886 | 0.842 | 1.349 | 15.214 | 21.094 | 0.433 | 1.867 | -0.517 | 19427 | 180 |
| 5 | 49973.961 | 0.877 | 0.836 | 1.336 | 14.655 | 20.787 | 0.478 | 1.955 | -0.619 | 19430 | 175 |

Paced p50 ms: median 0.842, range 0.836–0.856.

Paced p99 ms: median 1.349, range 1.336–1.365.

Paced bound ms: median 1.922, range 1.867–1.955.

Paced runs with p99 within the bound of the same run: 5 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.192 | 0.250 | 1.500 | -0.135 |
| 2 | 0.178 | 0.226 | 1.452 | -0.113 |
| 3 | 0.190 | 0.242 | 1.483 | -0.119 |
| 4 | 0.176 | 0.232 | 1.464 | -0.115 |
| 5 | 0.172 | 0.226 | 1.452 | -0.115 |

Paced runs with p99 within the bound from their own group flushes: 5 of 5.
