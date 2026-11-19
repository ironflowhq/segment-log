# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 309810.905 | 3.053 | 6.702 | 18.444 | 0.369 | 0.471 | 1.943 | 43.372 | 312642208 |
| 2 | 313378.135 | 3.014 | 6.645 | 18.528 | 0.372 | 0.459 | 1.917 | 43.549 | 312656144 |
| 3 | 310942.421 | 3.055 | 6.698 | 17.988 | 0.349 | 0.430 | 1.861 | 44.313 | 312641904 |
| 4 | 312693.614 | 3.033 | 6.852 | 17.153 | 0.359 | 0.426 | 1.853 | 43.423 | 312645432 |
| 5 | 312231.734 | 3.031 | 6.614 | 17.610 | 0.373 | 0.463 | 1.926 | 43.329 | 312648200 |

Throughput records/s: median 312231.734, range 309810.905–313378.135.

Append p50 ms: median 3.033, range 3.014–3.055.

Append p99 ms: median 6.698, range 6.614–6.852.

Append p999 ms: median 17.988, range 17.153–18.528.

Device p50 ms: median 0.369, range 0.349–0.373.

Device p99 ms: median 0.459, range 0.426–0.471.

Reopen ms: median 43.423, range 43.329–44.313.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 3.199 | 3.228 | 1238 | 413 |
| 2 | 3.161 | 3.191 | 1235 | 395 |
| 3 | 3.189 | 3.216 | 1305 | 385 |
| 4 | 3.167 | 3.198 | 1236 | 395 |
| 5 | 3.168 | 3.203 | 1238 | 407 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49977.122 | 0.925 | 0.841 | 1.353 | 22.863 | 28.687 | 0.446 | 1.891 | -0.538 | 19406 | 174 |
| 2 | 49974.375 | 0.932 | 0.848 | 1.362 | 23.066 | 28.043 | 0.506 | 2.013 | -0.651 | 19406 | 183 |
| 3 | 49978.253 | 0.926 | 0.838 | 1.351 | 23.509 | 28.581 | 0.431 | 1.861 | -0.510 | 19400 | 174 |
| 4 | 49980.817 | 0.934 | 0.847 | 1.368 | 23.262 | 28.921 | 0.451 | 1.903 | -0.535 | 19403 | 181 |
| 5 | 49979.067 | 0.936 | 0.850 | 1.370 | 23.307 | 29.698 | 0.457 | 1.913 | -0.544 | 19406 | 183 |

Paced p50 ms: median 0.847, range 0.838–0.850.

Paced p99 ms: median 1.362, range 1.351–1.370.

Paced bound ms: median 1.903, range 1.861–2.013.

Paced runs with p99 within the bound of the same run: 5 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.172 | 0.223 | 1.446 | -0.093 |
| 2 | 0.179 | 0.238 | 1.475 | -0.113 |
| 3 | 0.172 | 0.225 | 1.451 | -0.100 |
| 4 | 0.176 | 0.236 | 1.471 | -0.103 |
| 5 | 0.177 | 0.236 | 1.472 | -0.102 |

Paced runs with p99 within the bound from their own group flushes: 5 of 5.
