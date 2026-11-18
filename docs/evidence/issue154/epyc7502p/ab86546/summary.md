# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 203975.515 | 4.467 | 11.625 | 76.415 | 0.322 | 0.399 | 1.799 | 55.596 | 312676416 |
| 2 | 206260.686 | 4.455 | 13.524 | 60.773 | 0.337 | 0.404 | 1.809 | 55.041 | 312681792 |
| 3 | 203383.180 | 4.465 | 13.423 | 77.494 | 0.334 | 0.402 | 1.804 | 55.653 | 312672688 |
| 4 | 205028.253 | 4.448 | 13.233 | 75.153 | 0.337 | 0.392 | 1.785 | 55.029 | 312677928 |
| 5 | 202741.873 | 4.496 | 13.353 | 73.293 | 0.338 | 0.415 | 1.831 | 55.306 | 312670192 |

Throughput records/s: median 203975.515, range 202741.873–206260.686.

Append p50 ms: median 4.465, range 4.448–4.496.

Append p99 ms: median 13.353, range 11.625–13.524.

Append p999 ms: median 75.153, range 60.773–77.494.

Device p50 ms: median 0.337, range 0.322–0.338.

Device p99 ms: median 0.402, range 0.392–0.415.

Reopen ms: median 55.306, range 55.029–55.653.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 4.856 | 4.903 | 1999 | 346 |
| 2 | 4.804 | 4.848 | 1999 | 319 |
| 3 | 4.862 | 4.917 | 2002 | 330 |
| 4 | 4.823 | 4.877 | 2002 | 321 |
| 5 | 4.876 | 4.932 | 1999 | 322 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49902.420 | 1.466 | 0.865 | 27.124 | 78.671 | 96.290 | 0.418 | 1.836 | +25.288 | 19199 | 134 |
| 2 | 49898.262 | 1.493 | 0.875 | 28.352 | 77.042 | 94.360 | 0.430 | 1.860 | +26.492 | 19189 | 142 |
| 3 | 49893.728 | 1.478 | 0.868 | 28.011 | 76.233 | 93.717 | 0.427 | 1.854 | +26.157 | 19195 | 138 |
| 4 | 49895.716 | 1.395 | 0.870 | 25.194 | 61.054 | 78.305 | 0.389 | 1.778 | +23.416 | 19215 | 141 |
| 5 | 49894.701 | 1.471 | 0.873 | 27.258 | 74.396 | 92.295 | 0.408 | 1.816 | +25.442 | 19191 | 141 |

Paced p50 ms: median 0.870, range 0.865–0.875.

Paced p99 ms: median 27.258, range 25.194–28.352.

Paced bound ms: median 1.836, range 1.778–1.860.

Paced runs with p99 within the bound of the same run: 0 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.128 | 0.182 | 1.364 | +25.760 |
| 2 | 0.134 | 0.192 | 1.384 | +26.968 |
| 3 | 0.135 | 0.182 | 1.365 | +26.647 |
| 4 | 0.135 | 0.199 | 1.399 | +23.795 |
| 5 | 0.137 | 0.186 | 1.372 | +25.885 |

Paced runs with p99 within the bound from their own group flushes: 0 of 5.
