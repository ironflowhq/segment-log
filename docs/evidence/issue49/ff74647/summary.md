# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 61565.301 | 15.684 | 22.870 | 26.907 | 3.952 | 6.840 | 14.680 | 711.224 | 296064032 |
| 2 | 62047.646 | 15.495 | 22.587 | 25.878 | 3.913 | 6.709 | 14.418 | 708.442 | 296064096 |
| 3 | 60641.106 | 16.059 | 22.845 | 25.353 | 3.855 | 6.823 | 14.646 | 706.831 | 296064096 |
| 4 | 60789.251 | 15.742 | 22.598 | 25.702 | 3.889 | 6.736 | 14.472 | 711.192 | 296064096 |
| 5 | 61127.460 | 15.749 | 22.243 | 25.731 | 3.919 | 6.744 | 14.488 | 710.521 | 296064096 |

Throughput records/s: median 61127.460, range 60641.106–62047.646.

Append p50 ms: median 15.742, range 15.495–16.059.

Append p99 ms: median 22.598, range 22.243–22.870.

Append p999 ms: median 25.731, range 25.353–26.907.

Device p50 ms: median 3.913, range 3.855–3.952.

Device p99 ms: median 6.744, range 6.709–6.840.

Reopen ms: median 710.521, range 706.831–711.224.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 16.164 | 16.243 | 1999 | 3693 |
| 2 | 16.036 | 16.117 | 2001 | 3693 |
| 3 | 16.409 | 16.490 | 2001 | 3713 |
| 4 | 16.387 | 16.450 | 2001 | 3674 |
| 5 | 16.289 | 16.359 | 2001 | 3735 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49942.614 | 8.138 | 8.005 | 14.751 | 18.519 | 21.420 | 6.747 | 14.494 | +0.257 | 3937 | 3193 |
| 2 | 49942.535 | 7.898 | 7.783 | 14.206 | 17.441 | 20.847 | 6.721 | 14.443 | -0.237 | 4045 | 3169 |
| 3 | 49947.917 | 7.930 | 7.756 | 14.443 | 18.186 | 24.585 | 6.793 | 14.585 | -0.142 | 4057 | 3176 |
| 4 | 49948.545 | 8.377 | 8.178 | 15.171 | 19.197 | 23.151 | 6.719 | 14.437 | +0.734 | 3859 | 3186 |
| 5 | 49932.233 | 7.496 | 7.381 | 13.990 | 17.798 | 20.416 | 6.750 | 14.501 | -0.511 | 4248 | 3154 |

Paced p50 ms: median 7.783, range 7.381–8.178.

Paced p99 ms: median 14.443, range 13.990–15.171.

Paced bound ms: median 14.494, range 14.437–14.585.

Paced runs with p99 within the bound of the same run: 3 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 3.115 | 6.027 | 13.055 | +1.696 |
| 2 | 3.092 | 5.998 | 12.995 | +1.210 |
| 3 | 3.099 | 5.967 | 12.934 | +1.509 |
| 4 | 3.109 | 6.049 | 13.098 | +2.074 |
| 5 | 3.074 | 5.978 | 12.957 | +1.034 |

Paced runs with p99 within the bound from their own group flushes: 0 of 5.
