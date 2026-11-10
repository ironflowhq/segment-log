# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 198197.669 | 4.912 | 7.943 | 12.559 | 0.458 | 0.811 | 2.623 | 886.982 | 296064576 |
| 2 | 206657.494 | 4.647 | 8.102 | 16.874 | 0.312 | 1.913 | 4.826 | 859.137 | 296064352 |
| 3 | 196580.939 | 4.906 | 8.552 | 18.145 | 0.298 | 0.952 | 2.905 | 859.788 | 296064640 |
| 4 | 202869.883 | 4.805 | 7.617 | 11.678 | 0.380 | 1.580 | 4.161 | 868.401 | 296064416 |
| 5 | 196037.964 | 4.920 | 8.861 | 17.355 | 0.298 | 0.714 | 2.428 | 847.582 | 296064576 |

Throughput records/s: median 198197.669, range 196037.964–206657.494.

Append p50 ms: median 4.906, range 4.647–4.920.

Append p99 ms: median 8.102, range 7.617–8.861.

Append p999 ms: median 16.874, range 11.678–18.145.

Device p50 ms: median 0.312, range 0.298–0.458.

Device p99 ms: median 0.952, range 0.714–1.913.

Reopen ms: median 859.788, range 847.582–886.982.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 5.003 | 5.045 | 2016 | 510 |
| 2 | 4.794 | 4.839 | 2009 | 467 |
| 3 | 5.041 | 5.087 | 2018 | 519 |
| 4 | 4.884 | 4.929 | 2011 | 509 |
| 5 | 5.058 | 5.101 | 2016 | 554 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49920.313 | 1.237 | 1.192 | 2.758 | 13.456 | 21.371 | 0.732 | 2.465 | +0.293 | 18761 | 346 |
| 2 | 49920.242 | 1.313 | 1.218 | 4.221 | 15.799 | 22.137 | 0.672 | 2.344 | +1.876 | 18664 | 380 |
| 3 | 49920.801 | 1.283 | 1.213 | 3.834 | 13.153 | 17.679 | 0.681 | 2.362 | +1.472 | 18653 | 388 |
| 4 | 49921.904 | 1.280 | 1.225 | 3.449 | 12.642 | 17.371 | 0.534 | 2.067 | +1.382 | 18697 | 394 |
| 5 | 49922.555 | 1.251 | 1.175 | 4.044 | 14.533 | 21.072 | 0.835 | 2.671 | +1.373 | 18641 | 345 |

Paced p50 ms: median 1.213, range 1.175–1.225.

Paced p99 ms: median 3.834, range 2.758–4.221.

Paced bound ms: median 2.362, range 2.067–2.671.

Paced runs with p99 within the bound of the same run: 0 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 0.328 | 0.662 | 2.324 | +0.434 |
| 2 | 0.356 | 0.758 | 2.516 | +1.704 |
| 3 | 0.361 | 0.732 | 2.465 | +1.369 |
| 4 | 0.370 | 0.734 | 2.467 | +0.982 |
| 5 | 0.320 | 0.910 | 2.820 | +1.224 |

Paced runs with p99 within the bound from their own group flushes: 0 of 5.
