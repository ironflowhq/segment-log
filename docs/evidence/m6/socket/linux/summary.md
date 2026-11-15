# M6 socket diagnostic

## closed (5 pairs; median, min to max)

| Metric | In process | Unix socket | Socket change |
| --- | --- | --- | --- |
| records/s | 326,915 (310,572 to 329,891) | 73,636 (66,843 to 76,303) | -77.5% |
| p50 ms | 2.732 (2.694 to 2.859) | 12.581 (12.387 to 12.706) | +360.6% |
| p99 ms | 4.628 (4.372 to 5.326) | 30.471 (25.045 to 72.795) | +558.5% |
| p999 ms | 73.551 (69.497 to 80.631) | 116.690 (90.958 to 297.091) | +58.7% |

## paced (5 pairs; median, min to max)

| Metric | In process | Unix socket | Socket change |
| --- | --- | --- | --- |
| records/s | 49,970 (49,844 to 49,974) | 49,873 (49,820 to 49,898) | -0.2% |
| p50 ms | 1.005 (0.999 to 1.040) | 1.437 (1.426 to 1.504) | +43.0% |
| p99 ms | 30.106 (28.491 to 41.638) | 64.620 (60.947 to 71.627) | +114.6% |
| p999 ms | 67.951 (65.362 to 68.896) | 87.540 (85.226 to 96.473) | +28.8% |

