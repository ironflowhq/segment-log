# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 218657.657 | 4.311 | 7.232 | 34.549 | 0.320 | 0.412 | 1.824 | 55.685 | 312672584 |
| 2 | 218205.324 | 4.315 | 7.368 | 44.372 | 0.285 | 0.366 | 1.733 | 55.712 | 312670432 |
| 3 | 221499.161 | 4.253 | 8.744 | 32.024 | 0.307 | 0.389 | 1.777 | 55.951 | 312660048 |
| 4 | 219213.087 | 4.293 | 7.980 | 41.053 | 0.303 | 0.374 | 1.749 | 55.412 | 312673800 |
| 5 | 219314.096 | 4.254 | 8.385 | 41.177 | 0.301 | 0.382 | 1.764 | 55.982 | 312664472 |

Throughput records/s: median 219213.087, range 218205.324–221499.161.

Append p50 ms: median 4.293, range 4.253–4.315.

Append p99 ms: median 7.980, range 7.232–8.744.

Append p999 ms: median 41.053, range 32.024–44.372.

Device p50 ms: median 0.303, range 0.285–0.320.

Device p99 ms: median 0.382, range 0.366–0.412.

Reopen ms: median 55.712, range 55.412–55.982.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 4.524 | 4.573 | 1985 | 337 |
| 2 | 4.529 | 4.583 | 1940 | 322 |
| 3 | 4.473 | 4.515 | 1993 | 313 |
| 4 | 4.523 | 4.562 | 1974 | 317 |
| 5 | 4.505 | 4.560 | 1990 | 310 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49913.191 | 1.059 | 0.856 | 4.136 | 36.495 | 43.544 | 0.397 | 1.795 | +2.342 | 19386 | 133 |
| 2 | 49918.349 | 1.052 | 0.871 | 2.490 | 34.592 | 43.533 | 0.374 | 1.748 | +0.742 | 19399 | 151 |
| 3 | 49924.336 | 1.067 | 0.861 | 5.039 | 36.600 | 42.878 | 0.393 | 1.786 | +3.254 | 19380 | 138 |
| 4 | 49896.622 | 1.068 | 0.859 | 4.856 | 37.139 | 45.671 | 0.381 | 1.761 | +3.095 | 19376 | 133 |
| 5 | 49927.742 | 1.050 | 0.859 | 3.371 | 35.105 | 42.046 | 0.346 | 1.691 | +1.679 | 19394 | 139 |

Paced p50 ms: median 0.859, range 0.856–0.871.

Paced p99 ms: median 4.136, range 2.490–5.039.

Paced bound ms: median 1.761, range 1.691–1.795.

Paced runs with p99 within the bound of the same run: 0 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.129 | 0.189 | 1.377 | +2.759 |
| 2 | 0.154 | 0.199 | 1.398 | +1.092 |
| 3 | 0.133 | 0.182 | 1.364 | +3.676 |
| 4 | 0.125 | 0.178 | 1.357 | +3.500 |
| 5 | 0.131 | 0.179 | 1.359 | +2.012 |

Paced runs with p99 within the bound from their own group flushes: 0 of 5.
