# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 430991.910 | 2.280 | 3.425 | 4.506 | 0.309 | 0.603 | 2.205 | 475.149 | 296042176 |
| 2 | 410979.326 | 2.404 | 3.548 | 4.967 | 0.298 | 0.358 | 1.716 | 472.727 | 296045216 |
| 3 | 424813.280 | 2.329 | 3.346 | 4.042 | 0.310 | 0.367 | 1.733 | 485.868 | 296045088 |
| 4 | 427384.003 | 2.287 | 3.549 | 4.903 | 0.308 | 0.474 | 1.948 | 491.419 | 296045312 |
| 5 | 392852.906 | 2.301 | 3.386 | 80.296 | 0.301 | 0.457 | 1.915 | 469.743 | 296045184 |

Throughput records/s: median 424813.280, range 392852.906–430991.910.

Append p50 ms: median 2.301, range 2.280–2.404.

Append p99 ms: median 3.425, range 3.346–3.549.

Append p999 ms: median 4.903, range 4.042–80.296.

Device p50 ms: median 0.308, range 0.298–0.310.

Device p99 ms: median 0.457, range 0.358–0.603.

Reopen ms: median 475.149, range 469.743–491.419.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 2.302 | 2.320 | 1316 | 415 |
| 2 | 2.412 | 2.433 | 1411 | 488 |
| 3 | 2.335 | 2.354 | 1407 | 462 |
| 4 | 2.315 | 2.340 | 1414 | 421 |
| 5 | 2.528 | 2.545 | 1410 | 561 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49994.056 | 3.219 | 1.112 | 46.079 | 85.042 | 98.478 | 0.497 | 1.994 | +44.084 | 16866 | 433 |
| 2 | 49995.185 | 1.046 | 1.046 | 1.669 | 2.143 | 8.087 | 0.344 | 1.688 | -0.018 | 19004 | 343 |
| 3 | 49993.133 | 1.066 | 1.005 | 1.797 | 27.467 | 45.046 | 0.497 | 1.994 | -0.198 | 18945 | 288 |
| 4 | 49987.724 | 1.034 | 1.035 | 1.642 | 2.260 | 5.276 | 0.382 | 1.763 | -0.122 | 19010 | 326 |
| 5 | 49995.844 | 0.962 | 0.966 | 1.520 | 1.695 | 4.359 | 0.389 | 1.779 | -0.258 | 19017 | 263 |

Paced p50 ms: median 1.035, range 0.966–1.112.

Paced p99 ms: median 1.669, range 1.520–46.079.

Paced bound ms: median 1.779, range 1.688–1.994.

Paced runs with p99 within the bound of the same run: 4 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.277 | 4.189 | 9.377 | +36.701 |
| 2 | 0.285 | 0.526 | 2.053 | -0.384 |
| 3 | 0.267 | 0.617 | 2.234 | -0.438 |
| 4 | 0.275 | 0.482 | 1.965 | -0.323 |
| 5 | 0.256 | 0.430 | 1.859 | -0.339 |

Paced runs with p99 within the bound from their own group flushes: 4 of 5.
