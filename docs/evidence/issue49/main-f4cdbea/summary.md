# Linux default baseline

| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 63198.952 | 15.512 | 24.172 | 126.077 | 3.935 | 6.825 | 14.649 | 72.520 | 312651056 |
| 2 | 62203.469 | 15.594 | 25.677 | 130.587 | 3.915 | 6.841 | 14.682 | 72.849 | 312653040 |
| 3 | 60543.663 | 15.778 | 24.537 | 113.202 | 3.931 | 6.808 | 14.616 | 69.891 | 312654560 |
| 4 | 64516.290 | 15.391 | 24.029 | 119.497 | 3.942 | 6.785 | 14.570 | 72.815 | 312649472 |
| 5 | 60248.805 | 15.966 | 23.229 | 138.315 | 3.914 | 6.745 | 14.489 | 68.280 | 312654320 |

Throughput records/s: median 62203.469, range 60248.805–64516.290.

Append p50 ms: median 15.594, range 15.391–15.966.

Append p99 ms: median 24.172, range 23.229–25.677.

Append p999 ms: median 126.077, range 113.202–138.315.

Device p50 ms: median 3.931, range 3.914–3.942.

Device p99 ms: median 6.808, range 6.745–6.841.

Reopen ms: median 72.520, range 68.280–72.849.

## Closed-loop latency beside callers divided by throughput

| Run | Mean ms | Callers / throughput ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- |
| 1 | 15.795 | 15.823 | 1998 | 3593 |
| 2 | 16.028 | 16.076 | 1998 | 3606 |
| 3 | 16.493 | 16.517 | 1999 | 3588 |
| 4 | 15.468 | 15.500 | 1999 | 3573 |
| 5 | 16.571 | 16.598 | 1998 | 3630 |

# Linux paced runs at 50,000 records/s

| Run | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Device p99 ms | Bound ms | p99 minus bound ms | Groups | Mean group flush µs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 49954.705 | 11.867 | 8.180 | 87.830 | 134.318 | 149.421 | 6.823 | 14.646 | +73.184 | 3774 | 3195 |
| 2 | 49950.106 | 9.647 | 7.730 | 76.068 | 108.874 | 125.317 | 6.786 | 14.572 | +61.495 | 4014 | 3156 |
| 3 | 49941.404 | 11.489 | 8.728 | 83.775 | 114.131 | 128.940 | 6.825 | 14.650 | +69.125 | 3582 | 3191 |
| 4 | 49940.337 | 10.909 | 8.283 | 83.127 | 113.837 | 130.391 | 6.834 | 14.667 | +68.459 | 3754 | 3189 |
| 5 | 49954.900 | 11.531 | 8.418 | 82.870 | 123.604 | 140.312 | 6.805 | 14.610 | +68.259 | 3693 | 3187 |

Paced p50 ms: median 8.283, range 7.730–8.728.

Paced p99 ms: median 83.127, range 76.068–87.830.

Paced bound ms: median 14.646, range 14.572–14.667.

Paced runs with p99 within the bound of the same run: 0 of 5.

## Paced p99 beside the bound from the run's own group flushes

| Run | Group flush p50 ms | Group flush p99 ms | Bound from group flushes ms | p99 minus that bound ms |
| --- | --- | --- | --- | --- |
| 1 | 3.098 | 6.059 | 13.117 | +74.713 |
| 2 | 3.074 | 5.961 | 12.922 | +63.146 |
| 3 | 3.106 | 6.028 | 13.057 | +70.718 |
| 4 | 3.101 | 6.031 | 13.063 | +70.064 |
| 5 | 3.098 | 6.031 | 13.063 | +69.807 |

Paced runs with p99 within the bound from their own group flushes: 0 of 5.
