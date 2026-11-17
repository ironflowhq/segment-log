# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 259233.142 | 3.611 | 7.392 | 22.828 | 0.319 | 0.494 | 1.988 | 58.103 | 312641232 |
| 2 | 257785.330 | 3.625 | 7.445 | 22.913 | 0.369 | 0.526 | 2.052 | 56.968 | 312639552 |
| 3 | 257691.160 | 3.643 | 7.074 | 22.634 | 0.238 | 0.465 | 1.930 | 59.290 | 312639616 |
| 4 | 259769.872 | 3.619 | 7.655 | 23.791 | 0.377 | 0.564 | 2.129 | 57.569 | 312638280 |
| 5 | 258188.221 | 3.626 | 7.348 | 22.838 | 0.360 | 0.519 | 2.037 | 60.537 | 312642848 |

Throughput records/s: median 258188.221, range 257691.160–259769.872.

Append p50 ms: median 3.625, range 3.611–3.643.

Append p99 ms: median 7.392, range 7.074–7.655.

Append p999 ms: median 22.838, range 22.634–23.791.

Device p50 ms: median 0.360, range 0.238–0.377.

Device p99 ms: median 0.519, range 0.465–0.564.

Reopen ms: median 58.103, range 56.968–60.537.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 3.841 | 3.858 | 1821 | 315 |
| 2 | 3.862 | 3.879 | 2001 | 277 |
| 3 | 3.863 | 3.881 | 2001 | 286 |
| 4 | 3.833 | 3.850 | 2001 | 293 |
| 5 | 3.856 | 3.873 | 2001 | 281 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49965.807 | 1.033 | 0.933 | 1.482 | 30.923 | 47.472 | 0.518 | 2.035 | -0.553 | 19319 | 179 |
| 2 | 49963.435 | 1.023 | 0.923 | 1.464 | 30.907 | 47.487 | 0.513 | 2.027 | -0.563 | 19319 | 171 |
| 3 | 49969.299 | 1.020 | 0.924 | 1.463 | 30.012 | 47.510 | 0.502 | 2.004 | -0.541 | 19324 | 174 |
| 4 | 49965.961 | 1.027 | 0.928 | 1.470 | 31.098 | 47.732 | 0.566 | 2.131 | -0.661 | 19321 | 173 |
| 5 | 49969.753 | 1.032 | 0.935 | 1.476 | 30.553 | 47.438 | 0.463 | 1.925 | -0.449 | 19328 | 179 |

Paced p50 ms: median 0.928, range 0.923–0.935.

Paced p99 ms: median 1.470, range 1.463–1.482.

Paced bound ms: median 2.027, range 1.925–2.131.

Paced runs with p99 within the bound of the same run: 5 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.177 | 0.244 | 1.487 | -0.005 |
| 2 | 0.169 | 0.228 | 1.455 | +0.009 |
| 3 | 0.173 | 0.235 | 1.471 | -0.008 |
| 4 | 0.170 | 0.235 | 1.470 | +0.000 |
| 5 | 0.178 | 0.245 | 1.490 | -0.013 |

Paced runs with p99 within the bound from their own group flushes: 3 of 5.
