# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 221505.206 | 4.216 | 13.192 | 43.682 | 0.319 | 0.394 | 1.787 | 54.908 | 312668720 |
| 2 | 218022.436 | 4.263 | 12.983 | 45.064 | 0.306 | 0.382 | 1.763 | 54.474 | 312673680 |
| 3 | 216128.695 | 4.292 | 12.830 | 44.530 | 0.304 | 0.376 | 1.752 | 54.302 | 312674624 |
| 4 | 216666.529 | 4.308 | 12.905 | 35.132 | 0.305 | 0.385 | 1.769 | 54.167 | 312668864 |
| 5 | 219615.914 | 4.253 | 13.120 | 40.821 | 0.308 | 0.380 | 1.760 | 54.916 | 312673208 |

Throughput records/s: median 218022.436, range 216128.695–221505.206.

Append p50 ms: median 4.263, range 4.216–4.308.

Append p99 ms: median 12.983, range 12.830–13.192.

Append p999 ms: median 43.682, range 35.132–45.064.

Device p50 ms: median 0.306, range 0.304–0.319.

Device p99 ms: median 0.382, range 0.376–0.394.

Reopen ms: median 54.474, range 54.167–54.916.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 4.472 | 4.515 | 1941 | 305 |
| 2 | 4.536 | 4.587 | 1977 | 285 |
| 3 | 4.579 | 4.627 | 1982 | 303 |
| 4 | 4.576 | 4.615 | 1969 | 298 |
| 5 | 4.506 | 4.553 | 1976 | 293 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49928.180 | 1.085 | 0.859 | 8.399 | 35.337 | 40.347 | 0.346 | 1.691 | +6.708 | 19311 | 133 |
| 2 | 49899.467 | 1.096 | 0.852 | 9.164 | 37.984 | 44.104 | 0.388 | 1.777 | +7.387 | 19307 | 128 |
| 3 | 49919.859 | 1.096 | 0.858 | 8.817 | 37.566 | 45.448 | 0.376 | 1.752 | +7.065 | 19313 | 134 |
| 4 | 49926.916 | 1.101 | 0.861 | 9.003 | 37.155 | 43.679 | 0.391 | 1.781 | +7.222 | 19307 | 133 |
| 5 | 49914.566 | 1.099 | 0.869 | 8.945 | 37.159 | 44.959 | 0.370 | 1.741 | +7.204 | 19307 | 141 |

Paced p50 ms: median 0.859, range 0.852–0.869.

Paced p99 ms: median 8.945, range 8.399–9.164.

Paced bound ms: median 1.752, range 1.691–1.781.

Paced runs with p99 within the bound of the same run: 0 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.124 | 0.190 | 1.380 | +7.019 |
| 2 | 0.125 | 0.173 | 1.346 | +7.818 |
| 3 | 0.128 | 0.177 | 1.353 | +7.464 |
| 4 | 0.129 | 0.177 | 1.354 | +7.649 |
| 5 | 0.133 | 0.185 | 1.370 | +7.575 |

Paced runs with p99 within the bound from their own group flushes: 0 of 5.
