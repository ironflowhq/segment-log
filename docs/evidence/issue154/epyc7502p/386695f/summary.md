# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 181721.262 | 5.079 | 11.112 | 47.805 | 0.340 | 0.415 | 1.829 | 78.662 | 312646888 |
| 2 | 179546.167 | 5.169 | 12.640 | 33.206 | 0.343 | 0.431 | 1.862 | 80.025 | 312667784 |
| 3 | 177570.187 | 5.201 | 10.147 | 51.084 | 0.329 | 0.401 | 1.802 | 78.905 | 312666832 |
| 4 | 177236.641 | 5.179 | 11.278 | 49.213 | 0.282 | 0.370 | 1.741 | 78.328 | 312667824 |
| 5 | 178810.473 | 5.144 | 10.494 | 47.754 | 0.326 | 0.403 | 1.806 | 77.941 | 312658312 |

Throughput records/s: median 178810.473, range 177236.641–181721.262.

Append p50 ms: median 5.169, range 5.079–5.201.

Append p99 ms: median 11.112, range 10.147–12.640.

Append p999 ms: median 47.805, range 33.206–51.084.

Device p50 ms: median 0.329, range 0.282–0.343.

Device p99 ms: median 0.403, range 0.370–0.431.

Reopen ms: median 78.662, range 77.941–80.025.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 5.462 | 5.503 | 1324 | 360 |
| 2 | 5.519 | 5.570 | 1990 | 290 |
| 3 | 5.588 | 5.632 | 1940 | 289 |
| 4 | 5.594 | 5.642 | 1865 | 314 |
| 5 | 5.538 | 5.593 | 1593 | 328 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49894.715 | 1.121 | 0.865 | 6.101 | 50.615 | 64.175 | 0.387 | 1.774 | +4.326 | 19379 | 135 |
| 2 | 49901.803 | 1.127 | 0.866 | 7.178 | 50.654 | 65.465 | 0.400 | 1.800 | +5.378 | 19378 | 135 |
| 3 | 49928.202 | 1.037 | 0.871 | 1.743 | 32.553 | 48.468 | 0.394 | 1.787 | -0.044 | 19418 | 140 |
| 4 | 49937.769 | 1.032 | 0.868 | 1.686 | 32.102 | 47.404 | 0.386 | 1.773 | -0.087 | 19422 | 135 |
| 5 | 49912.832 | 1.108 | 0.866 | 5.472 | 49.047 | 63.979 | 0.404 | 1.808 | +3.664 | 19386 | 136 |

Paced p50 ms: median 0.866, range 0.865–0.871.

Paced p99 ms: median 5.472, range 1.686–7.178.

Paced bound ms: median 1.787, range 1.773–1.808.

Paced runs with p99 within the bound of the same run: 2 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.129 | 0.169 | 1.337 | +4.764 |
| 2 | 0.134 | 0.166 | 1.332 | +5.846 |
| 3 | 0.135 | 0.179 | 1.358 | +0.385 |
| 4 | 0.135 | 0.170 | 1.341 | +0.345 |
| 5 | 0.131 | 0.176 | 1.351 | +4.121 |

Paced runs with p99 within the bound from their own group flushes: 0 of 5.
