# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 212362.206 | 4.366 | 14.209 | 43.709 | 0.345 | 1.426 | 3.852 | 54.957 | 312668544 |
| 2 | 210174.776 | 4.415 | 14.821 | 31.906 | 0.314 | 1.449 | 3.898 | 55.750 | 312663584 |
| 3 | 211670.970 | 4.364 | 13.469 | 45.213 | 0.343 | 1.414 | 3.828 | 55.711 | 312670416 |
| 4 | 209568.340 | 4.381 | 13.133 | 31.795 | 0.343 | 1.442 | 3.884 | 54.756 | 312669504 |
| 5 | 210088.655 | 4.378 | 13.502 | 40.533 | 0.342 | 1.407 | 3.814 | 55.039 | 312672784 |

Throughput records/s: median 210174.776, range 209568.340–212362.206.

Append p50 ms: median 4.378, range 4.364–4.415.

Append p99 ms: median 13.502, range 13.133–14.821.

Append p999 ms: median 40.533, range 31.795–45.213.

Device p50 ms: median 0.343, range 0.314–0.345.

Device p99 ms: median 1.426, range 1.407–1.449.

Reopen ms: median 55.039, range 54.756–55.750.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 4.669 | 4.709 | 1986 | 393 |
| 2 | 4.706 | 4.758 | 1897 | 380 |
| 3 | 4.679 | 4.724 | 1973 | 374 |
| 4 | 4.711 | 4.772 | 1961 | 391 |
| 5 | 4.701 | 4.760 | 1980 | 381 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49926.882 | 1.164 | 0.906 | 9.034 | 37.049 | 43.714 | 1.360 | 3.719 | +5.315 | 19128 | 185 |
| 2 | 49919.811 | 1.150 | 0.895 | 9.392 | 37.813 | 46.231 | 1.377 | 3.753 | +5.639 | 19217 | 171 |
| 3 | 49901.099 | 1.105 | 0.869 | 8.990 | 37.369 | 42.846 | 0.415 | 1.830 | +7.161 | 19314 | 145 |
| 4 | 49901.436 | 1.095 | 0.864 | 8.350 | 37.533 | 45.720 | 0.431 | 1.862 | +6.488 | 19315 | 139 |
| 5 | 49919.426 | 1.110 | 0.866 | 9.425 | 37.775 | 45.574 | 0.438 | 1.876 | +7.549 | 19305 | 137 |

Paced p50 ms: median 0.869, range 0.864–0.906.

Paced p99 ms: median 9.034, range 8.350–9.425.

Paced bound ms: median 1.876, range 1.830–3.753.

Paced runs with p99 within the bound of the same run: 0 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.136 | 1.196 | 3.393 | +5.641 |
| 2 | 0.131 | 1.146 | 3.292 | +6.101 |
| 3 | 0.140 | 0.187 | 1.373 | +7.617 |
| 4 | 0.137 | 0.192 | 1.384 | +6.966 |
| 5 | 0.134 | 0.188 | 1.376 | +8.049 |

Paced runs with p99 within the bound from their own group flushes: 0 of 5.
