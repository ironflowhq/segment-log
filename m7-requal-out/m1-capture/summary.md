# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 132124.073 | 7.083 | 14.761 | 44.137 | 0.364 | 0.451 | 1.902 | 89.835 | 312663976 |
| 2 | 131329.347 | 7.142 | 15.183 | 44.399 | 0.395 | 0.492 | 1.985 | 91.141 | 312655824 |
| 3 | 129952.630 | 7.118 | 15.692 | 66.767 | 0.394 | 0.502 | 2.003 | 92.704 | 312677000 |
| 4 | 126619.056 | 7.314 | 14.869 | 67.745 | 0.381 | 0.451 | 1.901 | 90.527 | 312665912 |
| 5 | 132197.473 | 7.083 | 14.436 | 46.510 | 0.380 | 0.444 | 1.888 | 91.679 | 312662848 |

Throughput records/s: median 131329.347, range 126619.056–132197.473.

Append p50 ms: median 7.118, range 7.083–7.314.

Append p99 ms: median 14.869, range 14.436–15.692.

Append p999 ms: median 46.510, range 44.137–67.745.

Device p50 ms: median 0.381, range 0.364–0.395.

Device p99 ms: median 0.451, range 0.444–0.502.

Reopen ms: median 91.141, range 89.835–92.704.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 7.482 | 7.569 | 2000 | 442 |
| 2 | 7.550 | 7.614 | 1998 | 460 |
| 3 | 7.616 | 7.695 | 1995 | 435 |
| 4 | 7.817 | 7.898 | 2000 | 439 |
| 5 | 7.478 | 7.564 | 2002 | 425 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49764.325 | 1.741 | 1.034 | 30.073 | 74.742 | 87.937 | 0.440 | 1.881 | +28.192 | 18979 | 188 |
| 2 | 49743.077 | 1.743 | 1.022 | 29.132 | 73.370 | 88.426 | 0.476 | 1.952 | +27.180 | 18937 | 181 |
| 3 | 49761.977 | 1.713 | 1.029 | 28.545 | 74.648 | 88.213 | 0.470 | 1.940 | +26.604 | 18952 | 190 |
| 4 | 49757.850 | 1.696 | 1.024 | 28.467 | 73.543 | 87.195 | 0.468 | 1.937 | +26.531 | 18970 | 186 |
| 5 | 49759.150 | 2.021 | 1.021 | 41.616 | 93.761 | 108.048 | 0.477 | 1.953 | +39.663 | 18926 | 177 |

Paced p50 ms: median 1.024, range 1.021–1.034.

Paced p99 ms: median 29.132, range 28.467–41.616.

Paced bound ms: median 1.940, range 1.881–1.953.

Paced runs with p99 within the bound of the same run: 0 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.172 | 0.409 | 1.818 | +28.255 |
| 2 | 0.177 | 0.254 | 1.507 | +27.624 |
| 3 | 0.179 | 0.306 | 1.611 | +26.933 |
| 4 | 0.179 | 0.284 | 1.568 | +26.899 |
| 5 | 0.172 | 0.247 | 1.493 | +40.123 |

Paced runs with p99 within the bound from their own group flushes: 0 of 5.
